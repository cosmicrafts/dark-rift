use bevy::prelude::*;
use std::collections::BinaryHeap;
use crate::game::GameState;
use crate::protocol::*;
use crate::execution::{AttackExecution, PrioritizedTile};
use crate::pseudo_random::PseudoRandom;
use crate::map::TerrainType;
use crate::config;
pub fn merge_frontiers(
    mut a: BinaryHeap<PrioritizedTile>,
    mut b: BinaryHeap<PrioritizedTile>,
) -> BinaryHeap<PrioritizedTile> {
    let mut all: Vec<PrioritizedTile> = Vec::with_capacity(a.len() + b.len());
    all.extend(a.drain());
    all.extend(b.drain());

    // Primary key: spatial coordinate.
    // Secondary: keep the *best* tile (lowest priority, then lowest insert_seq).
    all.sort_by(|a, b| {
        (a.x, a.y)
            .cmp(&(b.x, b.y))
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.insert_seq.cmp(&b.insert_seq))
    });
    all.dedup_by_key(|t| (t.x, t.y));

    BinaryHeap::from(all)
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

    let exec_id = game.next_attack_id;
    game.next_attack_id = game.next_attack_id.wrapping_add(1).max(1);

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

pub fn apply_attack_intent(
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

    let exec_id = game.next_attack_id;
    game.next_attack_id = game.next_attack_id.wrapping_add(1).max(1);
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

