//! MOBIL lane-change decision model (Kesting, Treiber & Helbing 2007).
//!
//! MOBIL ("Minimizing Overall Braking Induced by Lane changes") decides whether
//! a vehicle should change to an adjacent lane by weighing the acceleration it
//! would *gain* against the acceleration its would-be new follower (and its
//! current follower) would *lose*, discounted by a politeness factor `p`. A
//! change is taken only if the net incentive clears a switching threshold and a
//! hard safety criterion on the new follower's deceleration is met.
//!
//! This module is pure: it consumes precomputed IDM accelerations (or the
//! ingredients to compute them) and returns a boolean decision plus the safety
//! deceleration. The kernel ([`crate::tick`]) is responsible for gathering the
//! neighbour state on the current and target lanes and for actually performing
//! the switch in phase 2.
//!
//! ## Lane-index / direction convention
//!
//! Lanes of one edge are indexed by [`traffic_net::Lane::index`], ascending.
//! We treat a **lower** index as the *right* lane and a **higher** index as the
//! *left* lane (European driving: overtake on the left, keep right otherwise).
//! The `bias_right` incentive is added when the candidate target lane has a
//! lower index than the current lane (a move to the right), encouraging
//! vehicles to return right after overtaking.

use crate::idm::{IdmParams, idm_accel};

/// MOBIL calibration parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobilParams {
    /// Politeness `p` in `[0, 1]`: how strongly the decider weighs the
    /// acceleration change it inflicts on its old and new followers. `0` is
    /// purely egoistic, `1` fully altruistic.
    pub politeness: f32,
    /// Switching threshold `a_thr` (m/s²): the minimum net incentive (advantage
    /// minus disadvantage) required before a change is worthwhile. Suppresses
    /// marginal flapping.
    pub a_thr: f32,
    /// Safety limit `b_safe` (m/s², positive): the change is vetoed unless the
    /// new follower's resulting acceleration stays above `-b_safe`, i.e. it is
    /// never forced to brake harder than `b_safe`.
    pub b_safe: f32,
    /// Keep-right bias `bias_right` (m/s²): incentive bonus applied to a target
    /// lane that lies to the right (lower index) of the current one, modelling
    /// the European keep-right rule.
    pub bias_right: f32,
}

impl Default for MobilParams {
    fn default() -> Self {
        MobilParams {
            politeness: 0.3,
            a_thr: 0.2,
            b_safe: 4.0,
            bias_right: 0.2,
        }
    }
}

/// The longitudinal neighbourhood a vehicle sees on one lane at its own arc
/// position `s`: the bumper-to-bumper gap and closing speed to the leader ahead,
/// and (if any) the follower behind it and that follower's own leader-gap.
///
/// All gaps are bumper-to-bumper (m); `dv = v_self − v_other` is positive when
/// closing. A missing neighbour is encoded with an infinite gap (free road).
#[derive(Debug, Clone, Copy)]
pub struct LaneNeighbourhood {
    /// Gap ahead to the leader on this lane (m). `INFINITY` if none.
    pub lead_gap: f32,
    /// Closing speed on the leader, `v_self − v_lead`.
    pub lead_dv: f32,
    /// The follower behind `s` on this lane, if any.
    pub follower: Option<Follower>,
}

/// The follower behind the deciding vehicle on a given lane.
#[derive(Debug, Clone, Copy)]
pub struct Follower {
    /// Follower speed (m/s).
    pub v: f32,
    /// Bumper-to-bumper gap from the follower to the deciding vehicle (m) — the
    /// gap the follower currently keeps to *its* leader, which is the decider on
    /// the target lane, or the decider's old leader on the current lane.
    pub gap_to_decider: f32,
    /// Bumper-to-bumper gap from the follower to *its* leader once the decider
    /// leaves this lane (current lane) or before it arrives (target lane) — i.e.
    /// the gap to the leader that is ahead of the decider. `INFINITY` if none.
    pub gap_without_decider: f32,
    /// Closing speed of the follower on the decider, `v_follower − v_self`.
    pub dv_to_decider: f32,
    /// Closing speed of the follower on the leader ahead of the decider.
    pub dv_without_decider: f32,
}

/// Outcome of a MOBIL evaluation for one candidate target lane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobilDecision {
    /// Whether the incentive + safety criteria are met for this target lane.
    pub change: bool,
    /// The candidate's net incentive (advantage − p·disadvantage + bias), for
    /// tie-breaking between two admissible target lanes (pick the larger).
    pub incentive: f32,
    /// The new follower's resulting acceleration on the target lane — must stay
    /// `> −b_safe`. Exposed so phase 2 can re-check safety against changes that
    /// were already applied this tick.
    pub new_follower_accel: f32,
}

