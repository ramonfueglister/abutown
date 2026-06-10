use bevy_ecs::prelude::*;
use std::collections::HashMap;

use crate::routing::graph::EdgeId;

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct TrafficRouteId(pub u32);

#[derive(Debug, Clone)]
pub struct TrafficRoute {
    pub id: TrafficRouteId,
    pub name: String,
    pub edges: Vec<EdgeId>,
    pub legacy_route_id: String,
}

#[derive(Resource, Debug, Default)]
pub struct TrafficRoutes {
    routes: Vec<TrafficRoute>,
    by_legacy_route_id: HashMap<String, TrafficRouteId>,
}

impl TrafficRoutes {
    pub fn new(routes: Vec<TrafficRoute>) -> Self {
        let mut by_legacy_route_id = HashMap::new();
        for route in &routes {
            by_legacy_route_id.insert(route.legacy_route_id.clone(), route.id);
        }
        Self {
            routes,
            by_legacy_route_id,
        }
    }

    pub fn route(&self, id: TrafficRouteId) -> &TrafficRoute {
        &self.routes[id.0 as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = &TrafficRoute> {
        self.routes.iter()
    }

    pub fn count(&self) -> usize {
        self.routes.len()
    }

    pub fn route_by_legacy(&self, legacy_id: &str) -> Option<TrafficRouteId> {
        self.by_legacy_route_id.get(legacy_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_routes_lookup_by_legacy_id() {
        let routes = TrafficRoutes::new(vec![TrafficRoute {
            id: TrafficRouteId(0),
            name: "arterial_0".into(),
            edges: vec![EdgeId(3)],
            legacy_route_id: "route:arterial:0".into(),
        }]);

        assert_eq!(routes.count(), 1);
        assert_eq!(
            routes.route_by_legacy("route:arterial:0"),
            Some(TrafficRouteId(0))
        );
        assert!(routes.route_by_legacy("route:unknown:0").is_none());
    }
}
