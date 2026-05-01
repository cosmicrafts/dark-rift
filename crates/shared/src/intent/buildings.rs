use bevy::prelude::*;
use crate::game::{GameState, BuildingKind, GameEvent, GamePhase};
use crate::building::{Building, resolve_structure_spawn_tile, structure_build_cost_gold, existing_structure_positions_with_width, structure_kind_enabled};
fn snapshot_buildings(q: &mut Query<(Entity, &mut Building)>) -> Vec<Building> {
    q.iter().map(|(_, b)| *b).collect()
}

pub fn apply_build_structure_intent(
    commands: &mut Commands,
    game: &mut GameState,
    q_building: &mut Query<(Entity, &mut Building)>,
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
    let snapshot = snapshot_buildings(q_building);
    let positions = existing_structure_positions_with_width(snapshot.iter(), w, None);
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
    let cost = structure_build_cost_gold(kind, player_id, &snapshot);
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
    commands.spawn(Building {
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
    q_building: &mut Query<(Entity, &mut Building)>,
    player_id: u16,
    building_id: u64,
) {
    if game.phase != GamePhase::Playing {
        return;
    }
    let mut order: Vec<(Entity, u64)> = q_building.iter().map(|(e, b)| (e, b.id)).collect();
    order.sort_by_key(|&(_, id)| id);
    let mut found: Option<(Entity, BuildingKind, u32)> = None;
    for (e, bid) in &order {
        if *bid != building_id {
            continue;
        }
        let Ok((_, b)) = q_building.get(*e) else {
            continue;
        };
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
        found = Some((*e, b.kind, b.tile_idx));
        break;
    }
    let Some((entity, kind, tile_idx)) = found else {
        bevy::log::warn!("apply_upgrade_structure: id {} not found", building_id);
        return;
    };
    if !structure_kind_enabled(kind) {
        return;
    }
    let snapshot = snapshot_buildings(q_building);
    let cost = structure_build_cost_gold(kind, player_id, &snapshot);
    let Some(player_mut) = game.player_mut(player_id) else {
        return;
    };
    if player_mut.gold < cost || !cost.is_finite() {
        return;
    }
    player_mut.gold = (player_mut.gold - cost).max(0.0);
    let Ok((_, mut b)) = q_building.get_mut(entity) else {
        return;
    };
    b.level = b.level.saturating_add(1);
    let new_level = b.level;
    game.events.push(GameEvent::StructureUpgraded {
        id: building_id,
        tile_idx,
        kind,
        level: new_level,
    });
}

