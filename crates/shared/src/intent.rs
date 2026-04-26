//! Intent → AttackExecution: merge, mutual cancel, upfront troop reservation (OpenFront parity).

use std::collections::{BinaryHeap, HashMap};

use bevy::prelude::*;

use crate::building::{
    existing_structure_positions_with_width,
    resolve_structure_spawn_tile, structure_build_cost_gold, Building,
};
use crate::config;
use crate::execution::{AttackExecution, PrioritizedTile};
use crate::game::{BuildingKind, GameEvent, GamePhase, GameState};
use crate::map::TerrainType;
use crate::pathfinding::WaterPathfinderScratch;
use crate::pseudo_random::PseudoRandom;
use crate::protocol::{AttackIntent, GameplayIntent, StampedIntent};
use crate::warp_fleet::{resolve_fleet_route, FleetRoute, WarpFleet};
use crate::water_components::WaterComponents;

/// Merge two frontiers: same cell keeps the tile with lower `priority` (better for expansion order).
pub fn merge_frontiers(
    mut a: BinaryHeap<PrioritizedTile>,
    mut b: BinaryHeap<PrioritizedTile>,
) -> BinaryHeap<PrioritizedTile> {
    let mut best: HashMap<(u32, u32), PrioritizedTile> = HashMap::new();
    for t in a.drain().chain(b.drain()) {
        let k = (t.x, t.y);
        best.entry(k)
            .and_modify(|cur| {
                if t.priority < cur.priority {
                    *cur = t;
                }
            })
            .or_insert(t);
    }
    let mut v: Vec<PrioritizedTile> = best.into_values().collect();
    v.sort_by(|x, y| x.cmp(y));
    BinaryHeap::from(v)
}

pub fn build_attack_frontier(
    game: &GameState,
    player_id: u16,
    target_owner: u16,
    border_tiles: &[(u32, u32)],
    rng: &mut PseudoRandom,
    insert_seq_counter: &mut u32,
) -> BinaryHeap<PrioritizedTile> {
    let mut to_conquer = BinaryHeap::new();
    for (bx, by) in border_tiles.iter().copied() {
        game.map.for_each_neighbor(bx, by, |nx, ny| {
            if game.map.owner_id(nx, ny) != target_owner {
                return;
            }
            let mut num_owned_by_me = 0u32;
            game.map.for_each_neighbor(nx, ny, |nnx, nny| {
                if game.map.owner_id(nnx, nny) == player_id {
                    num_owned_by_me += 1;
                }
            });

            let terrain = game.map.terrain_type(nx, ny);
            let mag_x2 = match terrain {
                TerrainType::Land => 2,
                TerrainType::Highland => 3,
                TerrainType::Mountain => 4,
                TerrainType::Water | TerrainType::Lake => 3, // Fallback, won't happen normally
            };
            let r = rng.next_int(0, 7) as i64;
            // Formula scaled by 4 to maintain OpenFront quartiles in integer space
            let prio = (r + 10) * (4 - (num_owned_by_me as i64 * 2) + mag_x2)
                + (game.tick as i64 * 4);
            
            let seq = *insert_seq_counter;
            *insert_seq_counter = insert_seq_counter.wrapping_add(1);

            to_conquer.push(PrioritizedTile {
                priority: prio,
                insert_seq: seq,
                x: nx,
                y: ny,
            });
        });
    }
    to_conquer
}

