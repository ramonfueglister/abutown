mod building;

pub use building::{BuildingLifecycle, SimBuilding, Usage};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("simworld json invalid: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("building {uuid} has invalid usage {value}")]
    BadUsage { uuid: String, value: u8 },
}

#[derive(Debug, Deserialize)]
struct RawSimWorld {
    buildings: Vec<RawBuilding>,
}

#[derive(Debug, Deserialize)]
struct RawBuilding {
    id: String,
    usage: u8,
    x: f32,
    z: f32,
    area_m2: f32,
    height_m: f32,
    access_edge: i64,
    access_offset: f32,
}

/// Unveränderliche Sim-Sicht auf die gebaute Stadt. `BuildingId` = Index in
/// `buildings`; durch die Sortierung nach UUID ist er über Bakes hinweg
/// stabil, solange der Gebäudebestand gleich bleibt.
#[derive(Debug)]
pub struct SimWorld {
    pub buildings: Vec<SimBuilding>,
    residential: Vec<u32>,
    workplaces: Vec<u32>,
}

impl SimWorld {
    pub fn load(json: &str) -> Result<SimWorld, WorldError> {
        let raw: RawSimWorld = serde_json::from_str(json)?;
        let mut buildings = Vec::with_capacity(raw.buildings.len());
        for b in raw.buildings {
            let usage = Usage::from_bake(b.usage).ok_or(WorldError::BadUsage {
                uuid: b.id.clone(),
                value: b.usage,
            })?;
            buildings.push(SimBuilding {
                uuid: b.id,
                usage,
                x: b.x,
                z: b.z,
                area_m2: b.area_m2,
                height_m: b.height_m,
                access_edge: b.access_edge,
                access_offset: b.access_offset,
            });
        }
        buildings.sort_by(|a, b| a.uuid.cmp(&b.uuid));

        let mut residential = Vec::new();
        let mut workplaces = Vec::new();
        for (index, b) in buildings.iter().enumerate() {
            let index = u32::try_from(index).expect("more than u32::MAX buildings");
            match b.usage {
                Usage::Residential => residential.push(index),
                Usage::Commercial | Usage::Industrial | Usage::Public => workplaces.push(index),
                Usage::Unknown | Usage::Agriculture => {}
            }
        }

        Ok(SimWorld {
            buildings,
            residential,
            workplaces,
        })
    }

    /// BuildingId-Indizes aller Wohngebäude.
    pub fn residential(&self) -> &[u32] {
        &self.residential
    }

    /// BuildingId-Indizes aller Arbeitsorte (Commercial + Industrial + Public).
    pub fn workplaces(&self) -> &[u32] {
        &self.workplaces
    }

    /// Alle Gebäude im Umkreis `r` Meter um `(cx, cz)`, aufsteigend nach
    /// BuildingId. Linearer Scan — nur für Seeding gedacht, kein Hot-Path.
    pub fn within_radius(&self, cx: f32, cz: f32, r: f32) -> Vec<u32> {
        let r2 = r * r;
        self.buildings
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                let dx = b.x - cx;
                let dz = b.z - cz;
                dx * dx + dz * dz <= r2
            })
            .map(|(i, _)| i as u32)
            .collect()
    }
}

/// 3-Gebäude-Fixture (Wohnhaus {B1} 200 m²/9 m → 3 Geschosse/Kapazität 15,
/// Workplace {A2}, Unknown {C3}) — geteilt von model-, citizens- und
/// rhythm-Tests.
#[cfg(test)]
pub(crate) mod test_fixture {
    pub(crate) const FIXTURE: &str = r#"{
      "meta": {"anchor": {"lon": 8.7285, "lat": 47.5069}, "bake_version": 1},
      "buildings": [
        {"id":"{B1}","usage":1,"x":0.0,"z":0.0,"area_m2":200.0,"height_m":9.0,"access_edge":5,"access_offset":2.0},
        {"id":"{A2}","usage":2,"x":100.0,"z":0.0,"area_m2":400.0,"height_m":12.0,"access_edge":7,"access_offset":1.0},
        {"id":"{C3}","usage":0,"x":500.0,"z":500.0,"area_m2":50.0,"height_m":4.0,"access_edge":-1,"access_offset":0.0}
      ]}"#;
}

#[cfg(test)]
mod tests {
    use super::test_fixture::FIXTURE;
    use super::*;

    #[test]
    fn loads_and_indexes_by_usage() {
        let w = SimWorld::load(FIXTURE).unwrap();
        assert_eq!(w.buildings.len(), 3);
        // sortiert nach uuid: {A2},{B1},{C3}
        assert_eq!(w.buildings[0].uuid, "{A2}");
        assert_eq!(w.residential(), &[1]);
        assert_eq!(w.workplaces(), &[0]);
        assert_eq!(w.within_radius(0.0, 0.0, 150.0), vec![0, 1]);
    }

    #[test]
    fn rejects_unknown_usage_value() {
        let bad = r#"{"buildings":[{"id":"{X}","usage":9,"x":0,"z":0,"area_m2":1,"height_m":1,"access_edge":-1,"access_offset":0}]}"#;
        assert!(matches!(
            SimWorld::load(bad),
            Err(WorldError::BadUsage { value: 9, .. })
        ));
    }
}
