use std::cmp::Ordering;
use std::collections::BinaryHeap;
use bevy::prelude::*;
use crate::config;
use crate::game::{GameState, GamePhase, GameEvent};
use crate::map::TerrainType;
use crate::pseudo_random::PseudoRandom;

#[derive(Component)]
pub struct PlayerExecutionTag {
    pub player_id: u16,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PrioritizedTile {
    pub priority: f32, // Lower is processed first
    pub x: u32,
    pub y: u32,
}

impl Eq for PrioritizedTile {}

impl Ord for PrioritizedTile {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse order so lowest priority behaves as max in BinaryHeap
        other.priority.partial_cmp(&self.priority).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for PrioritizedTile {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Component)]
pub struct AttackExecution {
    pub owner_id: u16,
    pub target_owner: u16, // 0 = TerraNullius/neutral
    pub troops: f64,
    pub to_conquer: BinaryHeap<PrioritizedTile>,
    pub rng: PseudoRandom,
}

impl AttackExecution {
    pub fn calc_priority(&mut self, num_owned_by_me: u32, terrain: TerrainType, tick: u64) -> f32 {
        let mag = match terrain {
            TerrainType::Plains => 1.0,
            TerrainType::Highland => 1.5,
            TerrainType::Mountain => 2.0,
            _ => 1.0,
        };
        let r = self.rng.next_int(0, 7) as f32;
        (r + 10.0) * (1.0 - (num_owned_by_me as f32) * 0.5 + mag / 2.0) + (tick as f32)
    }
}

// System analogous to PlayerExecution.tick() in OpenFront
pub fn system_player_income(game_opt: Option<ResMut<GameState>>) {
    let Some(mut game) = game_opt else { return };
    if game.phase != GamePhase::Playing {
        return;
    }

    let GameState { map, players, .. } = &mut *game;

    for player in players.iter_mut().filter(|p| p.alive) {
        let tiles_owned = map.tiles_owned_by(player.id);
        if tiles_owned == 0 {
            player.alive = false; // Dead
            continue;
        }

        player.max_troops =
            config::MAX_TROOPS_BASE + (tiles_owned as f64).powf(0.6) * config::MAX_TROOPS_SCALE;

        let raw_income = config::TROOP_BASE_INCOME + (player.troops.powf(0.73) / 4.0);
        let ratio = 1.0 - (player.troops / player.max_troops).min(1.0);
        let income = raw_income * ratio;

        player.troops = (player.troops + income).min(player.max_troops);
    }
}

// System analogous to AttackExecution.tick() in OpenFront
pub fn system_attack_execution(
    mut commands: Commands,
    game_opt: Option<ResMut<GameState>>,
    mut query: Query<(Entity, &mut AttackExecution)>,
) {
    let Some(mut game) = game_opt else { return };
    if game.phase != GamePhase::Playing {
        return;
    }

    let tick_now = game.tick;
    // We briefly borrow map to do operations.
    // Notice that a tile ownership change mutates map, so we must be careful with borrows.
    
    for (entity, mut execution) in query.iter_mut() {
        let mut expanded_this_tick = 0u32;
        
        loop {
            if expanded_this_tick >= config::EXPAND_TILES_PER_TICK {
                break;
            }

            if execution.troops < config::EXPAND_COST {
                // Out of troops! Despawn
                commands.entity(entity).despawn();
                break;
            }

            if let Some(target_tile) = execution.to_conquer.pop() {
                // Check if target tile still belongs to the exact target
                if game.map.owner_id(target_tile.x, target_tile.y) != execution.target_owner {
                    continue; // Skip, no longer controlled by target
                }

                // Check adjacency: ensure it still borders the attacker
                if !game.map.is_adjacent_to_player(target_tile.x, target_tile.y, execution.owner_id) {
                    continue; // Skip, edge was severed
                }

                // VALID CONQUEST
                // Apply change!
                game.map.set_owner_id(target_tile.x, target_tile.y, execution.owner_id);
                execution.troops -= config::EXPAND_COST;
                expanded_this_tick += 1;

                // Send event
                game.events.push(GameEvent::TileCaptured {
                    x: target_tile.x,
                    y: target_tile.y,
                    new_owner: execution.owner_id,
                });

                // Subtract from player total if player exists
                if let Some(player) = game.players.iter_mut().find(|p| p.id == execution.owner_id) {
                    player.troops -= config::EXPAND_COST;
                }

                // Enqueue new neutral/enemy neighbors that touch our newly acquired tile
                for (nx, ny) in game.map.neighbors(target_tile.x, target_tile.y) {
                    // It must belong to target
                    if game.map.owner_id(nx, ny) != execution.target_owner {
                        continue;
                    }

                    // How many tiles bordering (nx, ny) are owned by the attacker?
                    let mut num_owned_by_me = 0;
                    for (nnx, nny) in game.map.neighbors(nx, ny) {
                        if game.map.owner_id(nnx, nny) == execution.owner_id {
                            num_owned_by_me += 1;
                        }
                    }

                    let terrain = game.map.terrain_type(nx, ny);
                    let prio = execution.calc_priority(num_owned_by_me, terrain, tick_now);

                    execution.to_conquer.push(PrioritizedTile {
                        priority: prio,
                        x: nx,
                        y: ny,
                    });
                }
            } else {
                // Priority queue is empty - finished conquering!
                commands.entity(entity).despawn();
                break;
            }
        }
    }
}