/// Evaluate MOBIL for the deciding vehicle changing from its current lane to one
/// adjacent target lane.
///
/// * `p` / `idm`: IDM class of the vehicle and its neighbours (single class).
/// * `v_self`: decider speed (m/s).
/// * `cur`: neighbourhood on the **current** lane (its old leader + old
///   follower).
/// * `tgt`: neighbourhood on the **target** lane at the same `s` (the leader it
///   would gain + the new follower it would sit in front of).
/// * `to_right`: whether the target lane is to the decider's right (lower
///   index), which earns the keep-right `bias_right` bonus.
///
/// Returns whether the change passes the incentive threshold **and** the hard
/// safety criterion on the new follower's deceleration.
pub fn evaluate(
    m: &MobilParams,
    idm: &IdmParams,
    v_self: f32,
    cur: &LaneNeighbourhood,
    tgt: &LaneNeighbourhood,
    to_right: bool,
) -> MobilDecision {
    // Decider's own accel now (on the current lane) and after the move (on the
    // target lane, following the target lane's leader).
    let a_old_self = idm_accel(idm, v_self, cur.lead_dv, cur.lead_gap);
    let a_new_self = idm_accel(idm, v_self, tgt.lead_dv, tgt.lead_gap);

    // New follower (on the target lane): accel before the decider arrives
    // (following its current leader) vs after (following the decider).
    let (a_new_follower_before, a_new_follower_after) = match tgt.follower {
        Some(f) => {
            let before = idm_accel(idm, f.v, f.dv_without_decider, f.gap_without_decider);
            let after = idm_accel(idm, f.v, f.dv_to_decider, f.gap_to_decider);
            (before, after)
        }
        None => (0.0, 0.0),
    };

    // Old follower (on the current lane): accel while stuck behind the decider
    // vs after the decider leaves (it inherits the decider's old leader).
    let (a_old_follower_before, a_old_follower_after) = match cur.follower {
        Some(f) => {
            let before = idm_accel(idm, f.v, f.dv_to_decider, f.gap_to_decider);
            let after = idm_accel(idm, f.v, f.dv_without_decider, f.gap_without_decider);
            (before, after)
        }
        None => (0.0, 0.0),
    };

    // Safety: the new follower must not be forced to brake harder than b_safe.
    let new_follower_accel = a_new_follower_after;
    let safe = new_follower_accel > -m.b_safe;

    // MOBIL incentive (Kesting et al. 2007, eq. 2): advantage to self plus the
    // politeness-weighted sum of the accel changes it causes its two followers.
    let self_gain = a_new_self - a_old_self;
    let new_follower_delta = a_new_follower_after - a_new_follower_before; // <= 0 typically
    let old_follower_delta = a_old_follower_after - a_old_follower_before; // >= 0 typically
    let bias = if to_right { m.bias_right } else { 0.0 };
    let incentive = self_gain + m.politeness * (new_follower_delta + old_follower_delta) + bias;

    let change = safe && incentive > m.a_thr;

    MobilDecision {
        change,
        incentive,
        new_follower_accel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idm() -> IdmParams {
        IdmParams::default()
    }

    /// A clear target lane (no leader, no follower) and a jammed current lane
    /// should yield a strong positive incentive and pass.
    #[test]
    fn overtake_into_free_lane_accepts() {
        let m = MobilParams::default();
        let idm = idm();
        // Current lane: closing fast on a near-stopped leader 5 m ahead.
        let cur = LaneNeighbourhood {
            lead_gap: 5.0,
            lead_dv: 10.0,
            follower: None,
        };
        // Target lane (left): wide open.
        let tgt = LaneNeighbourhood {
            lead_gap: f32::INFINITY,
            lead_dv: 0.0,
            follower: None,
        };
        let d = evaluate(&m, &idm, 12.0, &cur, &tgt, false);
        assert!(
            d.change,
            "should overtake into a free lane, incentive {}",
            d.incentive
        );
    }

    /// Safety veto: even with a big incentive, a fast follower right behind on
    /// the target lane (tiny gap, high closing speed) must block the change.
    #[test]
    fn safety_veto_blocks_change_when_gap_too_small_behind() {
        let m = MobilParams::default();
        let idm = idm();
        let cur = LaneNeighbourhood {
            lead_gap: 5.0,
            lead_dv: 10.0,
            follower: None,
        };
        // Target lane: a fast follower 0.3 m behind, closing at 12 m/s -> would
        // have to brake catastrophically. new_follower_accel << -b_safe.
        let tgt = LaneNeighbourhood {
            lead_gap: f32::INFINITY,
            lead_dv: 0.0,
            follower: Some(Follower {
                v: 20.0,
                gap_to_decider: 0.3,
                gap_without_decider: f32::INFINITY,
                dv_to_decider: 12.0,
                dv_without_decider: 0.0,
            }),
        };
        let d = evaluate(&m, &idm, 8.0, &cur, &tgt, false);
        assert!(
            !d.change,
            "unsafe cut-in must be vetoed; new_follower_accel {}",
            d.new_follower_accel
        );
        assert!(d.new_follower_accel < -m.b_safe);
    }

    /// No incentive: both lanes are identically free -> self gain ~0, below
    /// threshold, so no gratuitous change.
    #[test]
    fn no_change_when_no_advantage() {
        let m = MobilParams::default();
        let idm = idm();
        let free = LaneNeighbourhood {
            lead_gap: f32::INFINITY,
            lead_dv: 0.0,
            follower: None,
        };
        let d = evaluate(&m, &idm, 10.0, &free, &free, false);
        assert!(
            !d.change,
            "no advantage, no change; incentive {}",
            d.incentive
        );
    }

    /// Keep-right bias: a marginal move that is below threshold to the left is
    /// pushed over threshold to the right by `bias_right`.
    #[test]
    fn keep_right_bias_tips_a_marginal_move() {
        let m = MobilParams {
            a_thr: 0.15,
            bias_right: 0.2,
            ..MobilParams::default()
        };
        let idm = idm();
        // Both lanes free at the same speed: self gain = 0. Only the bias can
        // tip it. Moving right (to_right = true) gets +0.2 > a_thr; moving left
        // gets +0 < a_thr.
        let free = LaneNeighbourhood {
            lead_gap: f32::INFINITY,
            lead_dv: 0.0,
            follower: None,
        };
        let right = evaluate(&m, &idm, 10.0, &free, &free, true);
        let left = evaluate(&m, &idm, 10.0, &free, &free, false);
        assert!(
            right.change,
            "keep-right should tip the move, incentive {}",
            right.incentive
        );
        assert!(
            !left.change,
            "no bias to the left, incentive {}",
            left.incentive
        );
    }
}
