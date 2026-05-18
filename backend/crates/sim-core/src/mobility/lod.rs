use crate::ids::ChunkCoord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Per-destination outflow rate (population units / tick). JSON keys are
    /// `ChunkCoord` structs, so the map serializes as a deterministic
    /// `Vec<(ChunkCoord, f32)>` via `chunk_keyed_map`.
    #[serde(with = "chunk_keyed_map")]
    pub outflow: HashMap<ChunkCoord, f32>,
    pub attractiveness: f32,
    pub last_tick: u64,
}

mod chunk_keyed_map {
    use super::{ChunkCoord, HashMap};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        map: &HashMap<ChunkCoord, f32>,
        ser: S,
    ) -> Result<S::Ok, S::Error> {
        let mut entries: Vec<(ChunkCoord, f32)> = map.iter().map(|(k, v)| (*k, *v)).collect();
        entries.sort_unstable_by_key(|(c, _)| (c.x, c.y));
        entries.serialize(ser)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        de: D,
    ) -> Result<HashMap<ChunkCoord, f32>, D::Error> {
        let entries: Vec<(ChunkCoord, f32)> = Vec::deserialize(de)?;
        Ok(entries.into_iter().collect())
    }
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
