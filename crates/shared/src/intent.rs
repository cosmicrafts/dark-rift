//! Intent → AttackExecution: merge, mutual cancel, upfront troop reservation (OpenFront parity).

use std::collections::{BinaryHeap, HashMap};

use bevy::prelude::*;

use crate::config;
use crate::execution::{AttackExecution, PrioritizedTile};
use crate::game::GameState;
use crate::map::TerrainType;
use crate::pseudo_random::PseudoRandom;
use crate::protocol::{AttackIntent, GameplayIntent, StampedIntent};

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

fn build_attack_frontier(
    game: &GameState,
    player_id: u16,
    target_owner: u16,
    rng: &mut PseudoRandom,
    insert_seq_counter: &mut u32,
) -> BinaryHeap<PrioritizedTile> {
    let mut to_conquer = BinaryHeap::new();
    for (bx, by) in game.map.border_tiles_of(player_id) {
        for (nx, ny) in game.map.neighbors(bx, by) {
            if game.map.owner_id(nx, ny) != target_owner {
                continue;
            }
            let num_owned_by_me = game
                .map
                .neighbors(nx, ny)
                .into_iter()
                .filter(|(nnx, nny)| game.map.owner_id(*nnx, *nny) == player_id)
                .count() as u32;

            let terrain = game.map.terrain_type(nx, ny);
            let mag_x2 = match terrain {
                TerrainType::Plains => 2,
                TerrainType::Highland => 3,
                TerrainType::Mountain => 4,
                _ => 2,
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
        }
    }
    to_conquer
}

/// Apply one stamped intent: mutates `GameState`, `AttackExecution` query, and queues spawns/despawns.
pub fn apply_stamped_intent(
    commands: &mut Commands,
    game: &mut GameState,
    q_exec: &mut Query<(Entity, &mut AttackExecution)>,
    stamped: &StampedIntent,
    intent_index: u32,
) {
    match &stamped.intent {
        GameplayIntent::CancelAttack { attack_id } => {
            let pid = stamped.player_id;
            for (_, mut exec) in q_exec.iter_mut() {
                if exec.id == *attack_id && exec.owner_id == pid {
                    exec.retreating = true;
                    return;
                }
            }
            warn!(
                "apply_stamped_intent: cancel attack_id={} for player {} — not found or not owner",
                attack_id, pid
            );
        }
        GameplayIntent::Attack(attack) => {
            apply_attack_intent(commands, game, q_exec, stamped.player_id, attack, intent_index);
        }
    }
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

    let Some(player) = game.players.iter().find(|p| p.id == player_id) else {
        warn!("apply_attack_intent: player {} not found", player_id);
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

    let Some(p) = game.players.iter_mut().find(|p| p.id == player_id) else {
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
        if let Some(p) = game.players.iter_mut().find(|p| p.id == player_id) {
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
    let fresh = build_attack_frontier(game, player_id, target_owner, &mut rng, &mut initial_seq);

    if fresh.is_empty() {
        if let Some(p) = game.players.iter_mut().find(|p| p.id == player_id) {
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
    config_opt: Option<Res<crate::game_config::GameConfig>>,
) {
    if game.phase != crate::game::GamePhase::Playing {
        return;
    }

    let bot_ids: Vec<u16> = game
        .players
        .iter()
        .filter(|p| !p.is_human() && p.alive)
        .map(|p| p.id)
        .collect();
    if bot_ids.is_empty() {
        return;
    }

    let mut rng = PseudoRandom::new(game.seed.wrapping_add(game.tick));

    let config = config_opt.map(|c| c.clone()).unwrap_or_default();
    let expand_cost = config.expand_cost;

    // Adjusted for 60 ticks per second (OpenFront was 4 ticks per second. Multiply by 15x)
    let (attack_interval_base, trigger_ratio, reserve_ratio) = match config.bot_difficulty {
        crate::game_config::BotDifficulty::BrainDead => (500, 0.9, 0.8),
        crate::game_config::BotDifficulty::Vanilla => (250, 0.6, 0.3),
        crate::game_config::BotDifficulty::Terminator => (125, 0.4, 0.1),
    };

    for bot_id in bot_ids {
        let Some(player) = game.player(bot_id) else {
            continue;
        };

        // Use a unique setup RNG for each bot that does not pollute the global PRNG
        let mut bot_setup_rng = PseudoRandom::new(game.seed.wrapping_add(bot_id as u64).wrapping_add(1337));
        // Bot interval varies between base and 2x base (e.g., Vanilla: 30 to 60)
        let my_interval = bot_setup_rng.next_int(attack_interval_base as i32, (attack_interval_base as i32 * 2).max(1)) as u64;
        let my_offset = bot_setup_rng.next_int(0, my_interval as i32) as u64;

        if game.tick % my_interval != my_offset {
            continue;
        }

        let borders = game.map.border_tiles_of(bot_id);
        if borders.is_empty() {
            continue;
        }

        let mut has_neutral = false;
        let mut player_targets = Vec::new();

        // Terra Nullius Pacifism: Scan entire perimeter for ANY empty land
        for &(bx, by) in &borders {
            for (nx, ny) in game.map.neighbors(bx, by) {
                let owner = game.map.owner_id(nx, ny);
                if owner != bot_id {
                    if owner == 0 {
                        has_neutral = true;
                    } else {
                        player_targets.push(owner);
                    }
                }
            }
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
            &stamped,
            bot_id as u32,
        );
    }
}
