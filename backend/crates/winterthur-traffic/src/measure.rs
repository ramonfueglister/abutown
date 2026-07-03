//! Per-edge travel-time measurement over rolling 5-sim-minute windows.
//!
//! Each tick, every alive vehicle contributes the reciprocal of its speed to
//! the edge it is on. At the end of a [`WINDOW_TICKS`] window the per-edge
//! **harmonic-mean speed** is `count / Σ(1/v)` — the space-mean speed estimator
//! appropriate for travel time (Wardrop 1952) — and the edge travel time is
//! `length / harmonic_speed`. Those times are handed to
//! [`Router::update_weights`] (MSA-smoothed) and the CH is rebuilt, so routing
//! reacts to congestion on the same 5-minute cadence (plan §7).
//!
//! Edges with no samples in a window keep their previous weight (a `NaN` in the
//! times vector, which `update_weights` skips) rather than snapping back to
//! free-flow — a quiet edge is not necessarily a fast one, and this avoids
//! oscillation.

use crate::Router;
use traffic_core::Core;
use traffic_net::TrafficNet;

/// Simulation timestep (s), echoed from the kernel.
const DT: f32 = traffic_core::DT;

/// Window length: 5 sim-minutes = 300 s / 0.1 s = 3000 ticks.
pub const WINDOW_TICKS: u64 = (300.0 / DT) as u64;

/// Speeds below this (m/s) are floored before reciprocal so a stopped vehicle
/// doesn't blow the harmonic mean to infinity (it still pulls the mean speed
/// far down, reflecting the congestion).
const MIN_SPEED_MS: f32 = 0.5;

/// Rolling per-edge harmonic-mean-speed accumulator.
pub struct EdgeMeasure {
    /// Σ(1/v) per edge over the current window.
    sum_inv_v: Vec<f64>,
    /// Sample count per edge over the current window.
    count: Vec<u32>,
    /// Representative length (m) per edge, precomputed from its first lane.
    edge_len_m: Vec<f32>,
    /// Free-flow travel time (s) per edge, used as the harmonic-speed fallback
    /// is not applied (we skip un-sampled edges instead); kept for reference /
    /// tests.
    free_flow_s: Vec<f32>,
}

impl EdgeMeasure {
    /// Build accumulators sized to the net's edge count.
    pub fn new(net: &TrafficNet) -> Self {
        let n = net.edges.len();
        let mut edge_len_m = vec![0.0f32; n];
        let mut free_flow_s = vec![0.0f32; n];
        for e in &net.edges {
            let len = net.lane_len(e.lanes[0]);
            edge_len_m[e.id as usize] = len;
            free_flow_s[e.id as usize] = len / e.speed_ms;
        }
        EdgeMeasure {
            sum_inv_v: vec![0.0; n],
            count: vec![0; n],
            edge_len_m,
            free_flow_s,
        }
    }

    /// Free-flow travel time (s) for `edge`. Exposed for tests.
    pub fn free_flow_s(&self, edge: u32) -> f32 {
        self.free_flow_s[edge as usize]
    }

    /// Accumulate this tick's per-vehicle speed samples into their edge bins.
    pub fn sample(&mut self, core: &Core, net: &TrafficNet) {
        let fleet = &core.fleet;
        for i in 0..fleet.slots() {
            if !fleet.alive[i] {
                continue;
            }
            let lane = fleet.lane[i];
            let edge = net.lanes[lane as usize].edge as usize;
            let v = fleet.v[i].max(MIN_SPEED_MS);
            self.sum_inv_v[edge] += 1.0 / v as f64;
            self.count[edge] += 1;
        }
    }

    /// Whether tick `t` closes a measurement window (and a rebuild is due).
    pub fn window_closes(t: u64) -> bool {
        t > 0 && t.is_multiple_of(WINDOW_TICKS)
    }

    /// Close the current window: derive per-edge travel times, push them into
    /// `router` (MSA-smoothed) and rebuild the CH, then reset the accumulators.
    /// Un-sampled edges emit `NaN` (skipped by `update_weights`).
    pub fn flush(&mut self, router: &mut Router) {
        let mut times_s = vec![f32::NAN; self.edge_len_m.len()];
        for (t, (&count, (&sum_inv, &len))) in times_s.iter_mut().zip(
            self.count
                .iter()
                .zip(self.sum_inv_v.iter().zip(self.edge_len_m.iter())),
        ) {
            if count > 0 {
                let harmonic_speed = count as f64 / sum_inv;
                *t = (len as f64 / harmonic_speed) as f32;
            }
        }
        router.update_weights(&times_s);
        router.rebuild();

        for v in self.sum_inv_v.iter_mut() {
            *v = 0.0;
        }
        for c in self.count.iter_mut() {
            *c = 0;
        }
    }
}
