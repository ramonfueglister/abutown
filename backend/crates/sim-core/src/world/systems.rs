use std::time::Instant;

use bevy_ecs::prelude::*;

use crate::ids::ChunkCoord;
use crate::scheduler::ChunkActivity;
use crate::tile::{TileKind, TileRecord};
use crate::world::components::{
    ActiveChunk, AsleepChunk, ChunkCoordComp, ChunkSize, ChunkSubscriberCount, ChunkVersion,
    DirtyTiles, HotChunk, LastPersistedVersion, LastSnapshotAt, LodCooldown, Tiles, WarmChunk,
};
use crate::world::events::*;
use crate::world::resources::{ChunksByCoord, DirtyChunks, PinnedActiveChunks};

/// Pump message buffers — Bevy's `Messages<T>` requires periodic `update()`
/// calls to drop already-read messages from the buffer. We do it once per
/// tick in `CoreSet::EventEmit` so downstream consumers (mobility, persistence,
/// future plugins) read against a fresh buffer next tick.
pub fn flush_event_buffers(
    mut chunk_loaded: ResMut<Messages<ChunkLoaded>>,
    mut chunk_unloaded: ResMut<Messages<ChunkUnloaded>>,
    mut tile_changed: ResMut<Messages<TileChanged>>,
    mut chunk_lod_changed: ResMut<Messages<ChunkLodChanged>>,
) {
    chunk_loaded.update();
    chunk_unloaded.update();
    tile_changed.update();
    chunk_lod_changed.update();
}

