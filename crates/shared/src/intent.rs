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
use crate::protocol::StampedIntent;

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
