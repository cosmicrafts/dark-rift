use bevy::prelude::*;
use crate::game::{GameState, BuildingKind, GameEvent, GamePhase};
use crate::building::{Building, BuildingGrid, resolve_structure_spawn_tile, structure_build_cost_gold, structure_kind_enabled};

pub fn apply_build_structure_intent(
    game: &mut GameState,
    player_id: u16,
    kind: BuildingKind,
    target_tile: u32,
) {
    if game.phase != GamePhase::Playing {
        return;
    }
    let Some(player) = game.player(player_id) else {
        return;
    };
    if !player.alive {
        return;
    }
    if !structure_kind_enabled(kind) {
        return;
    }
    let w = game.map.width;
    let area = w.saturating_mul(game.map.height);
    if area == 0 || target_tile >= area {
        return;
    }
    let mut positions = BuildingGrid::default();
    positions.rebuild(game.buildings.iter(), w, game.map.height, 32, None);
    
    let Some(spawn_idx) =
        resolve_structure_spawn_tile(&game.map, player_id, kind, target_tile, &positions)
    else {
        bevy::log::warn!(
            "apply_build_structure: no valid spawn for {:?} at tile {}",
            kind,
            target_tile
        );
        return;
    };
    let cost = structure_build_cost_gold(kind, player_id, &game.buildings);
    let Some(player_mut) = game.player_mut(player_id) else {
        return;
    };
    if player_mut.gold < cost || !cost.is_finite() {
        return;
    }
    player_mut.gold = (player_mut.gold - cost).max(0.0);
    let id = game.next_building_id;
    game.next_building_id = game.next_building_id.wrapping_add(1).max(1);
    let dur = kind.construction_duration_ticks();
    let under = dur > 0;
    let ticks = if under { dur } else { 0 };
    game.buildings.push(Building {
        id,
        owner_id: player_id,
        tile_idx: spawn_idx,
        kind,
        level: 1,
        under_construction: under,
        ticks_until_complete: ticks,
    });
    game.events.push(GameEvent::StructureSpawned {
        id,
        owner_id: player_id,
        tile_idx: spawn_idx,
        kind,
        level: 1,
    });
}

pub fn apply_upgrade_structure_intent(
    game: &mut GameState,
    player_id: u16,
    building_id: u64,
) {
    if game.phase != GamePhase::Playing {
        return;
    }
    
    let mut found_idx = None;
    for (i, b) in game.buildings.iter().enumerate() {
        if b.id == building_id {
            if b.owner_id != player_id {
                bevy::log::warn!("apply_upgrade_structure: wrong owner for {}", building_id);
                return;
            }
            if b.under_construction {
                return;
            }
            if !b.kind.upgradable() {
                return;
            }
            found_idx = Some(i);
            break;
        }
    }
    
    let Some(idx) = found_idx else {
        bevy::log::warn!("apply_upgrade_structure: id {} not found", building_id);
        return;
    };
    
    let kind = game.buildings[idx].kind;
    let tile_idx = game.buildings[idx].tile_idx;
    
    if !structure_kind_enabled(kind) {
        return;
    }
    let cost = structure_build_cost_gold(kind, player_id, &game.buildings);
    let Some(player_mut) = game.player_mut(player_id) else {
        return;
    };
    if player_mut.gold < cost || !cost.is_finite() {
        return;
    }
    player_mut.gold = (player_mut.gold - cost).max(0.0);
    
    let b = &mut game.buildings[idx];
    b.level = b.level.saturating_add(1);
    let new_level = b.level;
    game.events.push(GameEvent::StructureUpgraded {
        id: building_id,
        tile_idx,
        kind,
        level: new_level,
    });
}

