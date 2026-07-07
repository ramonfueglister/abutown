//! Far-LOD aggregate flow sampler (Task 11): a per-edge vehicle count + mean
//! speed, sent every [`FLOW_EVERY_N_TICKS`] publish ticks (2 s at the 10 Hz sim
//! rate) as a single self-contained [`FlowFrame`] — no per-vehicle identity, no
//! deltas. Cheaper than a `CellFrame` per subscribed cell for a client zoomed
//! out past the point where individual vehicles are legible.
//!
//! # Read-only discipline
//!
//! [`sample_flow_frame`] takes `&Core` and iterates `core.vehicle_view` (the
//! same read-only seam the gateway's cell publisher and `measure.rs` use). It
//! never mutates sim state.
//!
//! # Determinism
//!
//! `FlowFrame.edges` is sorted by ascending edge id before encoding — the
//! sampler's internal accumulation is a dense per-edge array (not a
//! `HashMap`), so the emission order is already deterministic; the explicit
//! sort documents and enforces that invariant rather than relying on it being
//! an accident of iteration.

use abutown_protocol::traffic::{FlowFrame, FlowState};
use traffic_core::Core;
use traffic_net::TrafficNet;

/// Sample-and-publish cadence: every 20 sim ticks (2 s at `dt = 0.1 s`) — 10x
/// coarser than the per-cell publish cadence ([`crate::gateway::PUBLISH_EVERY_N_TICKS`]).
pub const FLOW_EVERY_N_TICKS: u64 = 20;

/// Quantise a mean speed (m/s) to the wire's 0.25 m/s units — identical
/// quantisation to `VehicleState.v_q` (`gateway::quantise`), so a client
/// shares one speed color-scale across both channels. Negative/NaN clamps to 0.
#[inline]
fn quantise_v(v: f32) -> u32 {
    (v * 4.0).round().max(0.0) as u32
}

/// Iterate every alive vehicle once, bucket by edge (via `vehicle_view`, which
/// already carries the lane's edge id — no separate lane->edge lookup needed),
/// and build a self-contained [`FlowFrame`] for `tick`. Edges with zero
/// vehicles are omitted; `count` saturates at 255; `v_q` is the mean speed
/// over the edge's vehicles, quantised identically to `VehicleState.v_q`
/// (see `gateway::quantise`).
pub fn sample_flow_frame(core: &Core, net: &TrafficNet, tick: u64) -> FlowFrame {
    let mut sum_v = vec![0.0f64; net.edges.len()];
    let mut count = vec![0u32; net.edges.len()];

    let slots = core.fleet.slots();
    for veh in 0..slots as u32 {
        let Some(view) = core.vehicle_view(veh) else {
            continue;
        };
        let edge = view.edge as usize;
        sum_v[edge] += view.v as f64;
        count[edge] += 1;
    }

    let mut edges: Vec<FlowState> = Vec::new();
    for (edge, &c) in count.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let mean_v = (sum_v[edge] / c as f64) as f32;
        edges.push(FlowState {
            edge: edge as u32,
            count: c.min(255),
            v_q: quantise_v(mean_v),
        });
    }
    // Deterministic order (see module docs): the accumulator is already dense
    // by edge id, but sort explicitly so this is an enforced invariant, not an
    // accident of the `Vec` build order above.
    edges.sort_unstable_by_key(|e| e.edge);

    FlowFrame { tick, edges }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    fn fixture_json() -> String {
        let p = format!(
            "{}/tests/fixtures/diamond-gateway.json",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
    }

    fn fixture_net() -> TrafficNet {
        traffic_net::load(&fixture_json()).expect("diamond-gateway fixture must validate")
    }

    /// A 3-vehicle toy core over the diamond-gateway fixture: two vehicles on
    /// edge 0 (lane 0) at different speeds, one on edge 1 (lane 1). Edge 2 has
    /// no vehicles and must be omitted from the frame.
    fn toy_core(net: &TrafficNet) -> Core {
        let mut core = Core::new(net, 16, 0);
        let v0 = core.spawn(0, 5.0, 0, &[0, 1]).expect("spawn v0 on lane 0");
        let v1 = core.spawn(0, 20.0, 0, &[0, 1]).expect("spawn v1 on lane 0");
        let v2 = core.spawn(1, 10.0, 0, &[1]).expect("spawn v2 on lane 1");
        // Directly set post-spawn speeds (spawn() always starts a vehicle at
        // v=0) so the sampler has non-trivial per-vehicle speeds to average.
        core.fleet.v[v0 as usize] = 8.0;
        core.fleet.v[v1 as usize] = 12.0;
        core.fleet.v[v2 as usize] = 20.0;
        core
    }

    /// Encode/decode round-trip: counts and v_q must survive the wire, and an
    /// edge with zero vehicles (edge 2) must be entirely absent from the frame.
    #[test]
    fn flow_frame_round_trips_counts_and_speed() {
        let net = fixture_net();
        let core = toy_core(&net);

        let frame = sample_flow_frame(&core, &net, 42);
        let bytes = frame.encode_to_vec();
        let decoded = FlowFrame::decode(bytes.as_slice()).expect("decode FlowFrame");

        assert_eq!(decoded.tick, 42);
        // Edge 0: 2 vehicles, mean v = (8.0 + 12.0) / 2 = 10.0 -> v_q = round(10*4) = 40.
        // Edge 1: 1 vehicle, v = 20.0 -> v_q = round(20*4) = 80.
        // Edge 2: no vehicles -> omitted.
        assert_eq!(
            decoded.edges,
            vec![
                FlowState {
                    edge: 0,
                    count: 2,
                    v_q: 40
                },
                FlowState {
                    edge: 1,
                    count: 1,
                    v_q: 80
                },
            ],
            "edge 2 (zero vehicles) must be omitted; edges sorted ascending"
        );
    }

    /// `count` saturates at 255 rather than wrapping or overflowing.
    #[test]
    fn flow_frame_count_saturates_at_255() {
        let net = fixture_net();
        let mut core = Core::new(&net, 300, 0);
        for i in 0..260 {
            let s = 1.0 + (i as f32) * 0.2;
            core.spawn(0, s, 0, &[0, 1]).expect("spawn within capacity");
        }
        let frame = sample_flow_frame(&core, &net, 0);
        let edge0 = frame
            .edges
            .iter()
            .find(|e| e.edge == 0)
            .expect("edge 0 must be present");
        assert_eq!(
            edge0.count, 255,
            "count must saturate, not overflow u8 wire repr"
        );
    }
}
