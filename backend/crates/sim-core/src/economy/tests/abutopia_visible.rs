//! Blocker-1: prove the abutopia world data makes residential-corridor pedestrians
//! bind home_market to a co-located consumption market (9002), so attribution can
//! route them. These tests load the REAL abutopia bundle.

use crate::base_world::BaseWorldBundle;
use crate::economy::{EconomyPlugin, seed_from_markets_layer};
use crate::mobility::market_binding::{assign_binding, markets_with_positions};
use crate::mobility::seed::from_base_world_bundle;

/// Build the full abutopia world (graph + NodeSpatialIndex via the mobility builder)
/// and seed the economy on top, so `markets_with_positions` returns the snapped
/// market node positions.
fn abutopia_world_with_economy() -> bevy_ecs::world::World {
    use crate::world::schedule::SimPlugin;
    let bundle = BaseWorldBundle::load_from_dir("../../../data/worlds/abutopia")
        .expect("abutopia bundle loads");
    let (mut world, mut schedule) =
        from_base_world_bundle(&bundle).expect("abutopia world builds from bundle");
    EconomyPlugin.install(&mut world, &mut schedule);
    seed_from_markets_layer(&mut world, &bundle.markets);
    world
}

/// Corridor:sidewalk:south spans tiles x≈106..117 at y=64.51. After re-anchoring 9002
/// onto the corridor, every pedestrian there must bind home_market = 9002 (nearest).
#[test]
fn corridor_pedestrians_bind_home_market_9002() {
    let world = abutopia_world_with_economy();
    let positions = markets_with_positions(&world);
    assert_eq!(
        positions.len(),
        4,
        "all four abutopia markets snapped to graph nodes"
    );

    for px in [106.0_f32, 111.5, 117.0] {
        let binding =
            assign_binding((px, 64.51), &positions).expect("binding exists with four live markets");
        assert_eq!(
            binding.home_market, 9002,
            "pedestrian at ({px}, 64.51) must bind home_market=9002; got {}",
            binding.home_market
        );
    }
}
