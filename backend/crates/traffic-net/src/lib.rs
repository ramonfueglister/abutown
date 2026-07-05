//! `traffic-net`: loads + validates the baked lane-level Winterthur traffic
//! network (`data/winterthur/trafficnet.json`, produced by
//! `scripts/geo/lib/trafficnet.mjs`) and exposes a query-ready in-memory
//! representation for downstream crates (SoA sim kernel, server).
//!
//! Design: `load` deserializes with serde (field names must match the JSON
//! exactly — see [`types::TrafficNetDoc`]), then runs [`validate::validate`]
//! to fail fast on any structural inconsistency (dangling ids, lane length
//! drift, uncovered signal turns). No healing / defaulting on load, per
//! project convention — a corrupt bake is a hard error, not silently patched.

pub mod types;
pub mod validate;

pub use types::{
    Anchor, Edge, Lane, Meta, Node, NodeKind, Signal, SignalPhase, TrafficNetDoc, Turn,
};
pub use validate::NetError;

/// The lane-level traffic network, ready for query: raw parsed data plus
/// precomputed indices (turns-from-lane CSR, per-lane arc-length LUT).
#[derive(Debug, Clone)]
pub struct TrafficNet {
    pub meta: Meta,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub lanes: Vec<Lane>,
    pub turns: Vec<Turn>,

    /// CSR: `turns_from_offsets[lane]..turns_from_offsets[lane+1]` indexes
    /// into `turns_from_ids`, the turn ids departing that lane (as `fromLane`).
    turns_from_offsets: Vec<u32>,
    turns_from_ids: Vec<u32>,

    /// Per-lane cumulative arc length at each polyline vertex (same length as
    /// `lanes[i].pts`), used by `pos_at` for O(log n) interpolation.
    arc_lut: Vec<Vec<f32>>,

    /// Gateway node ids (kind `gateway`), sorted ascending.
    gateways: Vec<u32>,
    /// Lanes whose edge ends at (`to`) a gateway node, sorted by lane id.
    gateway_lanes_in: Vec<u32>,
    /// Lanes whose edge starts at (`from`) a gateway node, sorted by lane id.
    gateway_lanes_out: Vec<u32>,
}

/// Parse + validate a baked `trafficnet.json` document, returning a
/// query-ready [`TrafficNet`]. Fails fast on malformed JSON or any structural
/// invariant violation (see [`validate::validate`]) — never heals partial data.
pub fn load(json: &str) -> Result<TrafficNet, NetError> {
    let doc: TrafficNetDoc =
        serde_json::from_str(json).map_err(|e| NetError::Parse(e.to_string()))?;
    validate::validate(&doc)?;
    Ok(TrafficNet::from_doc(doc))
}

impl TrafficNet {
    fn from_doc(doc: TrafficNetDoc) -> Self {
        let lane_count = doc
            .lanes
            .iter()
            .map(|l| l.id)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0) as usize;