/// Spawn a chunk entity from the supplied chunk data. Inserts the entity into
/// `ChunksByCoord` and writes a `ChunkLoaded` message. Returns the new entity.
pub fn spawn_chunk_entity(
    world: &mut World,
    coord: ChunkCoord,
    chunk_size: u16,
    initial_tiles: Vec<TileRecord>,
    initial_version: u64,
    activity: ChunkActivity,
) -> Entity {
    let mut entity_commands = world.spawn((
        ChunkCoordComp(coord),
        ChunkSize(chunk_size),
        Tiles(initial_tiles),
        ChunkVersion(initial_version),
        DirtyTiles::default(),
        LastPersistedVersion(initial_version),
        LastSnapshotAt(Instant::now()),
        LodCooldown(0),
        ChunkSubscriberCount(0),
    ));
    match activity {
        ChunkActivity::Asleep => {
            entity_commands.insert(AsleepChunk);
        }
        ChunkActivity::Warm => {
            entity_commands.insert(WarmChunk);
        }
        ChunkActivity::Active => {
            entity_commands.insert(ActiveChunk);
        }
        ChunkActivity::Hot => {
            entity_commands.insert(HotChunk);
        }
    }
    let entity = entity_commands.id();
    world
        .resource_mut::<ChunksByCoord>()
        .0
        .insert(coord, entity);
    world
        .resource_mut::<Messages<ChunkLoaded>>()
        .write(ChunkLoaded {
            entity,
            coord,
            initial_version,
        });
    entity
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::schedule::Schedule;

    #[test]
    fn flush_event_buffers_runs_inside_schedule() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        // Write an event; running the schedule should not panic and should
        // rotate the buffer.
        let entity = world.spawn_empty().id();
        world
            .resource_mut::<Messages<ChunkLoaded>>()
            .write(ChunkLoaded {
                entity,
                coord: ChunkCoord { x: 0, y: 0 },
                initial_version: 0,
            });
        schedule.run(&mut world);
        // No panic = pass. Explicit assertions on buffer rotation are
        // brittle across bevy versions.
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TileMutationError {
    #[error("chunk not loaded: {coord:?}")]
    ChunkNotLoaded { coord: ChunkCoord },
    #[error("tile index {index} out of bounds (tile_count={tile_count})")]
    TileOutOfBounds { index: u16, tile_count: u32 },
    #[error("no state change: tile {local_index} in chunk {coord:?} already has kind {kind:?}")]
    NoStateChange {
        coord: ChunkCoord,
        local_index: u16,
        kind: TileKind,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct TileMutationResult {
    pub chunk_entity: Entity,
    pub new_version: u64,
    pub old_kind: TileKind,
}

/// Apply a tile-kind change to a chunk entity. Bumps version, updates
/// `Tiles`, marks `DirtyTiles`, writes `TileChanged` message. Returns the
/// new chunk version on success.
pub fn apply_set_tile_kind_ecs(
    world: &mut World,
    coord: ChunkCoord,
    local_index: u16,
    new_kind: TileKind,
    tick: u64,
) -> Result<TileMutationResult, TileMutationError> {
    let entity = *world
        .resource::<ChunksByCoord>()
        .0
        .get(&coord)
        .ok_or(TileMutationError::ChunkNotLoaded { coord })?;
    let (old_kind, new_version) = {
        let mut chunk_ent = world.entity_mut(entity);
        let old_kind;
        let new_version;
        {
            let mut tiles = chunk_ent
                .get_mut::<Tiles>()
                .expect("Tiles component on chunk entity");
            let tile_count = tiles.0.len() as u32;
            if local_index as u32 >= tile_count {
                return Err(TileMutationError::TileOutOfBounds {
                    index: local_index,
                    tile_count,
                });
            }
            old_kind = tiles.0[local_index as usize].kind;
            if old_kind == new_kind {
                return Err(TileMutationError::NoStateChange {
                    coord,
                    local_index,
                    kind: new_kind,
                });
            }
            tiles.0[local_index as usize].kind = new_kind;
            tiles.0[local_index as usize].flags.modified = true;
        }
        {
            let mut version = chunk_ent
                .get_mut::<ChunkVersion>()
                .expect("ChunkVersion on chunk entity");
            version.0 += 1;
            new_version = version.0;
        }
        // Re-borrow Tiles to update the per-tile version. Two separate get_mut
        // calls are required because we cannot hold both ChunkVersion and
        // Tiles mut at once.
        chunk_ent
            .get_mut::<Tiles>()
            .expect("Tiles component on chunk entity")
            .0[local_index as usize]
            .version = new_version;
        chunk_ent
            .get_mut::<DirtyTiles>()
            .expect("DirtyTiles on chunk entity")
            .0
            .insert(local_index);
        (old_kind, new_version)
    };
    world.resource_mut::<DirtyChunks>().0.insert(entity);
    world
        .resource_mut::<Messages<TileChanged>>()
        .write(TileChanged {
            chunk: entity,
            coord,
            local_index,
            old_kind,
            new_kind,
            new_version,
            tick,
        });
    Ok(TileMutationResult {
        chunk_entity: entity,
        new_version,
        old_kind,
    })
}

/// Query helper: collect chunk snapshot data for a coord. Returns `None`
/// if no chunk entity is loaded at that coord.
pub fn chunk_snapshot_data(
    world: &World,
    coord: ChunkCoord,
) -> Option<(u16, u64, Vec<TileRecord>, ChunkActivity)> {
    let entity = *world.resource::<ChunksByCoord>().0.get(&coord)?;
    let tiles = world.get::<Tiles>(entity)?.0.clone();
    let chunk_size = world.get::<ChunkSize>(entity)?.0;
    let version = world.get::<ChunkVersion>(entity)?.0;
    let activity = if world.get::<HotChunk>(entity).is_some() {
        ChunkActivity::Hot
    } else if world.get::<ActiveChunk>(entity).is_some() {
        ChunkActivity::Active
    } else if world.get::<WarmChunk>(entity).is_some() {
        ChunkActivity::Warm
    } else {
        ChunkActivity::Asleep
    };
    Some((chunk_size, version, tiles, activity))
}

#[cfg(test)]
mod ecs_mutation_tests {
    use super::*;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::schedule::Schedule;

    #[test]
    fn apply_set_tile_kind_ecs_bumps_version_and_writes_message() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        let coord = ChunkCoord { x: 2, y: 3 };
        let _entity = spawn_chunk_entity(
            &mut world,
            coord,
            4,
            vec![TileRecord::default(); 16],
            0,
            ChunkActivity::Active,
        );
        let result = apply_set_tile_kind_ecs(&mut world, coord, 5, TileKind::Road, 1).unwrap();
        assert_eq!(result.new_version, 1);
        let entity = world.resource::<ChunksByCoord>().0[&coord];
        let tiles = world.get::<Tiles>(entity).unwrap();
        assert_eq!(tiles.0[5].kind, TileKind::Road);
        let dirty = world.get::<DirtyTiles>(entity).unwrap();
        assert!(dirty.0.contains(&5));
        let messages = world.resource::<Messages<TileChanged>>();
        let mut cursor = messages.get_cursor();
        let read: Vec<_> = cursor.read(messages).collect();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].new_kind, TileKind::Road);
    }

    #[test]
    fn apply_set_tile_kind_ecs_rejects_no_state_change() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        let coord = ChunkCoord { x: 0, y: 0 };
        let _entity = spawn_chunk_entity(
            &mut world,
            coord,
            4,
            vec![TileRecord::default(); 16],
            0,
            ChunkActivity::Active,
        );
        let err = apply_set_tile_kind_ecs(&mut world, coord, 5, TileKind::Grass, 1).unwrap_err();
        assert!(matches!(err, TileMutationError::NoStateChange { .. }));
    }
}

