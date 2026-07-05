use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Nutzungsklasse eines Gebäudes; Zahlenwerte = `Usage`-Enum aus
/// `backend/crates/protocol/proto/world.proto` (Bake-Klassifikation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Usage {
    Unknown = 0,
    Residential = 1,
    Commercial = 2,
    Industrial = 3,
    Public = 4,
    Agriculture = 5,
}

impl Usage {
    pub fn from_bake(value: u8) -> Option<Usage> {
        match value {
            0 => Some(Usage::Unknown),
            1 => Some(Usage::Residential),
            2 => Some(Usage::Commercial),
            3 => Some(Usage::Industrial),
            4 => Some(Usage::Public),
            5 => Some(Usage::Agriculture),
            _ => None,
        }
    }
}

/// Sim-eigener Lebenszyklus-Zustand eines Gebäudes. Ab M1 im Datenmodell
/// (Snapshot + Wire); die Übergangs-Logik (Verfall, Abriss, Neubau) folgt
/// in späteren Meilensteinen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BuildingLifecycle {
    #[default]
    Occupied,
    Vacant,
    Decaying,
    Demolished,
    UnderConstruction,
}

/// Lebenszyklus-Abweichungen vom Default [`BuildingLifecycle::Occupied`],
/// BuildingId → Zustand. Nur ABWEICHUNGEN werden gehalten (und persistiert,
/// Task 10) — ein fehlender Eintrag IST `Occupied`. In M1 praktisch leer;
/// das Datenmodell steht für die späteren Übergangs-Systeme.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct BuildingStates(pub BTreeMap<u32, BuildingLifecycle>);

/// Ein Gebäude aus dem gebackenen Sim-Welt-Artefakt (`simworld.json`).
/// Positionen in lokalen Metern (Anker 8.7285°E / 47.5069°N, +x Ost, +z Süd).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimBuilding {
    /// swissBUILDINGS3D-UUID, bake-stabil.
    pub uuid: String,
    pub usage: Usage,
    pub x: f32,
    pub z: f32,
    pub area_m2: f32,
    pub height_m: f32,
    /// RoadGraph-Kantenindex (graph.pb) oder -1 ohne Strassen-Anbindung.
    pub access_edge: i64,
    /// Meter entlang der Zugangs-Kante.
    pub access_offset: f32,
}
