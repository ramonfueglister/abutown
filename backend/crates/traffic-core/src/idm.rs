//! Intelligent Driver Model (Treiber, Hennecke & Helbing 2000) — the
//! car-following acceleration law used by every vehicle each tick.
//!
//! Given the follower's speed `v`, the approach rate `dv = v − v_leader`
//! (positive when closing on the leader), and the bumper-to-bumper `gap` to
//! the leader, [`idm_accel`] returns the longitudinal acceleration (m/s²).
//! Leaderless vehicles (free road) pass `gap = f32::INFINITY`, which drops the
//! interaction term and leaves pure free-road acceleration toward `v0`.

/// Number of vehicle classes carried end-to-end (trips.bin byte 13 → kernel →
/// wire → silhouette): 0 = passenger car, 1 = delivery van (Lieferwagen),
/// 2 = HGV/truck (LKW).
pub const N_CLASSES: usize = 3;
/// Passenger car (PW).
pub const CLASS_CAR: u8 = 0;
/// Light commercial vehicle / delivery van (Lieferwagen).
pub const CLASS_DELIVERY: u8 = 1;
/// Heavy goods vehicle (LKW).
pub const CLASS_TRUCK: u8 = 2;

/// Vehicle length (m) by class, used for bumper-to-bumper gaps: 4.5 m car,
/// 6.5 m delivery van (Sprinter-class), 12 m rigid HGV. Out-of-range classes
/// are a caller bug; [`crate::tick::Core::spawn`] rejects them before this is
/// consulted.
pub fn class_len_m(class: u8) -> f32 {
    match class {
        CLASS_TRUCK => 12.0,
        CLASS_DELIVERY => 6.5,
        _ => 4.5,
    }
}

/// IDM calibration parameters for one vehicle class.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IdmParams {
    /// Desired free-road speed `v0` (m/s).
    pub v0: f32,
    /// Safe time headway `T` (s).
    pub t_headway: f32,
    /// Maximum acceleration `a` (m/s²).
    pub a_max: f32,
    /// Comfortable deceleration `b` (m/s²).
    pub b_comf: f32,
    /// Minimum jam distance `s0` (m).
    pub s0: f32,
}

impl Default for IdmParams {
    /// Canonical passenger-car values from Treiber et al. (2000), Table 1
    /// (highway calibration), with a modest urban `v0`.
    fn default() -> Self {
        IdmParams {
            v0: 13.9, // ~50 km/h urban
            t_headway: 1.5,
            a_max: 1.4,
            b_comf: 2.0,
            s0: 2.0,
        }
    }
}

impl IdmParams {
    /// Passenger car: the canonical Treiber et al. (2000) defaults.
    pub fn car() -> Self {
        Self::default()
    }

    /// Delivery van (Lieferwagen): same urban desired speed as a car but a
    /// slightly longer headway, weaker acceleration and larger jam distance —
    /// the light-commercial column of the Treiber & Kesting (2013, ch. 11)
    /// heavy-vehicle calibration, interpolated toward the car values.
    pub fn delivery() -> Self {
        IdmParams {
            v0: 13.9,
            t_headway: 1.6,
            a_max: 1.1,
            b_comf: 2.0,
            s0: 2.5,
        }
    }

    /// HGV (LKW): truck calibration after Treiber & Kesting (2013, ch. 11) —
    /// markedly weaker acceleration, longer headway and jam distance, and a
    /// desired speed slightly below the urban limit.
    pub fn truck() -> Self {
        IdmParams {
            v0: 12.5,
            t_headway: 1.7,
            a_max: 0.7,
            b_comf: 2.0,
            s0: 3.0,
        }
    }

    /// The calibration for a wire vehicle class (see [`N_CLASSES`]).
    pub fn for_class(class: u8) -> Self {
        match class {
            CLASS_TRUCK => Self::truck(),
            CLASS_DELIVERY => Self::delivery(),
            _ => Self::car(),
        }
    }
}

/// Minimum gap (m) fed to the interaction term — clamps the `(s*/gap)²`
/// singularity as `gap → 0` so a coincident leader yields strong but finite
/// braking instead of `−∞`.
const MIN_GAP: f32 = 0.1;

/// IDM acceleration (m/s²) for a follower at speed `v`, approaching its leader
/// at rate `dv = v − v_leader`, separated by bumper-to-bumper `gap` (m).
///
/// ```text
/// s* = s0 + max(0, v·T + v·Δv / (2·√(a·b)))
/// acc = a · (1 − (v/v0)⁴ − (s*/gap)²)
/// ```
///
/// `gap` is clamped to `>= 0.1 m`. Pass `gap = f32::INFINITY` for a leaderless
/// vehicle; the interaction term then vanishes and only the free-road term
/// `a·(1 − (v/v0)⁴)` remains.
#[inline]
pub fn idm_accel(p: &IdmParams, v: f32, dv: f32, gap: f32) -> f32 {
    let free = 1.0 - (v / p.v0).powi(4);

    if gap.is_infinite() {
        return p.a_max * free;
    }

    let gap = gap.max(MIN_GAP);
    let s_star = p.s0 + (v * p.t_headway + (v * dv) / (2.0 * (p.a_max * p.b_comf).sqrt())).max(0.0);
    let interaction = (s_star / gap).powi(2);
    p.a_max * (free - interaction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equilibrium_gap_yields_zero_accel() {
        // Follower matched to leader speed at the IDM equilibrium gap
        // s* = s0 + v·T (with dv = 0) makes the interaction term exactly 1, so
        // acc = a·(1 − (v/v0)⁴ − 1) = −a·(v/v0)⁴. At a low speed relative to v0
        // the free-road term (v/v0)⁴ is negligible, so |acc| < 0.01. (This is
        // the brief's equilibrium check; it holds precisely in the low-speed
        // limit — at higher v the true zero-accel gap is slightly larger.)
        let p = IdmParams::default();
        let v = 2.0;
        let dv = 0.0;
        let gap = p.s0 + v * p.t_headway;
        let acc = idm_accel(&p, v, dv, gap);
        assert!(acc.abs() < 0.01, "expected |acc| < 0.01, got {acc}");
    }

    #[test]
    fn closing_fast_brakes_hard() {
        // Small gap and high closing speed -> strong braking.
        let p = IdmParams::default();
        let v = 12.0;
        let dv = 10.0; // leader nearly stopped ahead
        let gap = 5.0;
        let acc = idm_accel(&p, v, dv, gap);
        assert!(acc < -2.0, "expected acc < -2, got {acc}");
    }

    #[test]
    fn free_road_accelerates_toward_v0_not_past() {
        let p = IdmParams::default();
        // Well below v0 on a clear road -> positive acceleration.
        let acc_slow = idm_accel(&p, 0.0, 0.0, f32::INFINITY);
        assert!(acc_slow > 0.0, "expected accel from rest, got {acc_slow}");
        // Exactly at v0 -> no acceleration.
        let acc_at = idm_accel(&p, p.v0, 0.0, f32::INFINITY);
        assert!(acc_at.abs() < 1e-4, "expected ~0 at v0, got {acc_at}");
        // Above v0 -> decelerates (never overshoots v0 at steady state).
        let acc_over = idm_accel(&p, p.v0 + 2.0, 0.0, f32::INFINITY);
        assert!(acc_over < 0.0, "expected decel above v0, got {acc_over}");
    }
}
