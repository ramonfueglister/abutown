use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// 24 Welt-Stunden vergehen in 4 realen Stunden (BitCraft-Hybridmodell,
/// Spec §Zeitmodell). Datum/Saison/Wetter bleiben real; nur Sonne/Mond und
/// Wirtschaftsprozesse laufen auf dieser Uhr.
pub const WORLD_TIME_SCALE: u64 = 6;

/// Tick-Rate des Sim-Loops (traffic DT = 0.1 s).
pub const TICKS_PER_SECOND: u64 = 10;

pub const WORLD_SECONDS_PER_DAY: u64 = 86_400;

/// Die eine Weltuhr. Zählt ausschliesslich Ticks — keine Wanduhr, damit
/// frozen-time gilt: Server down = Welt friert ein, Resume setzt exakt fort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource, Serialize, Deserialize, Default)]
pub struct WorldClock {
    pub world_tick: u64,
}

impl WorldClock {
    pub fn advance(&mut self) {
        self.world_tick += 1;
    }

    /// Vergangene Welt-Sekunden seit Weltbeginn.
    pub fn world_seconds(&self) -> u64 {
        self.world_tick * WORLD_TIME_SCALE / TICKS_PER_SECOND
    }

    /// Sekunde innerhalb des aktuellen Weltentags (0..86_400).
    pub fn s_of_world_day(&self) -> u32 {
        (self.world_seconds() % WORLD_SECONDS_PER_DAY) as u32
    }

    /// Index des aktuellen Weltentags seit Weltbeginn.
    pub fn world_day(&self) -> u64 {
        self.world_seconds() / WORLD_SECONDS_PER_DAY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_real_hours_is_one_world_day() {
        let mut c = WorldClock { world_tick: 0 };
        for _ in 0..(4 * 3600 * TICKS_PER_SECOND) {
            c.advance();
        }
        assert_eq!(c.world_day(), 1);
        assert_eq!(c.s_of_world_day(), 0);
    }

    #[test]
    fn resume_continues_exactly() {
        let c = WorldClock {
            world_tick: 987_654,
        };
        let s = serde_json::to_string(&c).unwrap();
        let r: WorldClock = serde_json::from_str(&s).unwrap();
        assert_eq!(r.world_tick, 987_654);
        assert_eq!(r.s_of_world_day(), c.s_of_world_day());
    }

    #[test]
    fn world_time_runs_six_times_real_time() {
        // 10 reale Minuten = 6000 Ticks → 60 Welt-Minuten.
        let c = WorldClock { world_tick: 6_000 };
        assert_eq!(c.world_seconds(), 3_600);
    }
}
