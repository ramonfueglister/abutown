//! Serde types mirroring the baked `trafficnet.json` schema exactly (field
//! names/casing match `scripts/geo/lib/trafficnet.mjs`, the producer). No
//! defaults / heal-on-load: every field is required, missing/malformed JSON
//! is a deserialize error surfaced by `serde_json` through `NetError::Parse`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Anchor {
    pub lon: f64,
    pub lat: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub anchor: Anchor,
    pub lane_width: f32,
    pub cell_size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Signal,
    Roundabout,
    Priority,
    Uncontrolled,
    DeadEnd,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalPhase {
    pub green_s: f32,
    pub turns: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Signal {
    pub cycle_s: f32,
    pub phases: Vec<SignalPhase>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: u32,
    pub x: f32,
    pub z: f32,
    pub kind: NodeKind,
    pub signal: Option<Signal>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub id: u32,
    pub from: u32,
    pub to: u32,
    pub speed_ms: f32,
    pub lane_count: u32,
    pub priority_road: bool,
    pub lanes: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lane {
    pub id: u32,
    pub edge: u32,
    pub index: u32,
    pub length_m: f32,
    pub pts: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: u32,
    pub from_lane: u32,
    pub to_lane: u32,
    pub node: u32,
    pub conflicts_with: Vec<u32>,
    pub yields_to: Vec<u32>,
}

/// The raw, on-wire schema — what `serde_json` deserializes directly. Kept
/// separate from [`crate::TrafficNet`], which adds the precomputed CSR /
/// arc-length lookups on top.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficNetDoc {
    pub meta: Meta,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub lanes: Vec<Lane>,
    pub turns: Vec<Turn>,
}
