use sim_core::base_world::BaseWorldBundle;
use sim_core::mobility::{MobilityPersistSnapshot, extract_from_world};

pub(crate) fn initial_mobility_snapshot_for_base_world(
    bundle: &BaseWorldBundle,
) -> Result<MobilityPersistSnapshot, sim_core::mobility::seed::SeedError> {
    let (seeded_world, _) = sim_core::mobility::seed::from_base_world_bundle(bundle)?;
    Ok(extract_from_world(&seeded_world))
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

/// Seed-time agent count (pedestrians + drivers). This is a property of the
/// authored base world only — used for carrying capacity and fresh-seed
/// expectations, never to judge a persisted snapshot (a live population
/// legitimately drifts from it).
pub(crate) fn expected_base_world_agent_count(base_world: &BaseWorldBundle) -> usize {
    let pedestrians: usize = base_world
        .spawns
        .pedestrian_groups
        .iter()
        .filter(|group| {
            base_world
                .transport
                .pedestrian_corridors
                .iter()
                .any(|path| path.id == group.corridor_id)
        })
        .map(|group| group.agents_per_corridor as usize)
        .sum();
    // one seeded driver per seeded car
    pedestrians + expected_base_world_car_routes(base_world).len()
}

#[cfg(test)]
pub(crate) fn expected_base_world_car_count(base_world: &BaseWorldBundle) -> usize {
    expected_base_world_car_routes(base_world).len()
}
