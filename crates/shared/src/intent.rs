//! Shared intent-to-execution converter.
//!
//! Given a StampedIntent from the server's Turn relay, spawns the appropriate
//! Bevy ECS entities on the client (AttackExecution with BFS queue seeded).
//!
//! This function is the single source of truth for intent → execution conversion,
//! used identically by every client to guarantee deterministic simulation.
//!
//! Mirrors OpenFront's Executor.createExecs() in core/GameRunner.ts.

use bevy::prelude::*;
use crate::game::GameState;
use crate::execution::{AttackExecution, PrioritizedTile};
use crate::map::TerrainType;
use crate::pseudo_random::PseudoRandom;
use crate::protocol::{StampedIntent, AttackIntent};
use crate::config;

/// Convert a StampedIntent into a seeded AttackExecution entity.
/// Called by every client when they receive a ServerTurnMessage.
///
/// Equivalent of OpenFront's Executor.createExecs() → AttackExecution constructor.
pub fn apply_stamped_intent(
    commands: &mut Commands,
    game: &GameState,
    stamped: &StampedIntent,
    intent_index: u32,
) {
    let player_id = stamped.player_id;
    let target_owner = stamped.intent.target_owner;

    let player = match game.players.iter().find(|p| p.id == player_id) {
        Some(p) => p,
        None => {
            warn!("apply_stamped_intent: player {} not found", player_id);
            return;
        }
    };

    if !player.alive {
        return;
    }

    let troops = stamped.intent.troops.unwrap_or(player.troops);

    // Seed the BFS priority queue from all border tiles adjacent to the target owner.
    // Deterministic: same seed, same tick, same result on every client.
    // Mirrors OpenFront AttackExecution constructor's initial queue seeding.
    let mut rng = PseudoRandom::new(game.seed.wrapping_add(game.tick));
    let mut to_conquer = std::collections::BinaryHeap::new();

    for (bx, by) in game.map.border_tiles_of(player_id) {
        for (nx, ny) in game.map.neighbors(bx, by) {
            if game.map.owner_id(nx, ny) != target_owner {
                continue;
            }

            // Count how many tiles bordering (nx,ny) we already own
            let num_owned_by_me = game.map.neighbors(nx, ny)
                .into_iter()
                .filter(|(nnx, nny)| game.map.owner_id(*nnx, *nny) == player_id)
                .count() as u32;

            let terrain = game.map.terrain_type(nx, ny);
            let mag = match terrain {
                TerrainType::Plains   => 1.0f32,
                TerrainType::Highland => 1.5,
                TerrainType::Mountain => 2.0,
                _                     => 1.0,
            };
            let r = rng.next_int(0, 7) as f32;
            let prio = (r + 10.0) * (1.0 - (num_owned_by_me as f32) * 0.5 + mag / 2.0)
                + game.tick as f32;

            to_conquer.push(PrioritizedTile { priority: prio, x: nx, y: ny });
        }
    }

    if to_conquer.is_empty() {
        return; // No valid border with target — intent is a no-op
    }

    let exec_id = ((game.tick as u64) << 32) | (intent_index as u64);

    commands.spawn(AttackExecution {
        id: exec_id,
        owner_id: player_id,
        target_owner,
        troops,
        to_conquer,
        rng,
    });
}

/// Simulation-driven Bot AI behavior. Runs locally on every client deterministically.
pub fn system_bot_think(
    mut commands: Commands,
    game: Res<GameState>,
) {
    if game.phase != crate::game::GamePhase::Playing {
        return;
    }

    let bot_ids: Vec<u16> = game.players.iter().filter(|p| !p.is_human() && p.alive).map(|p| p.id).collect();
    if bot_ids.is_empty() { return; }

    let mut rng = PseudoRandom::new(game.seed.wrapping_add(game.tick));

    for bot_id in bot_ids {
        let Some(player) = game.player(bot_id) else { continue; };

        // Organic Pacing:
        // Attack rate ~30 ticks (3 seconds). We use deterministic modulo based on bot_id.
        let attack_interval = 30; 
        if game.tick % attack_interval != (bot_id as u64 % attack_interval) {
            continue;
        }

        // Trigger Phase: Save troops until we reach 60% capacity
        let trigger_limit = player.max_troops * 0.6;
        if player.troops < trigger_limit { continue; }

        // Burst Phase: Send what we have, minus a 30% reserve
        let reserve = player.max_troops * 0.3;
        let p_send = (player.troops - reserve).max(0.0);
        if p_send < config::EXPAND_COST { continue; }

        let borders = game.map.border_tiles_of(bot_id);
        if borders.is_empty() { continue; }

        let r_idx = rng.next_int(0, borders.len() as i32) as usize;
        let (bx, by) = borders[r_idx];

        let neighbors = game.map.neighbors(bx, by);
        let mut targets = Vec::new();
        for (nx, ny) in neighbors {
            let owner = game.map.owner_id(nx, ny);
            if owner != bot_id {
                targets.push(owner);
                if owner == 0 {
                    // Double priority for expanding into neutral territory
                    targets.push(0);
                    targets.push(0);
                }
            }
        }

        if targets.is_empty() { continue; }
        let target_owner = targets[rng.next_int(0, targets.len() as i32) as usize];
        
        let intent = StampedIntent {
            player_id: bot_id,
            intent: AttackIntent { target_owner, troops: Some(p_send) }
        };
        
        apply_stamped_intent(&mut commands, &game, &intent, bot_id as u32);
    }
}