        // Group turns by fromLane, preserving turn-id ascending order within
        // each lane (input turns are already id-ordered in the baked JSON;
        // we don't rely on that, we bucket then sort for determinism).
        let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); lane_count];
        for t in &doc.turns {
            buckets[t.from_lane as usize].push(t.id);
        }
        for b in &mut buckets {
            b.sort_unstable();
        }

        let mut turns_from_offsets = Vec::with_capacity(lane_count + 1);
        let mut turns_from_ids = Vec::new();
        turns_from_offsets.push(0u32);
        for b in &buckets {
            turns_from_ids.extend_from_slice(b);
            turns_from_offsets.push(turns_from_ids.len() as u32);
        }

        // Per-lane arc-length LUT, indexed by lane array position (lanes are
        // looked up by id -> position via `lane_index_of_id`, so this stays
        // robust to any future gaps/reordering in `doc.lanes`).
        let arc_lut: Vec<Vec<f32>> = doc
            .lanes
            .iter()
            .map(|l| {
                let mut acc = Vec::with_capacity(l.pts.len());
                let mut running = 0.0f32;
                acc.push(0.0);
                for w in l.pts.windows(2) {
                    let dx = w[1][0] - w[0][0];
                    let dy = w[1][1] - w[0][1];
                    running += (dx * dx + dy * dy).sqrt();
                    acc.push(running);
                }
                acc
            })
            .collect();

        // Gateway index: node ids plus the lanes feeding into / out of them
        // (demand sources/sinks at the Gemeinde boundary), all id-sorted.
        let mut gateways: Vec<u32> = doc
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Gateway)
            .map(|n| n.id)
            .collect();
        gateways.sort_unstable();
        let mut gateway_lanes_in: Vec<u32> = Vec::new();
        let mut gateway_lanes_out: Vec<u32> = Vec::new();
        for e in &doc.edges {
            if gateways.binary_search(&e.to).is_ok() {
                gateway_lanes_in.extend_from_slice(&e.lanes);
            }
            if gateways.binary_search(&e.from).is_ok() {
                gateway_lanes_out.extend_from_slice(&e.lanes);
            }
        }
        gateway_lanes_in.sort_unstable();
        gateway_lanes_out.sort_unstable();

        TrafficNet {
            meta: doc.meta,
            nodes: doc.nodes,
            edges: doc.edges,
            lanes: doc.lanes,
            turns: doc.turns,
            turns_from_offsets,
            turns_from_ids,
            arc_lut,
            gateways,
            gateway_lanes_in,
            gateway_lanes_out,
        }
    }

    /// Gateway node ids (Gemeinde-boundary stubs, kind `gateway`), sorted
    /// ascending. Empty for nets baked without a boundary (test nets).
    pub fn gateways(&self) -> &[u32] {
        &self.gateways
    }

    /// Lanes whose edge ends at (`to`) a gateway node — where outbound demand
    /// leaves the network. Sorted by lane id.
    pub fn gateway_lanes_in(&self) -> &[u32] {
        &self.gateway_lanes_in
    }

    /// Lanes whose edge starts at (`from`) a gateway node — where inbound
    /// demand enters the network. Sorted by lane id.
    pub fn gateway_lanes_out(&self) -> &[u32] {
        &self.gateway_lanes_out
    }

    fn lane_index_of_id(&self, lane: u32) -> Option<usize> {
        self.lanes.iter().position(|l| l.id == lane)
    }

    /// The declared (baked) length in metres of `lane`.
    ///
    /// # Panics
    /// Panics if `lane` is not a valid lane id — callers are expected to only
    /// pass ids obtained from this same [`TrafficNet`] (edges/turns), and a
    /// dangling id here would already have been rejected by `validate` at
    /// load time.
    pub fn lane_len(&self, lane: u32) -> f32 {
        let idx = self
            .lane_index_of_id(lane)
            .unwrap_or_else(|| panic!("unknown lane id {lane}"));
        self.lanes[idx].length_m
    }

    /// Turn ids departing `lane` (as `fromLane`), in ascending turn-id order.
    /// Backed by a precomputed CSR index — O(1) plus the slice length.
    ///
    /// # Panics
    /// Panics if `lane` is not a valid lane id.
    pub fn turns_from(&self, lane: u32) -> &[u32] {
        let lane = lane as usize;
        assert!(
            lane + 1 < self.turns_from_offsets.len(),
            "unknown lane id {lane}"
        );
        let start = self.turns_from_offsets[lane] as usize;
        let end = self.turns_from_offsets[lane + 1] as usize;
        &self.turns_from_ids[start..end]
    }

    /// The world-space position and unit tangent (direction of travel) at arc
    /// length `s` metres along `lane`. `s` is clamped to `[0, lane_len]`.
    ///
    /// # Panics
    /// Panics if `lane` is not a valid lane id, or if the lane's polyline has
    /// fewer than 2 points (already rejected by `validate` in practice, since
    /// a single-point polyline has zero length and would fail the length
    /// check against any non-zero declared `lengthM`).
    pub fn pos_at(&self, lane: u32, s: f32) -> ([f32; 2], [f32; 2]) {
        let idx = self
            .lane_index_of_id(lane)
            .unwrap_or_else(|| panic!("unknown lane id {lane}"));
        let pts = &self.lanes[idx].pts;
        assert!(
            pts.len() >= 2,
            "lane {lane} polyline has fewer than 2 points"
        );
        let lut = &self.arc_lut[idx];
        let total = *lut.last().unwrap();
        let s = s.clamp(0.0, total);

        // find segment i such that lut[i] <= s <= lut[i+1]
        let seg = match lut.binary_search_by(|probe| probe.partial_cmp(&s).unwrap()) {
            Ok(i) => i.min(pts.len() - 2),
            Err(i) => (i.saturating_sub(1)).min(pts.len() - 2),
        };

        let a = pts[seg];
        let b = pts[seg + 1];
        let seg_len = lut[seg + 1] - lut[seg];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let tangent_len = (dx * dx + dy * dy).sqrt();
        let tangent = if tangent_len > 1e-9 {
            [dx / tangent_len, dy / tangent_len]
        } else {
            [0.0, 0.0]
        };

        let pos = if seg_len > 1e-9 {
            let t = (s - lut[seg]) / seg_len;
            [a[0] + dx * t, a[1] + dy * t]
        } else {
            a
        };

        (pos, tangent)
    }
}