// === LOD reclassification (CoreSet::LodReclassify) ===================

/// Hysteresis window: a chunk that just transitioned holds its new marker
/// for this many ticks before the next transition can fire. Matches
/// `mobility::lod::ACTIVITY_HYSTERESIS_TICKS`.
const LOD_COOLDOWN_TICKS: u8 = 30;

fn current_lod_marker(world: &World, entity: Entity) -> ChunkLod {
    if world.get::<HotChunk>(entity).is_some() {
        ChunkLod::Hot
    } else if world.get::<ActiveChunk>(entity).is_some() {
        ChunkLod::Active
    } else if world.get::<WarmChunk>(entity).is_some() {
        ChunkLod::Warm
    } else {
        ChunkLod::Asleep
    }
}

fn classify_target(
    subscribers: u8,
    population: u32,
    pinned_active: bool,
    previous: ChunkLod,
    cooldown_remaining: u8,
) -> ChunkLod {
    let target = if subscribers >= 2 {
        ChunkLod::Hot
    } else if subscribers == 1 || pinned_active {
        ChunkLod::Active
    } else if population > 0 {
        ChunkLod::Warm
    } else {
        ChunkLod::Asleep
    };
    if target != previous && cooldown_remaining > 0 {
        previous
    } else {
        target
    }
}

/// Reclassify every chunk entity's LOD marker based on subscriber count
/// and population. Swaps marker components atomically, decrements the
/// per-entity `LodCooldown`, and writes `ChunkLodChanged` messages for
/// every transition. Runs in `CoreSet::LodReclassify`.
///
/// Source of truth: per-entity `ChunkSubscriberCount` + the optional
/// mobility-owned `ChunkPopulations` resource. There is no longer any
/// compat-shim sync with a separate `ChunkSubscribers` / `ChunkActivities`
/// resource — those were deleted in Phase 8a follow-ups.
///
/// Uses a cached `QueryState` in a `Local` so we don't pay the
/// `world.query::<…>()` allocation each tick on the exclusive-system path.
type ChunkClassifyQuery = bevy_ecs::query::QueryState<(
    Entity,
    &'static ChunkCoordComp,
    &'static ChunkSubscriberCount,
    &'static LodCooldown,
)>;

