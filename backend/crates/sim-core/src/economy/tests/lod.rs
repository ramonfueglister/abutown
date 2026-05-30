use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::economy::{DormantMarkets, MarketChunks, MarketId, refresh_dormant_markets_system};
use crate::ids::ChunkCoord;
use crate::world::components::{ActiveChunk, AsleepChunk, ChunkCoordComp, HotChunk, WarmChunk};

#[test]
fn refresh_dormant_markets_marks_only_anchored_inactive() {
    let mut world = World::new();
    // Four chunks, one per LOD level.
    world.spawn((ChunkCoordComp(ChunkCoord { x: 0, y: 0 }), AsleepChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 1, y: 0 }), WarmChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 2, y: 0 }), ActiveChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 3, y: 0 }), HotChunk));

    let mut anchors = MarketChunks::default();
    anchors.0.insert(MarketId(10), ChunkCoord { x: 0, y: 0 }); // asleep -> dormant
    anchors.0.insert(MarketId(11), ChunkCoord { x: 1, y: 0 }); // warm   -> dormant
    anchors.0.insert(MarketId(12), ChunkCoord { x: 2, y: 0 }); // active -> awake
    anchors.0.insert(MarketId(13), ChunkCoord { x: 3, y: 0 }); // hot    -> awake
    world.insert_resource(anchors);
    world.insert_resource(DormantMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    let dormant = world.resource::<DormantMarkets>();
    let expected: BTreeSet<MarketId> = [MarketId(10), MarketId(11)].into_iter().collect();
    assert_eq!(dormant.0, expected);
}

#[test]
fn unanchored_market_is_never_dormant() {
    let mut world = World::new();
    // No active chunks at all, and the market is not anchored.
    world.insert_resource(MarketChunks::default());
    world.insert_resource(DormantMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    assert!(world.resource::<DormantMarkets>().0.is_empty());
}
