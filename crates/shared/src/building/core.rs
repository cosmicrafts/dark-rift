use bevy::prelude::*;
use crate::game::BuildingKind;
use crate::config;
use super::placement::{idx_xy, manhattan};
/// Build and upgrade costs use **gold**; see `structure_build_cost_gold`.

#[derive(Component, Debug, Clone, Copy)]
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
    defender_id: u16,
    tile_x: u32,
    tile_y: u32,
    map_width: u32,
) -> i64 {
    let mut bonus: i64 = 0;
    for b in buildings {
        if b.owner_id != defender_id || b.kind != BuildingKind::DefensePost || b.under_construction {
            continue;
        }
        let (bx, by) = idx_xy(b.tile_idx, map_width);
        let d = manhattan(tile_x as i32, tile_y as i32, bx as i32, by as i32);
        if d <= config::DEFENSE_POST_RANGE {
            bonus += b.level as i64 * config::DEFENSE_POST_PRIORITY_PER_LEVEL;
        }
    }
    bonus
}
