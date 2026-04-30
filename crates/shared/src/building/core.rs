use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use crate::game::BuildingKind;
use crate::config;
use super::placement::{idx_xy, manhattan};
/// Build and upgrade costs use **gold**; see `structure_build_cost_gold`.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Building {
    pub id: u64,
    pub owner_id: u16,
    /// Linear index `y * width + x`.
    pub tile_idx: u32,
    pub kind: BuildingKind,
    pub level: u8,
    pub under_construction: bool,
    pub ticks_until_complete: u32,
}

/// Per-player totals for income / fleet gates (only **ready** structures count).
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildingAggregate {
    pub city_levels: u32,
    pub factory_levels: u32,
    pub port_levels: u32,
    pub defense_levels: u32,
    /// True if any ready Port exists (same as `port_levels > 0` when ports are never level-0 ready without existing).
    pub has_completed_port: bool,
}

/// `out[v]` is aggregate for owner id `v` (resize to `max_player_id + 1`).
pub fn aggregate_buildings_per_player(
    buildings: impl Iterator<Item = Building>,
    max_player_id: usize,
) -> Vec<BuildingAggregate> {
    let mut out = vec![BuildingAggregate::default(); max_player_id.saturating_add(1)];
    for b in buildings {
        if b.under_construction {
            continue;
        }
        let i = b.owner_id as usize;
        if i >= out.len() {
            continue;
        }
        let a = &mut out[i];
        match b.kind {
            BuildingKind::City => a.city_levels += b.level as u32,
            BuildingKind::Factory => a.factory_levels += b.level as u32,
            BuildingKind::Port => {
                a.port_levels += b.level as u32;
                a.has_completed_port = true;
            }
            BuildingKind::DefensePost => a.defense_levels += b.level as u32,
            BuildingKind::SamLauncher | BuildingKind::MissileSilo => {}
        }
    }
    out
}

/// Extra frontier priority when conquering a tile near an enemy DefensePost (defender `target_owner`).
#[inline]
pub fn defense_post_priority_bonus(
    buildings: &[Building],
    tile_x: u32,
    tile_y: u32,
    map_width: u32,
) -> i64 {
    let mut bonus: i64 = 0;
    for b in buildings {
        let (bx, by) = idx_xy(b.tile_idx, map_width);
        let d = manhattan(tile_x as i32, tile_y as i32, bx as i32, by as i32);
        if d <= config::DEFENSE_POST_RANGE {
            bonus += b.level as i64 * config::DEFENSE_POST_PRIORITY_PER_LEVEL;
        }
    }
    bonus
}

/// A spatial grid specifically designed to optimize `defense_post_priority_bonus` from O(N) to O(1).
/// This should be cached in a `Local` within the combat execution system to avoid allocations.
#[derive(Default)]
pub struct DefenseGrid {
    pub cells: Vec<Vec<Building>>,
    pub grid_w: u32,
    pub grid_h: u32,
    pub cell_size: u32,
}

impl DefenseGrid {
    /// Rebuild the grid with the specified player's defense posts.
    /// This is allocation-free after the first few calls because the `cells` array
    /// simply clears its internal `Vec`s without dropping capacity.
    pub fn rebuild(&mut self, buildings: &[Building], map_width: u32, map_height: u32, cell_size: u32, target_owner: u16) {
        let grid_w = (map_width + cell_size - 1) / cell_size;
        let grid_h = (map_height + cell_size - 1) / cell_size;
        
        self.grid_w = grid_w;
        self.grid_h = grid_h;
        self.cell_size = cell_size;
        
        let num_cells = (grid_w * grid_h) as usize;
        if self.cells.len() < num_cells {
            self.cells.resize(num_cells, Vec::new());
        }
        for cell in self.cells.iter_mut() {
            cell.clear();
        }
        
        for &b in buildings {
            if b.owner_id == target_owner && b.kind == BuildingKind::DefensePost && !b.under_construction {
                let bx = b.tile_idx % map_width;
                let by = b.tile_idx / map_width;
                let cx = bx / cell_size;
                let cy = by / cell_size;
                if cx < grid_w && cy < grid_h {
                    self.cells[(cy * grid_w + cx) as usize].push(b);
                }
            }
        }
    }

