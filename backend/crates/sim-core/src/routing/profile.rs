use crate::routing::{Edge, EdgeKind, NodeKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModeState {
    Walking,
    Driving,
    OnTram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoutingProfileKey {
    Walk,
    Car,
    Tram,
    WalkTransit,
}

#[derive(Debug, Clone, Copy)]
pub struct RoutingProfile {
    pub key: RoutingProfileKey,
    pub walk_speed: f32,
    pub car_speed_factor: f32,
    pub tram_speed_factor: f32,
    pub board_tram_penalty: f32,
    pub alight_tram_penalty: f32,
}

impl RoutingProfile {
    pub fn for_key(key: RoutingProfileKey) -> Self {
        Self {
            key,
            walk_speed: 1.0,
            car_speed_factor: 1.0,
            tram_speed_factor: 1.0,
            board_tram_penalty: 10.0,
            alight_tram_penalty: 5.0,
        }
    }

    pub fn initial_mode(self) -> ModeState {
        match self.key {
            RoutingProfileKey::Walk | RoutingProfileKey::WalkTransit => ModeState::Walking,
            RoutingProfileKey::Car => ModeState::Driving,
            RoutingProfileKey::Tram => ModeState::OnTram,
        }
    }

    pub fn fastest_speed(self) -> f32 {
        match self.key {
            RoutingProfileKey::Walk => self.walk_speed,
            RoutingProfileKey::Car => 6.0 * self.car_speed_factor,
            RoutingProfileKey::Tram => 4.0 * self.tram_speed_factor,
            RoutingProfileKey::WalkTransit => (4.0 * self.tram_speed_factor).max(self.walk_speed),
        }
        .max(0.001)
    }

    pub fn transition(
        self,
        mode: ModeState,
        current_node_kind: NodeKind,
        edge: &Edge,
    ) -> Option<(ModeState, f32)> {
        let next = match self.key {
            RoutingProfileKey::Walk => {
                if mode == ModeState::Walking && edge.kind == EdgeKind::Footway {
                    Some((ModeState::Walking, edge.length / self.walk_speed.max(0.001)))
                } else {
                    None
                }
            }
            RoutingProfileKey::Car => {
                if mode == ModeState::Driving && edge.kind == EdgeKind::Road {
                    Some((
                        ModeState::Driving,
                        edge.length / (edge.speed_limit * self.car_speed_factor).max(0.001),
                    ))
                } else {
                    None
                }
            }
            RoutingProfileKey::Tram => {
                if mode == ModeState::OnTram && edge.kind == EdgeKind::TramTrack {
                    Some((
                        ModeState::OnTram,
                        edge.length / (edge.speed_limit * self.tram_speed_factor).max(0.001),
                    ))
                } else {
                    None
                }
            }
            RoutingProfileKey::WalkTransit => {
                self.walk_transit_transition(mode, current_node_kind, edge)
            }
        }?;
        next.1.is_finite().then_some(next)
    }

    fn walk_transit_transition(
        self,
        mode: ModeState,
        current_node_kind: NodeKind,
        edge: &Edge,
    ) -> Option<(ModeState, f32)> {
        match (mode, current_node_kind, edge.kind) {
            (ModeState::Walking, _, EdgeKind::Footway) => {
                Some((ModeState::Walking, edge.length / self.walk_speed.max(0.001)))
            }
            (ModeState::Walking, NodeKind::TransitStop, EdgeKind::TramTrack) => Some((
                ModeState::OnTram,
                edge.length / (edge.speed_limit * self.tram_speed_factor).max(0.001)
                    + self.board_tram_penalty,
            )),
            (ModeState::OnTram, _, EdgeKind::TramTrack) => Some((
                ModeState::OnTram,
                edge.length / (edge.speed_limit * self.tram_speed_factor).max(0.001),
            )),
            (ModeState::OnTram, NodeKind::TransitStop, EdgeKind::Footway) => Some((
                ModeState::Walking,
                edge.length / self.walk_speed.max(0.001) + self.alight_tram_penalty,
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{EdgeId, NodeId};

    fn edge(kind: EdgeKind, length: f32, speed_limit: f32) -> Edge {
        Edge {
            id: EdgeId(0),
            from: NodeId(0),
            to: NodeId(1),
            polyline: vec![(0.0, 0.0), (length, 0.0)],
            length,
            kind,
            speed_limit,
            capacity: 1,
            legacy_id: None,
        }
    }

    #[test]
    fn walk_profile_accepts_only_footway() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::Walk);
        assert!(
            profile
                .transition(
                    ModeState::Walking,
                    NodeKind::Intersection,
                    &edge(EdgeKind::Footway, 10.0, 1.0),
                )
                .is_some()
        );
        assert!(
            profile
                .transition(
                    ModeState::Walking,
                    NodeKind::Intersection,
                    &edge(EdgeKind::Road, 10.0, 6.0),
                )
                .is_none()
        );
        assert!(
            profile
                .transition(
                    ModeState::Walking,
                    NodeKind::Intersection,
                    &edge(EdgeKind::TramTrack, 10.0, 4.0),
                )
                .is_none()
        );
    }

    #[test]
    fn car_profile_accepts_only_road() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::Car);
        assert!(
            profile
                .transition(
                    ModeState::Driving,
                    NodeKind::Intersection,
                    &edge(EdgeKind::Road, 12.0, 6.0),
                )
                .is_some()
        );
        assert!(
            profile
                .transition(
                    ModeState::Driving,
                    NodeKind::Intersection,
                    &edge(EdgeKind::Footway, 12.0, 1.0),
                )
                .is_none()
        );
        assert!(
            profile
                .transition(
                    ModeState::Driving,
                    NodeKind::Intersection,
                    &edge(EdgeKind::TramTrack, 12.0, 4.0),
                )
                .is_none()
        );
    }

    #[test]
    fn walk_transit_boards_only_at_stops() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::WalkTransit);
        let tram = edge(EdgeKind::TramTrack, 20.0, 4.0);
        assert!(
            profile
                .transition(ModeState::Walking, NodeKind::Intersection, &tram)
                .is_none()
        );
        let (mode, cost) = profile
            .transition(ModeState::Walking, NodeKind::TransitStop, &tram)
            .expect("boarding at stop is legal");
        assert_eq!(mode, ModeState::OnTram);
        assert!(cost > 0.0);
    }

    #[test]
    fn walk_transit_alights_only_at_stops() {
        let profile = RoutingProfile::for_key(RoutingProfileKey::WalkTransit);
        let foot = edge(EdgeKind::Footway, 5.0, 1.0);
        assert!(
            profile
                .transition(ModeState::OnTram, NodeKind::Intersection, &foot)
                .is_none()
        );
        let (mode, cost) = profile
            .transition(ModeState::OnTram, NodeKind::TransitStop, &foot)
            .expect("alighting at stop is legal");
        assert_eq!(mode, ModeState::Walking);
        assert!(cost > 5.0);
    }

    #[test]
    fn fastest_speed_is_positive_for_heuristic() {
        for key in [
            RoutingProfileKey::Walk,
            RoutingProfileKey::Car,
            RoutingProfileKey::Tram,
            RoutingProfileKey::WalkTransit,
        ] {
            assert!(RoutingProfile::for_key(key).fastest_speed() > 0.0);
        }
    }
}
