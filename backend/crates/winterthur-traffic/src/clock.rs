//! Europe/Zurich wall clock for the trip spawner: binds sim ticks to real
//! local time-of-day (spec §5 "clock binding").
//!
//! At boot the server records the local seconds-since-midnight and the local
//! date; time-of-day at tick `t` is `boot_s_of_day + t·DT` wrapping daily,
//! and [`WallClock::day_kind`] re-evaluates workday/weekend (plus a small
//! authored fixed-date Swiss holiday list) at each wrap.
//!
//! DST caveat: elapsed days are derived by pure `mod 86400` arithmetic, so a
//! DST transition after boot shifts the sim clock by ±1 h relative to real
//! local time until the next reboot. Accepted — servers reboot far more
//! often than DST changes, and demand curves are hour-scale.

use crate::demand::DayKind;
use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Timelike, Utc};
use chrono_tz::Europe::Zurich;
use traffic_core::DT;

/// Seconds per day.
const DAY_S: u64 = 86_400;

/// Ticks per second. The integer tick→seconds math in [`WallClock::s_of_day`]
/// (`tick / TICKS_PER_S`) is only exact because `DT` is exactly 0.1 s; the
/// const assert below fails the build if the kernel tick ever changes.
const TICKS_PER_S: u64 = 10;
const _: () = assert!(DT == 0.1, "WallClock integer math assumes DT == 0.1 s");

/// Fixed-date Swiss national holidays (month, day) treated as weekend for
/// demand purposes: Neujahr, Berchtoldstag, Bundesfeier, Weihnachten,
/// Stephanstag. Good Friday / Easter Monday are NOT included — their dates
/// are variable (computus) and deliberately out of scope for this authored
/// list; worst case those two days run the weekday demand curve.
const HOLIDAYS: &[(u32, u32)] = &[(1, 1), (1, 2), (8, 1), (12, 25), (12, 26)];

/// Wall-clock anchor captured at boot, in Europe/Zurich local time.
#[derive(Debug, Clone, Copy)]
pub struct WallClock {
    /// Seconds since local midnight at tick 0.
    boot_s_of_day: u32,
    /// Local calendar date at tick 0.
    boot_date: NaiveDate,
}

impl WallClock {
    /// Anchor the clock at `now_utc`, converted to Europe/Zurich.
    /// `override_at` (the `ABUTOWN_TRAFFIC_AT` dev override) replaces the
    /// boot time-of-day but keeps the real local date.
    pub fn new(now_utc: DateTime<Utc>, override_at: Option<NaiveTime>) -> WallClock {
        let local = now_utc.with_timezone(&Zurich);
        let time = override_at.unwrap_or_else(|| local.time());
        WallClock {
            boot_s_of_day: time.num_seconds_from_midnight(),
            boot_date: local.date_naive(),
        }
    }

    /// Local seconds-since-midnight at `tick`, wrapping daily:
    /// `(boot_s_of_day + tick·DT) mod 86400`, computed in exact integer math.
    pub fn s_of_day(&self, tick: u64) -> u32 {
        u32::try_from((u64::from(self.boot_s_of_day) + tick / TICKS_PER_S) % DAY_S)
            .expect("mod 86400 fits u32")
    }

    /// Day class at `tick`: the boot date advanced by the elapsed whole
    /// days; Saturday, Sunday and the authored [`HOLIDAYS`] are weekend.
    pub fn day_kind(&self, tick: u64) -> DayKind {
        let elapsed_days = (u64::from(self.boot_s_of_day) + tick / TICKS_PER_S) / DAY_S;
        let date = self
            .boot_date
            .checked_add_days(chrono::Days::new(elapsed_days))
            .expect("date within chrono range");
        let weekend = matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun)
            || HOLIDAYS.contains(&(date.month(), date.day()));
        if weekend {
            DayKind::Weekend
        } else {
            DayKind::Workday
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Build the UTC instant whose Europe/Zurich local time is the given
    /// naive date+time (panics on ambiguous/skipped DST times — tests use
    /// safe times only).
    fn zurich(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime<Utc> {
        Zurich
            .with_ymd_and_hms(y, m, d, hh, mm, 0)
            .single()
            .expect("unambiguous local time")
            .with_timezone(&Utc)
    }

    #[test]
    fn override_at_sets_boot_seconds() {
        let clock = WallClock::new(
            zurich(2026, 7, 3, 14, 22),
            Some(NaiveTime::from_hms_opt(7, 30, 0).unwrap()),
        );
        assert_eq!(clock.s_of_day(0), 27_000);
    }

    #[test]
    fn s_of_day_advances_one_second_per_ten_ticks() {
        let clock = WallClock::new(
            zurich(2026, 7, 3, 0, 0),
            Some(NaiveTime::from_hms_opt(7, 30, 0).unwrap()),
        );
        assert_eq!(clock.s_of_day(0), 27_000);
        assert_eq!(clock.s_of_day(9), 27_000); // sub-second ticks truncate
        assert_eq!(clock.s_of_day(10), 27_001);
        assert_eq!(clock.s_of_day(600), 27_060);
    }

    #[test]
    fn real_boot_time_without_override() {
        // 2026-07-03 is CEST (UTC+2): local 07:30 == 05:30 UTC.
        let now = Utc.with_ymd_and_hms(2026, 7, 3, 5, 30, 0).unwrap();
        let clock = WallClock::new(now, None);
        assert_eq!(clock.s_of_day(0), 27_000);
    }

    #[test]
    fn day_kind_flips_workday_to_weekend_across_friday_midnight() {
        // Friday 2026-07-03 23:59 local boot.
        let clock = WallClock::new(zurich(2026, 7, 3, 23, 59), None);
        assert_eq!(clock.day_kind(0), DayKind::Workday);
        assert_eq!(clock.s_of_day(0), 86_340);

        // 60 s later it is Saturday 2026-07-04 00:00.
        let tick = 60 * 10;
        assert_eq!(clock.s_of_day(tick), 0);
        assert_eq!(clock.day_kind(tick), DayKind::Weekend);

        // Sunday stays weekend; Monday 2026-07-06 is a workday again.
        let day = 86_400 * 10;
        assert_eq!(clock.day_kind(tick + day), DayKind::Weekend);
        assert_eq!(clock.day_kind(tick + 2 * day), DayKind::Workday);
    }

    #[test]
    fn august_first_is_weekend_even_on_a_friday() {
        // 2025-08-01 is a Friday, but Bundesfeier → weekend demand.
        let clock = WallClock::new(zurich(2025, 8, 1, 12, 0), None);
        assert_eq!(clock.day_kind(0), DayKind::Weekend);
        // Saturday 2025-08-02 also weekend; Monday 2025-08-04 workday.
        let day = 86_400 * 10;
        assert_eq!(clock.day_kind(day), DayKind::Weekend);
        assert_eq!(clock.day_kind(3 * day), DayKind::Workday);
    }
}