pub fn reclassify_chunk_lod_system(
    world: &mut World,
    mut query: Local<Option<ChunkClassifyQuery>>,
) {
    // Phase 0: any populated chunk that doesn't yet have a chunk entity gets
    // an empty-tiles entity spawned for it — the classifier must see it to
    // emit the Asleep→Warm transition that drives the mobility demote path.
    // Subscriber-only spawns (from `apply_subscription_diff`) already happen
    // upstream; this catches chunks with agent/vehicle population but no
    // subscribers (e.g. seeded directly via `spawn_agent_from_record`).
    let needed: Vec<ChunkCoord> = world
        .get_resource::<crate::mobility::resources::ChunkPopulations>()
        .map(|p| {
            let by_coord = &world.resource::<ChunksByCoord>().0;
            p.0.iter()
                .filter(|(c, n)| **n > 0 && !by_coord.contains_key(*c))
                .map(|(c, _)| *c)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !needed.is_empty() {
        let chunk_size = world.resource::<crate::world::resources::ChunkSizeRes>().0;
        for coord in needed {
            // Invalidate the cached query: spawning new chunk entities adds
            // archetypes the cached QueryState would otherwise miss. Drop it
            // so the next `get_or_insert_with` recreates it.
            *query = None;
            spawn_chunk_entity(
                world,
                coord,
                chunk_size,
                Vec::new(),
                0,
                crate::scheduler::ChunkActivity::Asleep,
            );
        }
    }

    // Phase 1: collect work (immutable read of components + populations).
    let mut transitions: Vec<(Entity, ChunkCoord, ChunkLod, ChunkLod)> = Vec::new();
    let mut cooldown_updates: Vec<(Entity, u8)> = Vec::new();
    {
        let chunk_populations = world
            .get_resource::<crate::mobility::resources::ChunkPopulations>()
            .map(|p| p.0.clone())
            .unwrap_or_default();
        let pinned_active_chunks = world
            .get_resource::<PinnedActiveChunks>()
            .map(|pins| pins.0.clone())
            .unwrap_or_default();
        let q = query.get_or_insert_with(|| {
            world.query::<(Entity, &ChunkCoordComp, &ChunkSubscriberCount, &LodCooldown)>()
        });
        for (entity, coord, sub, cooldown) in q.iter(world) {
            let pop = chunk_populations.get(&coord.0).copied().unwrap_or(0);
            let previous = current_lod_marker(world, entity);
            let target = classify_target(
                sub.0,
                pop,
                pinned_active_chunks.contains(&coord.0),
                previous,
                cooldown.0,
            );
            let new_cooldown = cooldown.0.saturating_sub(1);
            cooldown_updates.push((entity, new_cooldown));
            if target != previous {
                transitions.push((entity, coord.0, previous, target));
            }
        }
    }

    // Phase 2: apply cooldown decrements (transitions will overwrite below).
    for (entity, new_cd) in cooldown_updates {
        if let Some(mut cd) = world.entity_mut(entity).get_mut::<LodCooldown>() {
            cd.0 = new_cd;
        }
    }

    // Phase 3: swap LOD marker components, reset cooldown, write events.
    for (entity, coord, from, to) in transitions {
        {
            let mut e = world.entity_mut(entity);
            match from {
                ChunkLod::Asleep => {
                    e.remove::<AsleepChunk>();
                }
                ChunkLod::Warm => {
                    e.remove::<WarmChunk>();
                }
                ChunkLod::Active => {
                    e.remove::<ActiveChunk>();
                }
                ChunkLod::Hot => {
                    e.remove::<HotChunk>();
                }
            }
            match to {
                ChunkLod::Asleep => {
                    e.insert(AsleepChunk);
                }
                ChunkLod::Warm => {
                    e.insert(WarmChunk);
                }
                ChunkLod::Active => {
                    e.insert(ActiveChunk);
                }
                ChunkLod::Hot => {
                    e.insert(HotChunk);
                }
            }
            e.insert(LodCooldown(LOD_COOLDOWN_TICKS));
        }
        world
            .resource_mut::<Messages<ChunkLodChanged>>()
            .write(ChunkLodChanged {
                entity,
                coord,
                from,
                to,
            });
    }
}

#[cfg(test)]
mod lod_reclassify_tests {
    use super::*;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::{CoreSet, SimPlugin};
    use bevy_ecs::schedule::Schedule;

    #[test]
    fn warm_to_active_when_subscriber_arrives() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        // The LOD reclassify system reads `ChunkPopulations` (mobility
        // resource). Post-Phase-8a it no longer writes any compat shim.
        world.insert_resource(crate::mobility::resources::ChunkPopulations::default());
        CorePlugin::default().install(&mut world, &mut schedule);
        schedule.add_systems(reclassify_chunk_lod_system.in_set(CoreSet::LodReclassify));

        let coord = ChunkCoord { x: 0, y: 0 };
        let entity = spawn_chunk_entity(
            &mut world,
            coord,
            4,
            vec![TileRecord::default(); 16],
            0,
            ChunkActivity::Warm,
        );
        world
            .entity_mut(entity)
            .get_mut::<ChunkSubscriberCount>()
            .unwrap()
            .0 = 1;
        // LodCooldown is 0 from spawn; the system will keep it at 0 until
        // a transition fires, at which point it bumps to LOD_COOLDOWN_TICKS.
        schedule.run(&mut world);
        assert!(
            world.get::<ActiveChunk>(entity).is_some(),
            "should have transitioned to Active",
        );
        assert!(world.get::<WarmChunk>(entity).is_none());

        let messages = world.resource::<Messages<ChunkLodChanged>>();
        let mut cursor = messages.get_cursor();
        let read: Vec<_> = cursor.read(messages).collect();
        assert!(
            read.iter()
                .any(|e| e.entity == entity && e.to == ChunkLod::Active),
            "ChunkLodChanged should be emitted for Warm -> Active",
        );
    }

    #[test]
    fn pinned_chunk_stays_active_without_browser_subscriber() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        world.insert_resource(crate::mobility::resources::ChunkPopulations::default());
        CorePlugin::default().install(&mut world, &mut schedule);

        let coord = ChunkCoord { x: 3, y: 2 };
        let entity = spawn_chunk_entity(
            &mut world,
            coord,
            4,
            vec![TileRecord::default(); 16],
            0,
            ChunkActivity::Warm,
        );
        world.resource_mut::<PinnedActiveChunks>().0.insert(coord);

        schedule.run(&mut world);

        assert!(
            world.get::<ActiveChunk>(entity).is_some(),
            "pinned chunks should remain concrete-simulated without viewport subscribers",
        );
        assert_eq!(
            world.get::<ChunkSubscriberCount>(entity).unwrap().0,
            0,
            "server-side pins must not fake browser subscriber counts",
        );
        assert!(world.get::<WarmChunk>(entity).is_none());
    }
}

#[cfg(test)]
mod spawn_tests {
    use super::*;
    use crate::ids::ChunkCoord;
    use crate::tile::TileRecord;
    use crate::world::plugin::CorePlugin;
    use crate::world::schedule::SimPlugin;
    use bevy_ecs::schedule::Schedule;

    #[test]
    fn spawn_chunk_entity_populates_chunks_by_coord_and_emits_loaded() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);

        let coord = ChunkCoord { x: 7, y: 11 };
        let entity = spawn_chunk_entity(
            &mut world,
            coord,
            4,
            vec![TileRecord::default(); 16],
            3,
            ChunkActivity::Warm,
        );

        // Indexed in ChunksByCoord
        assert_eq!(world.resource::<ChunksByCoord>().0[&coord], entity);

        // Has the right marker
        assert!(world.get::<WarmChunk>(entity).is_some());

        // ChunkLoaded was written
        let messages = world.resource::<Messages<ChunkLoaded>>();
        let mut cursor = messages.get_cursor();
        let read: Vec<_> = cursor.read(messages).collect();
        assert!(read.iter().any(|e| e.entity == entity && e.coord == coord));
    }
}
