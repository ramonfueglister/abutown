use bevy_ecs::prelude::*;
use std::collections::{HashMap, VecDeque};

use crate::ids::AgentId;
use crate::routing::graph::NodeId;

#[derive(Resource, Debug, Default)]
pub struct WaitingAgents(HashMap<NodeId, VecDeque<AgentId>>);

impl WaitingAgents {
    pub fn enqueue(&mut self, node: NodeId, agent: AgentId) {
        self.0.entry(node).or_default().push_back(agent);
    }

    pub fn dequeue(&mut self, node: NodeId) -> Option<AgentId> {
        self.0.get_mut(&node).and_then(|q| q.pop_front())
    }

    pub fn queue(&self, node: NodeId) -> Option<&VecDeque<AgentId>> {
        self.0.get(&node)
    }

    pub fn queue_mut(&mut self, node: NodeId) -> &mut VecDeque<AgentId> {
        self.0.entry(node).or_default()
    }

    pub fn remove_agent(&mut self, node: NodeId, agent: &AgentId) -> bool {
        if let Some(q) = self.0.get_mut(&node) {
            if let Some(pos) = q.iter().position(|a| a == agent) {
                q.remove(pos);
                return true;
            }
        }
        false
    }

    pub fn iter(&self) -> impl Iterator<Item = (&NodeId, &VecDeque<AgentId>)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.values().all(|q| q.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_and_dequeue_preserve_order() {
        let mut w = WaitingAgents::default();
        w.enqueue(NodeId(0), AgentId("a".into()));
        w.enqueue(NodeId(0), AgentId("b".into()));
        assert_eq!(w.dequeue(NodeId(0)).unwrap().0, "a");
        assert_eq!(w.dequeue(NodeId(0)).unwrap().0, "b");
        assert!(w.dequeue(NodeId(0)).is_none());
    }

    #[test]
    fn dequeue_empty_returns_none() {
        let mut w = WaitingAgents::default();
        assert!(w.dequeue(NodeId(42)).is_none());
    }

    #[test]
    fn remove_agent_targets_specific_id() {
        let mut w = WaitingAgents::default();
        w.enqueue(NodeId(0), AgentId("a".into()));
        w.enqueue(NodeId(0), AgentId("b".into()));
        assert!(w.remove_agent(NodeId(0), &AgentId("a".into())));
        assert_eq!(w.dequeue(NodeId(0)).unwrap().0, "b");
    }
}
