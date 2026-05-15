use abutown_protocol::ChunkStateDto;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkActivity {
    Asleep,
    Warm,
    Active,
    Hot,
}

impl From<ChunkActivity> for ChunkStateDto {
    fn from(value: ChunkActivity) -> Self {
        match value {
            ChunkActivity::Asleep => Self::Asleep,
            ChunkActivity::Warm => Self::Warm,
            ChunkActivity::Active => Self::Active,
            ChunkActivity::Hot => Self::Hot,
        }
    }
}

impl From<ChunkStateDto> for ChunkActivity {
    fn from(value: ChunkStateDto) -> Self {
        match value {
            ChunkStateDto::Asleep => Self::Asleep,
            ChunkStateDto::Warm => Self::Warm,
            ChunkStateDto::Active => Self::Active,
            ChunkStateDto::Hot => Self::Hot,
        }
    }
}

pub fn classify_chunk_activity(player_count: u32, dirty_tile_pressure: u32) -> ChunkActivity {
    if player_count >= 64 || dirty_tile_pressure >= 256 {
        return ChunkActivity::Hot;
    }
    if player_count > 0 {
        return ChunkActivity::Active;
    }
    if dirty_tile_pressure > 0 {
        return ChunkActivity::Warm;
    }
    ChunkActivity::Asleep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_activity_scales_with_players_and_mutation_pressure() {
        assert_eq!(classify_chunk_activity(0, 0), ChunkActivity::Asleep);
        assert_eq!(classify_chunk_activity(0, 4), ChunkActivity::Warm);
        assert_eq!(classify_chunk_activity(1, 0), ChunkActivity::Active);
        assert_eq!(classify_chunk_activity(80, 0), ChunkActivity::Hot);
        assert_eq!(classify_chunk_activity(3, 400), ChunkActivity::Hot);
    }
}
