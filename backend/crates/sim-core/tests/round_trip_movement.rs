use sim_core::base_world::BaseWorldBundle;
use sim_core::ids::{AgentId, ChunkCoord};
use sim_core::mobility::api;
use sim_core::mobility::components::{Position, WalkPlan};
use sim_core::mobility::resources::AgentIdIndex;
use sim_core::mobility::{PlanStage, apply_into_world, extract_from_world, seed};

const WALKER: &str = "agent:walk:0";

fn abutopia_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace root")
        .join("data/worlds/abutopia")
}

fn agent_entity(world: &bevy_ecs::world::World, id: &str) -> bevy_ecs::entity::Entity {
    *world
        .resource::<AgentIdIndex>()
        .0
        .get(&AgentId(id.to_string()))
        .expect("agent:walk:0 seeded")
}

/// The south-sidewalk corridor's two endpoints, read live from the world data
/// so the assertions track the actual abutopia geometry (not hardcoded coords
/// that silently drift when the world is regenerated).
fn south_corridor_ends(bundle: &BaseWorldBundle) -> ((f32, f32), (f32, f32)) {
    let corridor = bundle
        .transport
        .pedestrian_corridors
        .iter()
        .find(|c| c.id == "corridor:sidewalk:south")
        .expect("south sidewalk corridor exists");
    let first = corridor.points.first().expect("corridor has points");
    let last = corridor.points.last().expect("corridor has points");
    ((first.x, first.y), (last.x, last.y))
}

#[test]
fn abutopia_pedestrian_is_seeded_with_cyclic_round_trip_plan() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("bundle loads");
    let (world, _schedule) = seed::from_base_world_bundle(&bundle).expect("seed ok");
    let e = agent_entity(&world, WALKER);
    let plan = world.get::<WalkPlan>(e).expect("walk plan");
    assert!(plan.cyclic, "abutopia pedestrian plan must be cyclic");
    let activities: Vec<&str> = plan
        .stages
        .iter()
        .filter_map(|s| match s {
            PlanStage::WalkToActivity { activity_id, .. } => Some(activity_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(activities, vec!["activity:home", "activity:destination"]);
}

/// Step the abutopia mobility schedule, returning the walker's x each tick.
///
/// The LOD demote system despawns ("collapses into a flow cell") every
/// individual agent in a chunk the instant that chunk classifies to Warm. To
/// keep the lone pedestrian observable while it walks, re-assert a strong
/// (>=2 -> Hot) subscription over the agent's *current* chunk (and neighbours)
/// before every tick. The chunk is derived from the live position, so this is
/// robust to where the agent is and to world regeneration.
fn run_x_trace(ticks: usize) -> Vec<f32> {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("bundle loads");
    let (mut world, mut schedule) = seed::from_base_world_bundle(&bundle).expect("seed ok");
    let id = AgentId(WALKER.to_string());
    let mut xs = Vec::with_capacity(ticks);
    for tick in 0..ticks {
        if let Some(p) = world
            .resource::<AgentIdIndex>()
            .0
            .get(&id)
            .copied()
            .and_then(|e| world.get::<Position>(e).copied())
        {
            let here = sim_core::mobility::chunk_of(p.x, p.y, 32);
            let watched: Vec<ChunkCoord> = (-1..=1)
                .flat_map(|dx| {
                    (-1..=1).map(move |dy| ChunkCoord {
                        x: here.x + dx,
                        y: here.y + dy,
                    })
                })
                .collect();
            // Two subscribers -> Hot, applied synchronously before the step so
            // classify sees it regardless of in-schedule system ordering.
            api::apply_subscription_diff(
                &mut world,
                watched.iter(),
                std::iter::empty::<&ChunkCoord>(),
            );
            api::apply_subscription_diff(
                &mut world,
                watched.iter(),
                std::iter::empty::<&ChunkCoord>(),
            );
        }
        api::tick_mobility(&mut world, &mut schedule);
        let e = world
            .resource::<AgentIdIndex>()
            .0
            .get(&id)
            .copied()
            .unwrap_or_else(|| panic!("agent:walk:0 vanished at tick {tick}"));
        let p = world.get::<Position>(e).expect("position");
        xs.push(p.x);
    }
    xs
}

#[test]
fn abutopia_pedestrian_oscillates_between_corridor_ends() {
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("bundle loads");
    let (home, dest) = south_corridor_ends(&bundle);
    let (west, east) = (home.0.min(dest.0), home.0.max(dest.0));

    let xs = run_x_trace(1400);
    let max_x = xs.iter().cloned().fold(f32::MIN, f32::max);
    let argmax = xs.iter().position(|&x| x == max_x).unwrap();
    assert!(
        max_x >= east - 1.0,
        "should reach the east corridor end (~{east}), got max {max_x}"
    );
    // After reaching the east end it must head back west toward home.
    let min_after = xs[argmax..].iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        min_after <= west + 1.0,
        "should return toward home (~{west}) after the east end, got {min_after}"
    );
}

#[test]
fn round_trip_is_deterministic() {
    assert_eq!(
        run_x_trace(400),
        run_x_trace(400),
        "identical worlds → identical trajectory"
    );
}

#[test]
fn cyclic_round_trip_survives_snapshot_extract_apply() {
    // Production never steps the seed world: it seeds, extracts a
    // MobilityPersistSnapshot, then hydrates the live world from that snapshot
    // (sim-server runtime: from_base_world_bundle -> extract_from_world ->
    // apply_into_world). The round-trip intent must survive that path. If
    // `cyclic` or the WalkToActivity stages were dropped in extract/apply, the
    // integration tests above would still pass (they step the seed world
    // directly) while the running game wandered — the exact green-tests /
    // broken-production trap. Guard the field-preservation explicitly.
    let bundle = BaseWorldBundle::load_from_dir(abutopia_root()).expect("bundle loads");
    let (seed_world, _schedule) = seed::from_base_world_bundle(&bundle).expect("seed ok");
    let snapshot = extract_from_world(&seed_world);

    let (mut hydrated, _schedule) = api::empty_world_and_schedule();
    apply_into_world(&mut hydrated, snapshot);

    let e = agent_entity(&hydrated, WALKER);
    let plan = hydrated
        .get::<WalkPlan>(e)
        .expect("walk plan after snapshot hydration");
    assert!(
        plan.cyclic,
        "cyclic flag must survive seed -> extract -> apply (the live hydration path)"
    );
    let activities: Vec<&str> = plan
        .stages
        .iter()
        .filter_map(|s| match s {
            PlanStage::WalkToActivity { activity_id, .. } => Some(activity_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(activities, vec!["activity:home", "activity:destination"]);
}
