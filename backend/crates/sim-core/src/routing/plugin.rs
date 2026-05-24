use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

use crate::city_network::CityNetwork;
use crate::routing::builder::{SeededStop, SeededWalk, build_graph_from_city_network};
use crate::routing::graph::Graph;
use crate::routing::spatial_index::NodeSpatialIndex;
use crate::routing::transit::TransitLines;
use crate::routing::waiting::WaitingAgents;
use crate::world::schedule::SimPlugin;

#[derive(Default)]
pub struct RoutingPlugin {
    pub seeded_stops: Vec<SeededStop>,
    pub seeded_walks: Vec<SeededWalk>,
}

impl SimPlugin for RoutingPlugin {
    fn name(&self) -> &'static str {
        "routing"
    }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        let (graph, transit_lines, spatial_index) = match world.get_resource::<CityNetwork>() {
            Some(network) => {
                build_graph_from_city_network(network, &self.seeded_stops, &self.seeded_walks)
            }
            None => (
                Graph::default(),
                TransitLines::default(),
                NodeSpatialIndex::default(),
            ),
        };
        world.insert_resource(graph);
        world.insert_resource(transit_lines);
        world.insert_resource(spatial_index);
        world.insert_resource(WaitingAgents::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::plugin::CorePlugin;

    #[test]
    fn routing_plugin_installs_empty_graph_without_city_network() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        RoutingPlugin::default().install(&mut world, &mut schedule);
        assert!(world.contains_resource::<Graph>());
        assert!(world.contains_resource::<TransitLines>());
        assert!(world.contains_resource::<NodeSpatialIndex>());
        assert!(world.contains_resource::<WaitingAgents>());
        assert_eq!(world.resource::<Graph>().node_count(), 0);
    }
}