/// Spawn or merge an `AttackExecution` after a fleet lands (troops already reserved at launch).
pub fn spawn_or_merge_attack_for_fleet_arrival(
    commands: &mut Commands,
    game: &mut GameState,
    q_exec: &mut Query<(Entity, &mut AttackExecution)>,
    owner_id: u16,
    target_owner: u16,
    troops: f64,
    fleet_id: u64,
) {
    if troops < config::EXPAND_COST || !troops.is_finite() {
        if let Some(p) = game.player_mut(owner_id) {
            p.troops = (p.troops + troops.max(0.0)).min(p.max_troops);
        }
        return;
    }

    let exec_seed = game
        .seed
        .wrapping_add(game.tick)
        .wrapping_add(fleet_id)
        .wrapping_add(0xB04F_0000);
    let mut rng = PseudoRandom::new(exec_seed);
    let mut initial_seq = 0u32;
    let Some(player) = game.player(owner_id) else {
        return;
    };
    let border_tiles = player.border_coords(game.map.width);
    let fresh = build_attack_frontier(
        game,
        owner_id,
        target_owner,
        &border_tiles,
        &mut rng,
        &mut initial_seq,
    );

    if fresh.is_empty() {
        if let Some(p) = game.player_mut(owner_id) {
            p.troops = (p.troops + troops).min(p.max_troops);
        }
        return;
    }

    let mut merge_candidates: Vec<(Entity, u64)> = q_exec
        .iter()
        .filter(|(_, ex)| {
            ex.owner_id == owner_id && ex.target_owner == target_owner && !ex.retreating
        })
        .map(|(e, ex)| (e, ex.id))
        .collect();
    merge_candidates.sort_by_key(|(_, id)| *id);

    if let Some((e_merge, _)) = merge_candidates.first().copied() {
        let Ok((_, mut ex)) = q_exec.get_mut(e_merge) else {
            return;
        };
        ex.troops += troops;
        ex.initial_troops += troops;
        let merged = merge_frontiers(std::mem::take(&mut ex.to_conquer), fresh);
        ex.to_conquer = merged;
        return;
    }

    let exec_id = game
        .tick
        .wrapping_shl(32)
        .wrapping_add(fleet_id)
        .wrapping_add(0xF1EE0000);

    commands.spawn(AttackExecution {
        id: exec_id,
        owner_id,
        target_owner,
        troops,
        initial_troops: troops,
        to_conquer: fresh,
        insert_seq_counter: initial_seq,
        rng,
        retreating: false,
    });
}

fn snapshot_buildings(q: &mut Query<(Entity, &mut Building)>) -> Vec<Building> {
    q.iter().map(|(_, b)| *b).collect()
}

