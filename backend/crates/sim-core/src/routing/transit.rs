use bevy_ecs::prelude::*;
use std::collections::HashMap;

use crate::routing::graph::{EdgeId, NodeId};

#[derive(Component, Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct LineId(pub u32);

#[derive(Debug, Clone)]
pub struct TransitLine {
    pub id: LineId,
    pub name: String,
    pub edges: Vec<EdgeId>,
    pub stops: Vec<NodeId>,
    /// Legacy wire id (e.g., "route:horizontal"). `None` for lines
    /// introduced by the builder without legacy ancestry.
    pub legacy_route_id: Option<String>,
}

#[derive(Resource, Debug, Default)]
pub struct TransitLines {
    lines: Vec<TransitLine>,
    by_legacy_route_id: HashMap<String, LineId>,
}

impl TransitLines {
    pub fn new(lines: Vec<TransitLine>) -> Self {
        let mut by_legacy_route_id = HashMap::new();
        for line in &lines {
            if let Some(legacy) = &line.legacy_route_id {
                by_legacy_route_id.insert(legacy.clone(), line.id);
            }
        }
        Self { lines, by_legacy_route_id }
    }

    pub fn line(&self, id: LineId) -> &TransitLine {
        &self.lines[id.0 as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = &TransitLine> {
        self.lines.iter()
    }

    pub fn count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_by_legacy(&self, legacy_id: &str) -> Option<LineId> {
        self.by_legacy_route_id.get(legacy_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transit_lines_lookup_by_index_and_legacy() {
        let lines = vec![TransitLine {
            id: LineId(0),
            name: "tram_h".into(),
            edges: vec![EdgeId(0), EdgeId(2)],
            stops: vec![NodeId(1)],
            legacy_route_id: Some("route:horizontal".into()),
        }];
        let tl = TransitLines::new(lines);
        assert_eq!(tl.count(), 1);
        assert_eq!(tl.line(LineId(0)).name, "tram_h");
        assert_eq!(tl.line_by_legacy("route:horizontal"), Some(LineId(0)));
        assert!(tl.line_by_legacy("missing").is_none());
    }
}
