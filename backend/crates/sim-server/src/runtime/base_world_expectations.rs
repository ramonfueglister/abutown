use sim_core::base_world::BaseWorldBundle;
use sim_core::mobility::{MobilityPersistSnapshot, extract_from_world};

pub(crate) fn initial_mobility_snapshot_for_base_world(
    bundle: &BaseWorldBundle,
) -> Result<MobilityPersistSnapshot, sim_core::mobility::seed::SeedError> {
    let (seeded_world, _) = sim_core::mobility::seed::from_base_world_bundle(bundle)?;
    Ok(extract_from_world(&seeded_world))
}

/// The authored base world's concrete agent count, used to size the population
/// carrying capacity on every boot (the demographic ceiling, not a resume gate).
pub(crate) fn expected_base_world_agent_count(base_world: &BaseWorldBundle) -> usize {
    expected_base_world_pedestrian_count(base_world)
        + expected_base_world_driver_vehicles(base_world).len()
}

#[cfg(test)]
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

fn expected_base_world_pedestrian_count(base_world: &BaseWorldBundle) -> usize {
    base_world
        .spawns
        .pedestrian_groups
        .iter()
        .filter(|group| {
            // Skip a dangling corridor reference defensively (unreachable after
            // validate()); it contributes no concrete agents.
            base_world
                .transport
                .pedestrian_corridors
                .iter()
                .any(|path| path.id == group.corridor_id)
        })
        .map(|group| group.agents_per_corridor as usize)
        .sum()
}

#[cfg(test)]
pub(crate) fn expected_base_world_car_count(base_world: &BaseWorldBundle) -> usize {
    expected_base_world_car_routes(base_world).len()
}
