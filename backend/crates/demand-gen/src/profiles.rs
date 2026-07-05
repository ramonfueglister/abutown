//! Authored departure-time profiles: piecewise-linear daily PDFs
//! (hour → weight), sampled by inverse-CDF from a deterministic `u01` draw.
//! Workday carries the two commuter peaks (07–08 / 17–18); the morning and
//! evening variants are the two halves used for the mirrored in/out commuter
//! trips; weekend is a flat noon hump; through traffic is a 06–20 plateau.

/// A departure profile (piecewise-linear PDF over the 24 h day).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Full workday curve (both commuter peaks) — internal trips.
    Workday,
    /// Morning commuter peak only — the outbound-from-home leg of in/out.
    Morning,
    /// Evening commuter peak only — the mirrored return leg of in/out.
    Evening,
    /// Weekend: flat hump around noon.
    Weekend,
    /// Through traffic: plateau 06:00–20:00.
    Through,
}

/// Authored control points `(hour, weight)` of each piecewise-linear PDF.
/// Weights are relative (normalized by total area at sampling time).
fn breakpoints(profile: Profile) -> &'static [(f64, f64)] {
    match profile {
        Profile::Workday => &[
            (0.0, 0.15),
            (5.0, 0.15),
            (6.0, 0.6),
            (7.0, 1.6),
            (7.5, 2.0),
            (8.0, 1.6),
            (9.0, 0.8),
            (12.0, 0.9),
            (16.0, 1.2),
            (17.0, 1.8),
            (17.5, 2.0),
            (18.0, 1.5),
            (19.0, 0.8),
            (22.0, 0.35),
            (24.0, 0.15),
        ],
        Profile::Morning => &[
            (0.0, 0.0),
            (4.0, 0.0),
            (5.0, 0.2),
            (6.0, 0.8),
            (7.0, 1.8),
            (7.5, 2.0),
            (8.0, 1.6),
            (9.0, 0.7),
            (10.0, 0.3),
            (12.0, 0.05),
            (12.5, 0.0),
            (24.0, 0.0),
        ],
        Profile::Evening => &[
            (0.0, 0.0),
            (14.5, 0.0),
            (15.0, 0.3),
            (16.0, 0.9),
            (17.0, 1.8),
            (17.5, 2.0),
            (18.0, 1.5),
            (19.0, 0.7),
            (20.0, 0.35),
            (22.0, 0.1),
            (23.0, 0.0),
            (24.0, 0.0),
        ],
        Profile::Weekend => &[
            (0.0, 0.1),
            (6.0, 0.1),
            (9.0, 0.6),
            (12.0, 1.0),
            (15.0, 1.0),
            (18.0, 0.6),
            (22.0, 0.2),
            (24.0, 0.1),
        ],
        Profile::Through => &[
            (0.0, 0.2),
            (5.0, 0.3),
            (6.0, 1.0),
            (20.0, 1.0),
            (21.0, 0.3),
            (24.0, 0.2),
        ],
    }
}

/// Inverse-CDF sample of `profile` at quantile `u ∈ [0, 1)`, returning a
/// second-of-day in `[0, 86400)`. Monotone in `u`, deterministic.
pub fn sample_departure_s(profile: Profile, u: f32) -> u32 {
    let bp = breakpoints(profile);
    // total area under the piecewise-linear density (trapezoids, in
    // hour-weight units)
    let total: f64 = bp
        .windows(2)
        .map(|w| (w[0].1 + w[1].1) * 0.5 * (w[1].0 - w[0].0))
        .sum();
    debug_assert!(total > 0.0, "profile has zero mass");
    let mut target = (u as f64).clamp(0.0, 1.0) * total;

    for w in bp.windows(2) {
        let ((x0, w0), (x1, w1)) = (w[0], w[1]);
        let dx = x1 - x0;
        let area = (w0 + w1) * 0.5 * dx;
        if target > area {
            target -= area;
            continue;
        }
        // solve w0*t + k*t^2/2 = target on [0, dx], k = (w1-w0)/dx
        let k = (w1 - w0) / dx;
        let t = if k.abs() < 1e-12 {
            if w0 > 0.0 { target / w0 } else { dx }
        } else {
            let disc = (w0 * w0 + 2.0 * k * target).max(0.0);
            (disc.sqrt() - w0) / k
        };
        let hour = x0 + t.clamp(0.0, dx);
        return ((hour * 3600.0) as u32).min(86_399);
    }
    86_399 // u == 1.0 boundary / fp residue: last second of the day
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Profile; 5] = [
        Profile::Workday,
        Profile::Morning,
        Profile::Evening,
        Profile::Weekend,
        Profile::Through,
    ];

    #[test]
    fn samples_in_range_and_monotone_in_u() {
        for p in ALL {
            let mut prev = 0u32;
            for k in 0..=1000 {
                let u = k as f32 / 1001.0;
                let s = sample_departure_s(p, u);
                assert!(s < 86_400, "{p:?} out of range: {s}");
                assert!(s >= prev, "{p:?} not monotone at u={u}");
                prev = s;
            }
        }
    }

    #[test]
    fn morning_before_noon_evening_after() {
        for k in 0..100 {
            let u = k as f32 / 100.0;
            assert!(sample_departure_s(Profile::Morning, u) < 13 * 3600);
            assert!(sample_departure_s(Profile::Evening, u) >= 14 * 3600);
        }
    }

    #[test]
    fn workday_mass_concentrates_at_peaks() {
        // median of the morning half lands near the 07–08 peak
        let s = sample_departure_s(Profile::Morning, 0.5);
        assert!((6 * 3600..9 * 3600).contains(&s), "morning median {s}");
        let s = sample_departure_s(Profile::Evening, 0.5);
        assert!((16 * 3600..19 * 3600).contains(&s), "evening median {s}");
    }

    #[test]
    fn through_plateau_covers_daytime() {
        // the bulk of through traffic departs 06:00–20:00
        let lo = sample_departure_s(Profile::Through, 0.15);
        let hi = sample_departure_s(Profile::Through, 0.85);
        assert!(lo >= 5 * 3600, "through 15th pct too early: {lo}");
        assert!(hi <= 21 * 3600, "through 85th pct too late: {hi}");
    }

    #[test]
    fn deterministic() {
        assert_eq!(
            sample_departure_s(Profile::Workday, 0.37),
            sample_departure_s(Profile::Workday, 0.37)
        );
    }
}
