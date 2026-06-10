use sim_core::base_world::BaseWorldBundle;
use sim_core::mobility::{MobilityPersistSnapshot, extract_from_world};

pub(crate) fn initial_mobility_snapshot_for_base_world(
    bundle: &BaseWorldBundle,
) -> Result<MobilityPersistSnapshot, sim_core::mobility::seed::SeedError> {
    let (seeded_world, _) = sim_core::mobility::seed::from_base_world_bundle(bundle)?;
    Ok(extract_from_world(&seeded_world))
}

/// Does a persisted snapshot belong to THIS base-world generation?
///
/// The population is DEMOGRAPHIC (births mint new agent ids, deaths remove
/// seeded ones, agents wander off their seed link into activities and
/// vehicles), so identity with the freshly-seeded world is the wrong
/// criterion — the static-era version of this check silently reseeded the
/// live world (tick→0) on every restart, killing frozen-time resume
/// (2026-06-10). What actually signals "snapshot of another world
/// generation" is GEOMETRY:
/// - cars are statically authored (no demographics) — exact match stays;
/// - every corridor link the snapshot references must exist in this base
///   world, and its persisted polyline must match the authored corridor
///   (a regenerated/relocated world ⇒ mismatch ⇒ reseed);
/// - non-corridor walking links (graph edges, grass lattice) and activity
///   ids are re-derived from the loaded world at hydrate time, so they
///   cannot go stale and are not checked here;
/// - an `InVehicle` reference to a vehicle the snapshot does not carry is
///   internally inconsistent ⇒ reseed.
pub(crate) fn mobility_snapshot_matches_base_world(
    snapshot: &MobilityPersistSnapshot,
    base_world: &BaseWorldBundle,
) -> bool {
    let expected_cars = expected_base_world_car_routes(base_world);
    if snapshot.vehicles.len() != expected_cars.len()
        || !snapshot.vehicles.values().all(|vehicle| {
            vehicle.kind == sim_core::mobility::VehicleKind::Car
                && expected_cars
                    .get(&vehicle.id.0)
                    .is_some_and(|route_id| route_id == &vehicle.route_id)
        })
    {
        return false;
    }

    const CORRIDOR_LINK_PREFIX: &str = "link:walk:corridor:";
    let authored_corridors: std::collections::HashMap<String, Vec<(f32, f32)>> = base_world
        .transport
        .pedestrian_corridors
        .iter()
        .enumerate()
        .map(|(index, corridor)| {
            (
                format!("{CORRIDOR_LINK_PREFIX}{index}"),
                corridor
                    .points
                    .iter()
                    .map(|point| (point.x, point.y))
                    .collect(),
            )
        })
        .collect();

    // Every corridor polyline the snapshot persisted must match the authored
    // geometry — this catches a regenerated world even for links no agent
    // currently walks.
    let corridor_polylines_current = snapshot
        .link_polylines
        .iter()
        .filter(|(link_id, _)| link_id.starts_with(CORRIDOR_LINK_PREFIX))
        .all(|(link_id, polyline)| {
            authored_corridors
                .get(link_id)
                .is_some_and(|authored| polylines_match(polyline, authored))
        });
    if !corridor_polylines_current {
        return false;
    }

    snapshot.agents.values().all(|agent| match &agent.state {
        sim_core::mobility::AgentMobilityState::Walking { link_id, .. } => {
            !link_id.starts_with(CORRIDOR_LINK_PREFIX) || authored_corridors.contains_key(link_id)
        }
        sim_core::mobility::AgentMobilityState::InVehicle { vehicle_id, .. } => {
            snapshot.vehicles.contains_key(vehicle_id)
        }
        _ => true,
    })
}

pub(crate) fn normalize_seeded_agent_birth_ticks(
    snapshot: &mut MobilityPersistSnapshot,
    base_world: &BaseWorldBundle,
) {
    let mut seed_ids = expected_base_world_pedestrian_walks(base_world);
    seed_ids.extend(
        expected_base_world_driver_vehicles(base_world)
            .keys()
            .cloned(),
    );

    let clock = sim_core::time::SimClock::default();
    let now_tick = snapshot.tick;

    for id in seed_ids {
        let agent_id = sim_core::ids::AgentId(id);
        let Some(agent) = snapshot.agents.get_mut(&agent_id) else {
            continue;
        };
        if agent.birth_tick == 0 {
            agent.birth_tick = sim_core::mobility::seed::seeded_birth_tick_for_agent_id(
                &agent.id, now_tick, &clock,
            );
        }
    }
}

pub(crate) fn expected_base_world_car_routes(
    base_world: &BaseWorldBundle,
) -> std::collections::HashMap<String, String> {
    let mut expected = std::collections::HashMap::new();
    for group in &base_world.spawns.car_groups {
        let Some(arterial_index) = base_world
            .transport
            .arterial_paths
            .iter()
            .position(|path| path.id == group.arterial_id)
        else {
            continue; // unreachable after validate(); skip defensively rather than abort
        };
        let route_id = format!("route:arterial:{arterial_index}");
        for n in 0..group.cars_per_arterial {
            expected.insert(
                format!("vehicle:car:{arterial_index}:{n}"),
                route_id.clone(),
            );
        }
    }
    expected
}

pub(crate) fn expected_base_world_agent_count(base_world: &BaseWorldBundle) -> usize {
    expected_base_world_pedestrian_walks(base_world).len()
        + expected_base_world_driver_vehicles(base_world).len()
}

fn expected_base_world_driver_vehicles(
    base_world: &BaseWorldBundle,
) -> std::collections::HashMap<String, String> {
    let mut expected = std::collections::HashMap::new();
    let mut driver_index = 0u32;
    for group in &base_world.spawns.car_groups {
        let Some(arterial_index) = base_world
            .transport
            .arterial_paths
            .iter()
            .position(|path| path.id == group.arterial_id)
        else {
            continue; // unreachable after validate(); skip defensively rather than abort
        };
        for n in 0..group.cars_per_arterial {
            expected.insert(
                format!("agent:driver:{driver_index}"),
                format!("vehicle:car:{arterial_index}:{n}"),
            );
            driver_index += 1;
        }
    }
    expected
}

/// Seeded pedestrian agent ids (`agent:walk:N`). Only ids — since the
/// 2026-06-10 resume fix the base-world match no longer pins seeded agents
/// to their seed link/polyline (the population is demographic); the ids are
/// still needed for seed-count capacity and birth-tick normalization.
fn expected_base_world_pedestrian_walks(
    base_world: &BaseWorldBundle,
) -> std::collections::HashSet<String> {
    let mut expected = std::collections::HashSet::new();
    let mut agent_index = 0u32;
    for group in &base_world.spawns.pedestrian_groups {
        if !base_world
            .transport
            .pedestrian_corridors
            .iter()
            .any(|path| path.id == group.corridor_id)
        {
            continue; // unreachable after validate(); skip defensively rather than abort
        }
        for _ in 0..group.agents_per_corridor {
            expected.insert(format!("agent:walk:{agent_index}"));
            agent_index += 1;
        }
    }
    expected
}

fn polylines_match(actual: &[(f32, f32)], expected: &[(f32, f32)]) -> bool {
    const EPSILON: f32 = 0.001;
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| {
                (actual.0 - expected.0).abs() <= EPSILON && (actual.1 - expected.1).abs() <= EPSILON
            })
}

#[cfg(test)]
pub(crate) fn expected_base_world_car_count(base_world: &BaseWorldBundle) -> usize {
    expected_base_world_car_routes(base_world).len()
}
