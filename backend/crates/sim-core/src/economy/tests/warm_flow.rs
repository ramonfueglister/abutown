use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use crate::economy::{
    DormantMarkets, MarketChunks, MarketId, WarmMarkets, refresh_dormant_markets_system,
};
use crate::ids::ChunkCoord;
use crate::world::components::{ActiveChunk, AsleepChunk, ChunkCoordComp, WarmChunk};

#[test]
fn bridge_classifies_warm_dormant_and_active() {
    let mut world = World::new();
    world.spawn((ChunkCoordComp(ChunkCoord { x: 0, y: 0 }), AsleepChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 1, y: 0 }), WarmChunk));
    world.spawn((ChunkCoordComp(ChunkCoord { x: 2, y: 0 }), ActiveChunk));

    let mut anchors = MarketChunks::default();
    anchors.0.insert(MarketId(10), ChunkCoord { x: 0, y: 0 }); // asleep
    anchors.0.insert(MarketId(11), ChunkCoord { x: 1, y: 0 }); // warm
    anchors.0.insert(MarketId(12), ChunkCoord { x: 2, y: 0 }); // active
    world.insert_resource(anchors);
    world.insert_resource(DormantMarkets::default());
    world.insert_resource(WarmMarkets::default());

    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(refresh_dormant_markets_system);
    schedule.run(&mut world);

    let dormant = &world.resource::<DormantMarkets>().0;
    let warm = &world.resource::<WarmMarkets>().0;
    assert_eq!(*dormant, [MarketId(10), MarketId(11)].into_iter().collect::<BTreeSet<_>>());
    assert_eq!(*warm, [MarketId(11)].into_iter().collect::<BTreeSet<_>>());
}
