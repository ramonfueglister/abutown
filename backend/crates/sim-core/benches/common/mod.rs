//! Shared infrastructure for the `mobility_tick*` benches.

use sim_core::city_network::{CityNetwork, NetworkCoord, WorldTiles};

pub struct SyntheticNetwork {
    pub world_id: &'static str,
    pub world_w: u32,
    pub world_h: u32,
    pub corridor_count: u32,
    pub corridor_rows: u32,
    pub corridor_x_step: i32,
    pub corridor_len: i32,
    pub arterial_count: u32,
    pub arterial_y_step: i32,
    pub arterial_len: i32,
}

impl SyntheticNetwork {
    /// Build a deterministic CityNetwork laid out as a horizontal lattice of
    /// pedestrian corridors crossed by a few long arterials.
    pub fn build(&self) -> CityNetwork {
        let corridors = (0..self.corridor_count)
            .map(|i| {
                let y = ((i % self.corridor_rows) * 2) as i32;
                let x_start = (i / self.corridor_rows) as i32 * self.corridor_x_step;
                vec![
                    NetworkCoord { x: x_start, y },
                    NetworkCoord {
                        x: x_start + self.corridor_len,
                        y,
                    },
                ]
            })
            .collect();
        let arterials = (0..self.arterial_count)
            .map(|i| {
                let y = i as i32 * self.arterial_y_step;
                vec![
                    NetworkCoord { x: 0, y },
                    NetworkCoord {
                        x: self.arterial_len,
                        y,
                    },
                ]
            })
            .collect();
        CityNetwork {
            version: 1,
            world_id: self.world_id.to_string(),
            chunk_size: 32,
            world_tiles: WorldTiles {
                width: self.world_w,
                height: self.world_h,
            },
            arterial_paths: arterials,
            pedestrian_corridors: corridors,
        }
    }
}
