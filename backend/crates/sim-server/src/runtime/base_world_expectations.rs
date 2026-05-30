use sim_core::base_world::BaseWorldBundle;
use sim_core::mobility::{MobilityPersistSnapshot, extract_from_world};

pub(crate) fn initial_mobility_snapshot_for_base_world(
    bundle: &BaseWorldBundle,
) -> Result<MobilityPersistSnapshot, sim_core::mobility::seed::SeedError> {
    let (seeded_world, _) = sim_core::mobility::seed::from_base_world_bundle(bundle)?;
    Ok(extract_from_world(&seeded_world))
}

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

    let expected_pedestrians = expected_base_world_pedestrian_walks(base_world);
    let expected_drivers = expected_base_world_driver_vehicles(base_world);
    if snapshot.agents.len() != expected_pedestrians.len() + expected_drivers.len() {
        return false;
    }

    snapshot.agents.values().all(|agent| {
        if let Some(vehicle_id) = expected_drivers.get(&agent.id.0) {
            return matches!(
                &agent.state,
                sim_core::mobility::AgentMobilityState::InVehicle { vehicle_id: actual, .. }
                    if actual.0 == *vehicle_id
            );
        }

        let Some(expected) = expected_pedestrians.get(&agent.id.0) else {
            return false;
        };
        let sim_core::mobility::AgentMobilityState::Walking { link_id, .. } = &agent.state else {
            return false;
        };
        link_id == &expected.link_id
            && snapshot
                .link_polylines
                .get(link_id)
                .is_none_or(|polyline| polylines_match(polyline, &expected.polyline))
    })
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

struct ExpectedPedestrianWalk {
    link_id: String,
    polyline: Vec<(f32, f32)>,
}

fn expected_base_world_pedestrian_walks(
    base_world: &BaseWorldBundle,
) -> std::collections::HashMap<String, ExpectedPedestrianWalk> {
    let mut expected = std::collections::HashMap::new();
    let mut agent_index = 0u32;
    for group in &base_world.spawns.pedestrian_groups {
        let Some(corridor_index) = base_world
            .transport
            .pedestrian_corridors
            .iter()
            .position(|path| path.id == group.corridor_id)
        else {
            continue; // unreachable after validate(); skip defensively rather than abort
        };
        let corridor = &base_world.transport.pedestrian_corridors[corridor_index];
        let polyline: Vec<(f32, f32)> = corridor
            .points
            .iter()
            .map(|point| (point.x, point.y))
            .collect();
        for _ in 0..group.agents_per_corridor {
            expected.insert(
                format!("agent:walk:{agent_index}"),
                ExpectedPedestrianWalk {
                    link_id: format!("link:walk:corridor:{corridor_index}"),
                    polyline: polyline.clone(),
                },
            );
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
