use crate::mobility::components::*;
use crate::mobility::records::{AgentMobilityState, PlanStage};
use crate::mobility::resources::*;
use crate::world::components::{ActiveChunk, ChunkCoordComp, HotChunk, WarmChunk};
use crate::world::events::{ChunkLod, ChunkLodChanged};
use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;

mod bookkeeping;
mod common;
mod lod;
mod output;
mod routing;
mod vehicles;
mod walking;

pub use bookkeeping::{tick_increment_system, track_chunk_populations_system};
pub use lod::{
    consume_chunk_lod_transitions_system, demote_active_to_warm_system,
    promote_warm_to_active_system, refresh_simulated_chunks_system, warm_chunk_flow_system,
};
pub use output::{compute_direction_system, compute_world_coord_system};
pub use routing::{route_advance_system, route_assignment_system};
pub use vehicles::vehicle_advance_system;
pub use walking::{stop_arrival_system, update_link_polyline_cache_system, walk_advance_system};

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum MobilitySet {
    LOD,
    Advance,
    Output,
    Bookkeeping,
}

pub fn install_systems(schedule: &mut Schedule) {
    use crate::world::schedule::CoreSet;
    schedule.configure_sets((
        MobilitySet::LOD,
        MobilitySet::Advance.after(MobilitySet::LOD),
        MobilitySet::Output.after(MobilitySet::Advance),
        MobilitySet::Bookkeeping.after(MobilitySet::Output),
    ));
    // Mobility's LOD set drains the `ChunkLodChanged` event stream produced
    // by `reclassify_chunk_lod_system` (in `CoreSet::LodReclassify`), so it
    // must run after the reclassifier. The population tracker runs BEFORE
    // the reclassifier so populated-but-unsubscribed chunks get classified
    // (and demoted) in the same tick they were seeded.
    schedule.configure_sets(MobilitySet::LOD.after(CoreSet::LodReclassify));
    schedule.configure_sets(MobilitySet::Bookkeeping.before(CoreSet::EventEmit));
    // Population tracking is intentionally NOT in MobilitySet::LOD: it must
    // run BEFORE `CoreSet::LodReclassify` so reclassify sees same-tick
    // populations and can emit the Asleep→Warm transition that drives
    // demote within the same schedule run.
    schedule.add_systems(track_chunk_populations_system.before(CoreSet::LodReclassify));
    schedule.add_systems((
        refresh_simulated_chunks_system.in_set(MobilitySet::LOD),
        consume_chunk_lod_transitions_system.in_set(MobilitySet::LOD),
        promote_warm_to_active_system
            .in_set(MobilitySet::LOD)
            .after(consume_chunk_lod_transitions_system),
        demote_active_to_warm_system
            .in_set(MobilitySet::LOD)
            .after(consume_chunk_lod_transitions_system),
    ));
    // Advance set: route movement + warm flow. Ordering within Advance:
    //
    //   1. route_assignment    — assign graph routes to un-routed walkers.
    //   2. route_advance       — move completed route edges to the next edge.
    //   3. update_link_cache   — refresh edge polylines after route changes.
    //   4. walk_advance        — push Walking agents along their link.
    //   5. stop_arrival        — convert progress=1.0 walkers into terminal states.
    //   6. vehicle_advance     — decrement dwell or push cars along road routes.
    schedule.add_systems((
        route_assignment_system.in_set(MobilitySet::Advance),
        route_advance_system
            .in_set(MobilitySet::Advance)
            .after(route_assignment_system),
        update_link_polyline_cache_system
            .in_set(MobilitySet::Advance)
            .after(route_advance_system),
        walk_advance_system
            .in_set(MobilitySet::Advance)
            .after(update_link_polyline_cache_system),
        stop_arrival_system
            .in_set(MobilitySet::Advance)
            .after(walk_advance_system),
        vehicle_advance_system
            .in_set(MobilitySet::Advance)
            .after(stop_arrival_system),
        warm_chunk_flow_system.in_set(MobilitySet::Advance),
        // Output set
        compute_world_coord_system.in_set(MobilitySet::Output),
        compute_direction_system.in_set(MobilitySet::Output),
        // Bookkeeping
        tick_increment_system.in_set(MobilitySet::Bookkeeping),
    ));
}

/// Advance a plan cursor by one; cyclic, non-empty plans wrap back to the start.
pub fn advance_cursor(plan: &mut crate::mobility::components::WalkPlan) {
    plan.cursor += 1;
    if plan.cyclic && !plan.stages.is_empty() && plan.cursor >= plan.stages.len() {
        plan.cursor = 0;
    }
}

#[cfg(test)]
mod route_execution_tests;
#[cfg(test)]
mod tests;