    /// Priority bonus from defender defense posts near `(tile_x, tile_y)` using the spatial grid.
    #[inline]
    pub fn priority_bonus(&self, tile_x: u32, tile_y: u32, map_width: u32) -> i64 {
        let mut bonus: i64 = 0;
        let range = config::DEFENSE_POST_RANGE as u32;

        let cx_min = tile_x.saturating_sub(range) / self.cell_size;
        let cx_max = (tile_x + range) / self.cell_size;
        let cy_min = tile_y.saturating_sub(range) / self.cell_size;
        let cy_max = (tile_y + range) / self.cell_size;

        let cx_max = cx_max.min(self.grid_w.saturating_sub(1));
        let cy_max = cy_max.min(self.grid_h.saturating_sub(1));

        for cy in cy_min..=cy_max {
            for cx in cx_min..=cx_max {
                let idx = (cy * self.grid_w + cx) as usize;
                for b in &self.cells[idx] {
                    let bx = b.tile_idx % map_width;
                    let by = b.tile_idx / map_width;
                    let d = manhattan(tile_x as i32, tile_y as i32, bx as i32, by as i32);
                    if d <= config::DEFENSE_POST_RANGE {
                        bonus += b.level as i64 * config::DEFENSE_POST_PRIORITY_PER_LEVEL;
                    }
                }
            }
        }
        bonus
    }
}

/// A generic spatial grid for all buildings, optimizing spacing checks during construction and railway updates.
#[derive(Clone)]
pub struct BuildingGrid {
    pub cells: Vec<Vec<(u32, u32)>>,
    pub grid_w: u32,
    pub grid_h: u32,
    pub cell_size: u32,
}

impl Default for BuildingGrid {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            grid_w: 0,
            grid_h: 0,
            cell_size: 32, // Prevent division by zero
        }
    }
}

impl BuildingGrid {
    /// Rebuild the grid with the specified buildings.
    pub fn rebuild<'a>(&mut self, buildings: impl Iterator<Item = &'a Building>, map_width: u32, map_height: u32, cell_size: u32, exclude_id: Option<u64>) {
        let grid_w = (map_width + cell_size - 1) / cell_size;
        let grid_h = (map_height + cell_size - 1) / cell_size;
        
        self.grid_w = grid_w;
        self.grid_h = grid_h;
        self.cell_size = cell_size;
        
        let num_cells = (grid_w * grid_h) as usize;
        if self.cells.len() < num_cells {
            self.cells.resize(num_cells, Vec::new());
        }
        for cell in self.cells.iter_mut() {
            cell.clear();
        }
        
        for b in buildings {
            if exclude_id.map(|id| b.id != id).unwrap_or(true) {
                let bx = b.tile_idx % map_width;
                let by = b.tile_idx / map_width;
                let cx = bx / cell_size;
                let cy = by / cell_size;
                if cx < grid_w && cy < grid_h {
                    self.cells[(cy * grid_w + cx) as usize].push((bx, by));
                }
            }
        }
    }

    /// Iterator over buildings within the specified Manhattan radius.
    pub fn iter_in_range(&self, x: u32, y: u32, range: u32) -> impl Iterator<Item = &(u32, u32)> {
        let cx_min = x.saturating_sub(range) / self.cell_size;
        let cx_max = (x + range) / self.cell_size;
        let cy_min = y.saturating_sub(range) / self.cell_size;
        let cy_max = (y + range) / self.cell_size;
        
        let cx_max = cx_max.min(self.grid_w.saturating_sub(1));
        let cy_max = cy_max.min(self.grid_h.saturating_sub(1));
        
        let mut idxs = Vec::with_capacity(((cx_max - cx_min + 1) * (cy_max - cy_min + 1)) as usize);
        if self.grid_w > 0 && self.grid_h > 0 && cx_min <= cx_max && cy_min <= cy_max {
            for cy in cy_min..=cy_max {
                for cx in cx_min..=cx_max {
                    idxs.push((cy * self.grid_w + cx) as usize);
                }
            }
        }
        
        let cells = &self.cells;
        idxs.into_iter().flat_map(move |i| cells[i].iter())
    }
}

