use crate::game::{BuildingKind, GameEvent, GamePhase};
use crate::engine::DarkRiftEngine;
use crate::building::{Building, resolve_structure_spawn_tile, structure_build_cost_gold, structure_kind_enabled};


impl DarkRiftEngine {
pub(super) fn apply_build_structure_intent(
    &mut self,
    player_id: u16,
    kind: BuildingKind,
    target_tile: u32,
) {
    if self.state.phase != GamePhase::Playing {
        return;
    }
    let Some(player) = self.state.player(player_id) else {
        return;
    };
    if !player.alive {
        return;
    }
    if !structure_kind_enabled(kind) {
        return;
    }
    let w = self.state.map.width;
    let area = w.saturating_mul(self.state.map.height);
    if area == 0 || target_tile >= area {
        return;
    }
    
    self.refresh_building_grid();
    let Some(spawn_idx) =
        resolve_structure_spawn_tile(&self.state.map, player_id, kind, target_tile, &self.building_grid, &mut self.placement_scratch)
    else {
        bevy::log::warn!(
            "apply_build_structure: no valid spawn for {:?} at tile {}",
            kind,
            target_tile
        );
        return;
    };
    let cost = structure_build_cost_gold(kind, player_id, &self.buildings);
    let Some(player_mut) = self.state.player_mut(player_id) else {
        return;
    };
    if player_mut.gold < cost || !cost.is_finite() {
        return;
    }
    player_mut.gold = (player_mut.gold - cost).max(0.0);
    let id = self.state.next_building_id;
    self.state.next_building_id = self.state.next_building_id.wrapping_add(1).max(1);
    let dur = kind.construction_duration_ticks();
    let under = dur > 0;
    let ticks = if under { dur } else { 0 };
    self.add_building(Building {
        id,
        owner_id: player_id,
        tile_idx: spawn_idx,
        kind,
        level: 1,
        under_construction: under,
        ticks_until_complete: ticks,
    });
    self.state.events.push(GameEvent::StructureSpawned {
        id,
        owner_id: player_id,
        tile_idx: spawn_idx,
        kind,
        level: 1,
    });
}

pub(super) fn apply_upgrade_structure_intent(
    &mut self,
    player_id: u16,
    building_id: u64,
) {
    if self.state.phase != GamePhase::Playing {
        return;
    }
    let mut found: Option<(usize, BuildingKind, u32)> = None;
    if let Ok(idx) = self.buildings.binary_search_by_key(&building_id, |b| b.id) {
        let b = &self.buildings[idx];
        // ID matches since we used binary_search_by_key
        if b.owner_id != player_id {
            return;
        }
        if b.under_construction {
            return;
        }
        if !b.kind.upgradable() {
            return;
        }
        found = Some((idx, b.kind, b.tile_idx));
    }
    let Some((idx, kind, tile_idx)) = found else {
        bevy::log::warn!("apply_upgrade_structure: id {} not found", building_id);
        return;
    };
    if !structure_kind_enabled(kind) {
        return;
    }
    
    let cost = structure_build_cost_gold(kind, player_id, &self.buildings);
    let Some(player_mut) = self.state.player_mut(player_id) else {
        return;
    };
    if player_mut.gold < cost || !cost.is_finite() {
        return;
    }
    player_mut.gold = (player_mut.gold - cost).max(0.0);
    
    let b = &mut self.buildings[idx];
    b.level = b.level.saturating_add(1);
    let new_level = b.level;
    self.state.events.push(GameEvent::StructureUpgraded {
        id: building_id,
        tile_idx,
        kind,
        level: new_level,
    });
}
}