fn apply_build_structure_intent(
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

fn apply_upgrade_structure_intent(
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

/// Apply one stamped intent: mutates `GameState`, `AttackExecution` query, and queues spawns/despawns.
pub fn apply_stamped_intent(
    commands: &mut Commands,
    game: &mut GameState,
    q_exec: &mut Query<(Entity, &mut AttackExecution)>,
    q_fleet: &mut Query<(Entity, &mut WarpFleet)>,
    q_building: &mut Query<(Entity, &mut Building)>,
    stamped: &StampedIntent,
    intent_index: u32,
    path_scratch: &mut WaterPathfinderScratch,
    water_components: &WaterComponents,
) {
    match &stamped.intent {
        GameplayIntent::RecallFleet { fleet_id } => {
            let pid = stamped.player_id;
            let mut pairs: Vec<(Entity, u64)> = q_fleet.iter().map(|(e, w)| (e, w.id)).collect();
            pairs.sort_by_key(|&(_, id)| id);
            for (entity, fid) in pairs {
                if fid != *fleet_id {
                    continue;
                }
                let Ok((_, mut wf)) = q_fleet.get_mut(entity) else {
                    continue;
                };
                if wf.owner_id != pid {
                    continue;
                }
                wf.retreating = true;
                wf.retreat_dst = None;
                wf.path.clear();
                wf.path_cursor = 0;
                break;
            }
        }
        GameplayIntent::LaunchFleet {
            target_owner,
            target_tile,
            troops,
        } => {
            let buildings: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();
            apply_launch_fleet_intent(
                commands,
                game,
                path_scratch,
                water_components,
                q_fleet,
                stamped.player_id,
                *target_owner,
                *target_tile,
                *troops,
                &buildings,
            );
        }
        GameplayIntent::CancelAttack { attack_id } => {
            let pid = stamped.player_id;
            for (_, mut exec) in q_exec.iter_mut() {
                if exec.id == *attack_id && exec.owner_id == pid {
                    exec.retreating = true;
                    return;
                }
            }
            bevy::log::warn!(
                "apply_stamped_intent: cancel attack_id={} for player {} — not found or not owner",
                attack_id, pid
            );
        }
        GameplayIntent::Attack(attack) => {
            apply_attack_intent(commands, game, q_exec, stamped.player_id, attack, intent_index);
        }
        GameplayIntent::BuildStructure { kind, target_tile } => {
            apply_build_structure_intent(
                commands,
                game,
                q_building,
                stamped.player_id,
                *kind,
                *target_tile,
            );
        }
        GameplayIntent::UpgradeStructure { building_id } => {
            apply_upgrade_structure_intent(game, q_building, stamped.player_id, *building_id);
        }
    }
}

fn apply_launch_fleet_intent(
    commands: &mut Commands,
    game: &mut GameState,
    path_scratch: &mut WaterPathfinderScratch,
    water_components: &WaterComponents,
    _q_fleet: &mut Query<(Entity, &mut WarpFleet)>,
    player_id: u16,
    target_owner: u16,
    target_tile: u32,
    troops: f64,
    _buildings: &[Building],
) {
    let Some(player) = game.player(player_id) else {
        bevy::log::warn!("apply_launch_fleet_intent: player {} not found", player_id);
        return;
    };
    if !player.alive {
        return;
    }
    if target_owner == player_id {
        return;
    }

    let map_area = game.map.width.saturating_mul(game.map.height);
    if map_area == 0 || target_tile >= map_area {
        bevy::log::warn!(
            "apply_launch_fleet_intent: out-of-range target_tile={} (player {})",
            target_tile, player_id
        );
        return;
    }

    let border_tiles: &[u32] = match game.player(player_id) {
        Some(p) => p.border_tiles.as_slice(),
        None => return,
    };

    let target_border: Option<&[u32]> = if target_owner != 0 {
        match game.player(target_owner) {
            Some(p) => Some(p.border_tiles.as_slice()),
            None => {
                bevy::log::warn!(
                    "apply_launch_fleet_intent: target_owner={} not found (player {})",
                    target_owner, player_id
                );
                return;
            }
        }
    } else {
        None
    };

    let route = match resolve_fleet_route(
        &game.map,
        water_components,
        path_scratch,
        player_id,
        target_owner,
        target_tile,
        border_tiles,
        target_border,
        true,
    ) {
        Ok(r) => r,
        Err(e) => {
            bevy::log::warn!(
                "apply_launch_fleet_intent: {} (player {}, target_owner={}, tile={})",
                e,
                player_id,
                target_owner,
                target_tile
            );
            return;
        }
    };

    let pool_cap = player.troops;
    let launch = troops.max(0.0).min(pool_cap);
    if launch < config::EXPAND_COST {
        return;
    }

    let FleetRoute {
        src_tile: src,
        landing_tile: landing,
        path,
    } = route;

    let Some(p) = game.player_mut(player_id) else {
        return;
    };
    p.troops -= launch;
    p.troops = p.troops.max(0.0);

    let fid = game.next_fleet_id;
    game.next_fleet_id = game.next_fleet_id.wrapping_add(1).max(1);

    commands.spawn(WarpFleet::new(
        fid,
        player_id,
        target_owner,
        launch,
        src,
        landing,
        path,
    ));
}

fn apply_attack_intent(
    commands: &mut Commands,
    game: &mut GameState,
    q_exec: &mut Query<(Entity, &mut AttackExecution)>,
    player_id: u16,
    attack: &AttackIntent,
    intent_index: u32,
) {
    let target_owner = attack.target_owner;

    let Some(player) = game.player(player_id) else {
        bevy::log::warn!("apply_attack_intent: player {} not found", player_id);
        return;
    };

    if !player.alive {
        return;
    }

    let pool_cap = player.troops;
    let requested = attack
        .troops
        .unwrap_or(pool_cap)
        .max(0.0)
        .min(pool_cap);

    if requested < config::EXPAND_COST {
        return;
    }

    let Some(p) = game.player_mut(player_id) else {
        return;
    };
    p.troops -= requested;
    p.troops = p.troops.max(0.0);

    let mut remaining = requested;

    // Mutual annihilation with enemy attacks that target us (deterministic by enemy exec id).
    let mut enemy_hits: Vec<(Entity, u64)> = q_exec
        .iter()
        .filter(|(_, ex)| {
            ex.owner_id == target_owner && ex.target_owner == player_id && !ex.retreating
        })
        .map(|(e, ex)| (e, ex.id))
        .collect();
    enemy_hits.sort_by_key(|(_, id)| *id);

    for (e_enemy, _) in &enemy_hits {
        if remaining < config::EXPAND_COST {
            break;
        }
        let Ok((_, mut enemy_ex)) = q_exec.get_mut(*e_enemy) else {
            continue;
        };
        if enemy_ex.troops < config::ATTACK_COST {
            continue;
        }
        let clash = enemy_ex.troops.min(remaining);
        enemy_ex.troops -= clash;
        enemy_ex.troops = enemy_ex.troops.max(0.0);
        remaining -= clash;
    }

    for (e_enemy, _) in enemy_hits {
        let despawn = q_exec
            .get(e_enemy)
            .map(|(_, ex)| ex.troops < config::EXPAND_COST)
            .unwrap_or(false);
        if despawn {
            commands.entity(e_enemy).despawn();
        }
    }

    if remaining < config::EXPAND_COST {
        if let Some(p) = game.player_mut(player_id) {
            p.troops = (p.troops + remaining).min(p.max_troops);
        }
        return;
    }

    let mut rng = PseudoRandom::new(
        game
            .seed
            .wrapping_add(game.tick)
            .wrapping_add(intent_index as u64),
    );
    let mut initial_seq = 0;
    let border_tiles = game.player(player_id).unwrap().border_coords(game.map.width);
    let fresh = build_attack_frontier(
        game,
        player_id,
        target_owner,
        &border_tiles,
        &mut rng,
        &mut initial_seq,
    );

    if fresh.is_empty() {
        if let Some(p) = game.player_mut(player_id) {
            p.troops = (p.troops + remaining).min(p.max_troops);
        }
        return;
    }

    // Merge into existing outgoing attack to same target (lowest exec id wins).
    let mut merge_candidates: Vec<(Entity, u64)> = q_exec
        .iter()
        .filter(|(_, ex)| {
            ex.owner_id == player_id && ex.target_owner == target_owner && !ex.retreating
        })
        .map(|(e, ex)| (e, ex.id))
        .collect();
    merge_candidates.sort_by_key(|(_, id)| *id);

    if let Some((e_merge, _)) = merge_candidates.first().copied() {
        let Ok((_, mut ex)) = q_exec.get_mut(e_merge) else {
            return;
        };
        ex.troops += remaining;
        ex.initial_troops += remaining;
        let merged = merge_frontiers(std::mem::take(&mut ex.to_conquer), fresh);
        ex.to_conquer = merged;
        return;
    }

    let exec_id = ((game.tick as u64) << 32) | (intent_index as u64);
    commands.spawn(AttackExecution {
        id: exec_id,
        owner_id: player_id,
        target_owner,
        troops: remaining,
        initial_troops: remaining,
        to_conquer: fresh,
        insert_seq_counter: initial_seq,
        rng,
        retreating: false,
    });
}

pub fn system_bot_think(
    mut commands: Commands,
    mut game: ResMut<GameState>,
    mut q_exec: Query<(Entity, &mut AttackExecution)>,
    mut q_fleet: Query<(Entity, &mut WarpFleet)>,
    mut q_building: Query<(Entity, &mut Building)>,
    mut path_scratch: ResMut<crate::pathfinding::WaterPathfinderScratch>,
    water_components: Res<WaterComponents>,
    config_opt: Option<Res<crate::game_config::GameConfig>>,
) {
    if game.phase != crate::game::GamePhase::Playing {
        return;
    }

    let config = config_opt.map(|c| c.clone()).unwrap_or_default();
    let expand_cost = config.expand_cost;

    let (attack_interval_base, trigger_ratio, reserve_ratio) = match config.bot_difficulty {
        crate::game_config::BotDifficulty::BrainDead => (500, 0.9, 0.8),
        crate::game_config::BotDifficulty::Vanilla => (250, 0.6, 0.3),
        crate::game_config::BotDifficulty::Terminator => (125, 0.4, 0.1),
    };

    let tick = game.tick;
    let seed = game.seed;

    // Amortized collection of active bots
    let mut active_bots = Vec::new();

    for player in game.players.iter() {
        if player.is_human() || !player.alive {
            continue;
        }
        
        let bot_id = player.id;
        let mut bot_setup_rng = PseudoRandom::new(seed.wrapping_add(bot_id as u64).wrapping_add(1337));
        let my_interval = bot_setup_rng.next_int(attack_interval_base as i32, (attack_interval_base as i32 * 2).max(1)) as u64;
        let my_offset = bot_setup_rng.next_int(0, my_interval as i32) as u64;

        if tick % my_interval == my_offset {
            active_bots.push(bot_id);
        }
    }

    if active_bots.is_empty() {
        return;
    }

    let mut rng = PseudoRandom::new(seed.wrapping_add(tick));

    for bot_id in active_bots {
        let Some(player) = game.player(bot_id) else {
            continue;
        };

        let border_tiles = player.border_coords(game.map.width);
        if border_tiles.is_empty() { continue; }

        let mut has_neutral = false;
        let mut player_targets = Vec::new();

        // Terra Nullius Pacifism: Scan entire perimeter for ANY empty land
        for &(bx, by) in &border_tiles {
            game.map.for_each_neighbor(bx, by, |nx, ny| {
                let owner = game.map.owner_id(nx, ny);
                if owner != bot_id {
                    if owner == 0 {
                        has_neutral = true;
                    } else {
                        player_targets.push(owner);
                    }
                }
            });
        }

        // OpenFront AI Algorithm Port:
        // 1. If Neutral, we expand organically. No trigger limit, tiny reserve.
        // 2. If Player, we stockpile (dumb/slow AI). Strict trigger limit, larger reserve.
        let (target_owner, is_neutral) = if has_neutral {
            (0, true)
        } else {
            if player_targets.is_empty() {
                continue;
            }
            (player_targets[rng.next_int(0, player_targets.len() as i32) as usize], false)
        };

        if !is_neutral {
            // Player Attack: Must save up troops first to burst
            let trigger_limit = player.max_troops * trigger_ratio;
            if player.troops < trigger_limit {
                continue;
            }
        }

        // Neutral attacks use expand_ratio (e.g. 0.15) so they trickle constantly. Player attacks use reserve_ratio.
        let active_reserve_ratio = if is_neutral { 0.15 } else { reserve_ratio };
        let reserve = player.max_troops * active_reserve_ratio;
        let p_send = (player.troops - reserve).max(0.0);
        
        if p_send < expand_cost as f64 {
            continue;
        }

        let stamped = StampedIntent {
            player_id: bot_id,
            intent: GameplayIntent::Attack(AttackIntent {
                target_owner,
                troops: Some(p_send),
            }),
        };

        apply_stamped_intent(
            &mut commands,
            &mut game,
            &mut q_exec,
            &mut q_fleet,
            &mut q_building,
            &stamped,
            bot_id as u32,
            &mut *path_scratch,
            &water_components,
        );
    }
}

#[cfg(test)]
mod building_intent_tests {
    use super::*;
    use bevy::ecs::system::SystemState;
    use crate::building::Building;
    use crate::game::{BuildingKind, GamePhase, GameState};
    use crate::map::{GameMap, MapTile};
    use crate::player::Player;

    fn tiny_owned_game_state() -> GameState {
        let mut m = GameMap::new(5, 1);
        let owner = 1u16;
        for x in 0..5 {
            m.set_owner_id(x, 0, owner);
            let ri = m.ref_id(x, 0);
            m.terrain[ri] = MapTile::from_byte(0b1000_0000);
        }
        let r0 = m.ref_id(0, 0);
        m.terrain[r0] = MapTile::from_byte(0b1100_0000);
        let r4 = m.ref_id(4, 0);
        m.terrain[r4] = MapTile::from_byte(0b1100_0000);

        let mut game = GameState::new(99, 5, 1);
        game.phase = GamePhase::Playing;
        game.map = m;
        game.players.push(Player::new_human(owner, "t".into(), [1.0, 0.0, 0.0]));
        game.player_lookup.resize(owner as usize + 1, None);
        game.player_lookup[owner as usize] = Some(0);
        if let Some(p) = game.player_mut(owner) {
            p.troops = 50_000.0;
            p.gold = 50_000.0;
        }
        game
    }

    #[test]
    fn build_structure_intent_spawns_building() {
        let mut world = World::new();
        world.insert_resource(tiny_owned_game_state());

        let mut state = SystemState::<(
            Commands,
            ResMut<GameState>,
            Query<(Entity, &mut Building)>,
        )>::new(&mut world);
        {
            let (mut commands, mut game, mut q_building) = state.get_mut(&mut world);
            apply_build_structure_intent(
                &mut commands,
                &mut game,
                &mut q_building,
                1,
                BuildingKind::City,
                2,
            );
        }
        state.apply(&mut world);
        world.flush();

        let mut q = world.query::<&Building>();
        let count = q.iter(&world).count();
        assert_eq!(count, 1);
        let b = q.iter(&world).next().unwrap();
        assert_eq!(b.owner_id, 1);
        assert_eq!(b.kind, BuildingKind::City);
        assert!(b.under_construction);
        assert!(b.id >= 1);

        let game = world.resource::<GameState>();
        let p = game.player(1).unwrap();
        assert_eq!(p.troops, 50_000.0);
        assert!(p.gold < 50_000.0);
    }

    fn wide_owned_line() -> GameState {
        let w = 48u32;
        let mut m = GameMap::new(w, 1);
        let owner = 1u16;
        for x in 0..w {
            m.set_owner_id(x, 0, owner);
            let ri = m.ref_id(x, 0);
            m.terrain[ri] = MapTile::from_byte(0b1000_0000);
        }
        let r0 = m.ref_id(0, 0);
        m.terrain[r0] = MapTile::from_byte(0b1100_0000);
        let re = m.ref_id(w - 1, 0);
        m.terrain[re] = MapTile::from_byte(0b1100_0000);

        let mut game = GameState::new(101, w, 1);
        game.phase = GamePhase::Playing;
        game.map = m;
        game.players.push(Player::new_human(owner, "w".into(), [1.0, 0.0, 0.0]));
        game.player_lookup.resize(owner as usize + 1, None);
        game.player_lookup[owner as usize] = Some(0);
        if let Some(p) = game.player_mut(owner) {
            p.troops = 50_000.0;
            p.gold = 50_000.0;
        }
        game
    }

    #[test]
    fn two_builds_get_monotonic_ids() {
        let mut world = World::new();
        world.insert_resource(wide_owned_line());

        let mut state = SystemState::<(
            Commands,
            ResMut<GameState>,
            Query<(Entity, &mut Building)>,
        )>::new(&mut world);
        {
            let (mut commands, mut game, mut q_building) = state.get_mut(&mut world);
            apply_build_structure_intent(
                &mut commands,
                &mut game,
                &mut q_building,
                1,
                BuildingKind::City,
                5,
            );
            apply_build_structure_intent(
                &mut commands,
                &mut game,
                &mut q_building,
                1,
                BuildingKind::Factory,
                40,
            );
        }
        state.apply(&mut world);
        world.flush();

        let mut q = world.query::<&Building>();
        let mut ids: Vec<u64> = q.iter(&world).map(|b| b.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn upgrade_while_under_construction_is_ignored() {
        let mut world = World::new();
        world.insert_resource(tiny_owned_game_state());
        world.spawn(Building {
            id: 9,
            owner_id: 1,
            tile_idx: 2,
            kind: BuildingKind::City,
            level: 1,
            under_construction: true,
            ticks_until_complete: 5,
        });

        let mut state = SystemState::<(ResMut<GameState>, Query<(Entity, &mut Building)>)>::new(&mut world);
        {
            let (mut game, mut q_building) = state.get_mut(&mut world);
            apply_upgrade_structure_intent(&mut game, &mut q_building, 1, 9);
        }
        state.apply(&mut world);
        world.flush();

        let b = world
            .query::<&Building>()
            .iter(&world)
            .find(|b| b.id == 9)
            .unwrap();
        assert_eq!(b.level, 1);
        assert!(b.under_construction);
    }
}
