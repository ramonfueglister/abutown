//! Deterministic, server-authoritative simulation clock for the single shared
//! observer-world. `sim_time` is derived from the mobility `Tick`; there is no
//! player-facing speed control (see the 8i spec).

use bevy_ecs::prelude::Resource;

pub const SECONDS_PER_DAY: u64 = 86_400;
pub const DAYS_PER_YEAR: u64 = 365;
pub const SECONDS_PER_YEAR: u64 = SECONDS_PER_DAY * DAYS_PER_YEAR;
pub const SECONDS_PER_MONTH: u64 = SECONDS_PER_YEAR / 12;

/// Fixed-rate clock. `sim_seconds_per_tick` is the one tunable time-compression
/// knob; everything else is derived from the authoritative tick.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimClock {
    pub sim_seconds_per_tick: u64,
}

impl Default for SimClock {
    /// ~2000x at a 10 Hz tick (1 real day ≈ 5.48 sim-years). Tunable.
    fn default() -> Self {
        Self {
            sim_seconds_per_tick: 200,
        }
    }
}

impl SimClock {
    pub fn sim_seconds(&self, tick: u64) -> u64 {
        tick.saturating_mul(self.sim_seconds_per_tick)
    }
    pub fn calendar(&self, tick: u64) -> SimDate {
        SimDate::from_seconds(self.sim_seconds(tick))
    }
    pub fn age_seconds(&self, now_tick: u64, birth_tick: u64) -> u64 {
        now_tick
            .saturating_sub(birth_tick)
            .saturating_mul(self.sim_seconds_per_tick)
    }
    pub fn age_years(&self, now_tick: u64, birth_tick: u64) -> f32 {
        self.age_seconds(now_tick, birth_tick) as f32 / SECONDS_PER_YEAR as f32
    }
    pub fn month_index(&self, tick: u64) -> u64 {
        self.sim_seconds(tick) / SECONDS_PER_MONTH
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimDate {
    pub year: u64,
    pub day_of_year: u64,
    pub hour: u64,
    pub minute: u64,
    pub second: u64,
}

impl SimDate {
    pub fn from_seconds(s: u64) -> Self {
        let year = s / SECONDS_PER_YEAR;
        let rem = s % SECONDS_PER_YEAR;
        let day_of_year = rem / SECONDS_PER_DAY;
        let day_rem = rem % SECONDS_PER_DAY;
        Self {
            year,
            day_of_year,
            hour: day_rem / 3600,
            minute: (day_rem % 3600) / 60,
            second: day_rem % 60,
        }
    }
}

/// Plugin that installs the `SimClock` resource. No per-tick system: sim-time is
/// derived from the existing `Tick`. Future calendar-boundary events live here.
pub struct TimePlugin;

impl crate::world::schedule::SimPlugin for TimePlugin {
    fn name(&self) -> &'static str {
        "time"
    }
    fn install(
        &self,
        world: &mut bevy_ecs::world::World,
        _schedule: &mut bevy_ecs::schedule::Schedule,
    ) {
        world.insert_resource(SimClock::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_seconds_is_tick_times_rate() {
        let clock = SimClock {
            sim_seconds_per_tick: 200,
        };
        assert_eq!(clock.sim_seconds(0), 0);
        assert_eq!(clock.sim_seconds(10), 2000);
    }

    #[test]
    fn calendar_derives_from_seconds() {
        let clock = SimClock {
            sim_seconds_per_tick: 200,
        };
        // one sim-year = 31_536_000 s = 157_680 ticks at 200 s/tick
        let d = clock.calendar(157_680);
        assert_eq!(d.year, 1);
        assert_eq!(d.day_of_year, 0);
    }

    #[test]
    fn age_years_is_elapsed_ticks_times_rate() {
        let clock = SimClock {
            sim_seconds_per_tick: 200,
        };
        let years = clock.age_years(157_680, 0);
        assert!((years - 1.0).abs() < 1e-3, "got {years}");
        assert!(clock.age_years(157_680, 78_840) < clock.age_years(157_680, 0));
    }
}
