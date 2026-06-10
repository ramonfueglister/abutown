use bevy_ecs::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::collections::{HashMap, HashSet};

use crate::ids::ChunkCoord;

#[derive(Resource, Default, Debug)]
pub struct ChunksByCoord(pub HashMap<ChunkCoord, Entity>);

#[derive(Resource, Default, Debug, Copy, Clone)]
pub struct EventCount(pub usize);

#[derive(Resource, Debug, Copy, Clone)]
pub struct ChunkSizeRes(pub u16);

impl Default for ChunkSizeRes {
    fn default() -> Self {
        Self(32)
    }
}

#[derive(Resource, Default, Debug, Copy, Clone)]
pub struct WorldDimensions {
    pub width_tiles: u32,
    pub height_tiles: u32,
}

#[derive(Resource, Default, Debug)]
pub struct DirtyChunks(pub HashSet<Entity>);

/// Server-owned simulation interest. A pinned chunk is kept at least Active by
/// the LOD classifier even when no browser currently subscribes to it.
#[derive(Resource, Default, Debug, Clone)]
pub struct PinnedActiveChunks(pub HashSet<ChunkCoord>);

#[derive(Resource, Debug)]
pub struct WorldIdRes(pub String);

impl Default for WorldIdRes {
    fn default() -> Self {
        Self("abutopia".to_string())
    }
}

#[derive(Resource)]
pub struct DeterministicRng(StdRng);

impl DeterministicRng {
    pub fn from_world_id(world_id: &str) -> Self {
        let hash = blake3::hash(world_id.as_bytes());
        let bytes: [u8; 32] = *hash.as_bytes();
        Self(StdRng::from_seed(bytes))
    }

    pub fn next_u32(&mut self) -> u32 {
        use rand::RngCore;
        self.0.next_u32()
    }

    pub fn next_u64(&mut self) -> u64 {
        use rand::RngCore;
        self.0.next_u64()
    }

    pub fn next_f32(&mut self) -> f32 {
        use rand::Rng;
        self.0.r#gen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_rng_is_seeded_from_world_id() {
        let mut a = DeterministicRng::from_world_id("abutopia");
        let mut b = DeterministicRng::from_world_id("abutopia");
        assert_eq!(a.next_u64(), b.next_u64());

        let mut c = DeterministicRng::from_world_id("other-world");
        let mut d = DeterministicRng::from_world_id("abutopia");
        assert_ne!(c.next_u64(), d.next_u64());
    }

    #[test]
    fn chunks_by_coord_default_is_empty() {
        let r = ChunksByCoord::default();
        assert!(r.0.is_empty());
    }
}
