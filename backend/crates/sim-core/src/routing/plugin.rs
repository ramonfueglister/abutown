use bevy_ecs::prelude::*;
use bevy_ecs::schedule::Schedule;

use crate::city_network::CityNetwork;
use crate::routing::builder::{SeededStop, SeededWalk, build_graph_from_city_network};
use crate::routing::flow_field::FlowFieldCache;
use crate::routing::graph::Graph;
use crate::routing::hpa::{HpaConfig, HpaIndex};
use crate::routing::path_cache::PathCache;
use crate::routing::spatial_index::NodeSpatialIndex;
use crate::routing::traffic::TrafficRoutes;
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
        let (graph, traffic_routes, spatial_index) = match world.get_resource::<CityNetwork>() {
            Some(network) => {
                build_graph_from_city_network(network, &self.seeded_stops, &self.seeded_walks)
            }
            None => (
                Graph::default(),
                TrafficRoutes::default(),
                NodeSpatialIndex::default(),
            ),
        };
        world.insert_resource(graph);
        world.insert_resource(traffic_routes);
        world.insert_resource(spatial_index);
        world.insert_resource(WaitingAgents::default());
    }
}

pub struct PathfindingPlugin {
    pub cache_capacity: usize,
}

impl Default for PathfindingPlugin {
    fn default() -> Self {
        Self {
            cache_capacity: 8192,
        }
    }
}

impl SimPlugin for PathfindingPlugin {
    fn name(&self) -> &'static str {
        "pathfinding"
    }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        world.insert_resource(PathCache::with_capacity(self.cache_capacity));
    }
}

pub struct FlowFieldPlugin {
    pub cache_capacity: usize,
}

impl Default for FlowFieldPlugin {
    fn default() -> Self {
        Self {
            cache_capacity: 4096,
        }
    }
}

impl SimPlugin for FlowFieldPlugin {
    fn name(&self) -> &'static str {
        "flow_field"
    }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        world.insert_resource(FlowFieldCache::with_capacity(self.cache_capacity));
    }
}

#[derive(Default)]
pub struct HierarchicalRoutingPlugin {
    pub config: HpaConfig,
}

impl SimPlugin for HierarchicalRoutingPlugin {
    fn name(&self) -> &'static str {
        "hierarchical_routing"
    }

    fn install(&self, world: &mut World, _schedule: &mut Schedule) {
        let index = {
            let graph = world.resource::<Graph>();
            HpaIndex::build(graph, self.config)
                .expect("hierarchical routing index must build from routing graph")
        };
        world.insert_resource(index);
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
        assert!(world.contains_resource::<TrafficRoutes>());
        assert!(world.contains_resource::<NodeSpatialIndex>());
        assert!(world.contains_resource::<WaitingAgents>());
        assert_eq!(world.resource::<Graph>().node_count(), 0);
    }

    #[test]
    fn pathfinding_plugin_installs_path_cache() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        RoutingPlugin::default().install(&mut world, &mut schedule);
        PathfindingPlugin::default().install(&mut world, &mut schedule);
        assert!(world.contains_resource::<crate::routing::PathCache>());
        assert_eq!(world.resource::<crate::routing::PathCache>().len(), 0);
    }

    #[test]
    fn flow_field_plugin_installs_cache() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        FlowFieldPlugin::default().install(&mut world, &mut schedule);

        assert!(world.contains_resource::<crate::routing::FlowFieldCache>());
        assert_eq!(world.resource::<crate::routing::FlowFieldCache>().len(), 0);
    }

    #[test]
    fn hierarchical_routing_plugin_installs_hpa_index() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        RoutingPlugin::default().install(&mut world, &mut schedule);
        PathfindingPlugin::default().install(&mut world, &mut schedule);
        HierarchicalRoutingPlugin::default().install(&mut world, &mut schedule);

        assert!(world.contains_resource::<crate::routing::HpaIndex>());
        assert_eq!(
            world.resource::<crate::routing::HpaIndex>().cluster_count(),
            0
        );
    }

    #[test]
    fn hierarchical_routing_plugin_uses_custom_config() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        CorePlugin::default().install(&mut world, &mut schedule);
        RoutingPlugin::default().install(&mut world, &mut schedule);
        HierarchicalRoutingPlugin {
            config: HpaConfig {
                cluster_size_tiles: 16,
                corridor_margin_clusters: 1,
            },
        }
        .install(&mut world, &mut schedule);

        assert_eq!(
            world.resource::<crate::routing::HpaIndex>().config,
            HpaConfig {
                cluster_size_tiles: 16,
                corridor_margin_clusters: 1,
            }
        );
    }
}
