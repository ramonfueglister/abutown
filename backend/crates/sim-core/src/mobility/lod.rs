use crate::ids::ChunkCoord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MobilityActivity {
    #[default]
    Asleep,
    Warm,
    Active,
    Hot,
}

pub const ACTIVITY_HYSTERESIS_TICKS: u8 = 30;

pub fn classify_chunk_mobility_activity(
    subscribers: u8,
    population: u32,
    previous: MobilityActivity,
    cooldown_remaining: u8,
) -> MobilityActivity {
    let target = if subscribers >= 2 {
        MobilityActivity::Hot
    } else if subscribers == 1 {
        MobilityActivity::Active
    } else if population > 0 {
        MobilityActivity::Warm
    } else {
        MobilityActivity::Asleep
    };
    if target == previous || cooldown_remaining == 0 {
        target
    } else {
        previous
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowCell {
    pub population: f32,
    pub outflow: Vec<(ChunkCoord, f32)>,
    pub attractiveness: f32,
    pub last_tick: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_to_hot_when_two_or_more_subscribers() {
        assert_eq!(
            classify_chunk_mobility_activity(2, 0, MobilityActivity::Hot, 0),
            MobilityActivity::Hot
        );
        assert_eq!(
            classify_chunk_mobility_activity(5, 100, MobilityActivity::Asleep, 0),
            MobilityActivity::Hot
        );
    }

    #[test]
    fn classifies_to_active_with_single_subscriber() {
        assert_eq!(
            classify_chunk_mobility_activity(1, 0, MobilityActivity::Active, 0),
            MobilityActivity::Active
        );
        assert_eq!(
            classify_chunk_mobility_activity(1, 100, MobilityActivity::Warm, 0),
            MobilityActivity::Active
        );
    }

    #[test]
    fn classifies_to_warm_with_population_no_subscribers() {
        assert_eq!(
            classify_chunk_mobility_activity(0, 5, MobilityActivity::Warm, 0),
            MobilityActivity::Warm
        );
    }

    #[test]
    fn classifies_to_asleep_when_empty() {
        assert_eq!(
            classify_chunk_mobility_activity(0, 0, MobilityActivity::Asleep, 0),
            MobilityActivity::Asleep
        );
    }

    #[test]
    fn hysteresis_holds_previous_state_during_cooldown() {
        assert_eq!(
            classify_chunk_mobility_activity(0, 5, MobilityActivity::Hot, 10),
            MobilityActivity::Hot
        );
    }

    #[test]
    fn hysteresis_allows_transition_after_cooldown_expires() {
        assert_eq!(
            classify_chunk_mobility_activity(0, 5, MobilityActivity::Hot, 0),
            MobilityActivity::Warm
        );
    }

    #[test]
    fn flow_cell_default_is_empty_and_serializes_round_trip() {
        let cell = FlowCell::default();
        let json = serde_json::to_value(&cell).unwrap();
        let back: FlowCell = serde_json::from_value(json).unwrap();
        assert_eq!(cell, back);
    }
}
