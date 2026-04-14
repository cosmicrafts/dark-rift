use std::cmp::Ordering;
use std::collections::BinaryHeap;
use bevy::prelude::*;
use crate::config;
use crate::game::{GameState, GamePhase, GameEvent};
use crate::map::TerrainType;
use crate::pseudo_random::PseudoRandom;

/// Fraction of refunded troops lost when retreating from an attack on another player (OpenFront parity).
pub const RETREAT_PENALTY_VS_PLAYER: f64 = 0.25;

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
        match other.priority.partial_cmp(&self.priority).unwrap_or(Ordering::Equal) {
            Ordering::Equal => {
                // Secondary sort by coordinate to ensure 100% determinism 
                // between identically scored priority tiles.
                match other.y.cmp(&self.y) {
                    Ordering::Equal => other.x.cmp(&self.x),
                    ord => ord,
                }
            },
            ord => ord,
        }
    }
}

impl PartialOrd for PrioritizedTile {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Component)]
pub struct AttackExecution {
    pub id: u64,
    pub owner_id: u16,
    pub target_owner: u16,
    pub troops: f64,
    pub initial_troops: f64,
    pub to_conquer: BinaryHeap<PrioritizedTile>,
    pub rng: PseudoRandom,
    /// Player cancelled via HUD; next tick refunds remaining troops (with penalty vs players).
    pub retreating: bool,
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

        // Defensive mathematical bound: if player troops violently drain below zero during massive intent execution bursts, 
        // the calculation mathematically breaks and introduces desyncing NaN generations universally. We clamp to safely avoid this!
        let safe_troops = player.troops.max(0.0);
        let raw_income = config::TROOP_BASE_INCOME + (safe_troops.powf(0.73) / 4.0);
        let ratio = 1.0 - (safe_troops / player.max_troops).min(1.0);
        let income = raw_income * ratio;

        player.troops = (safe_troops + income).min(player.max_troops);
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
    // Collect all AttackExecution entities and sort them by their perfectly coherent monotonic ID 
    // to strictly enforce identical processing execution order across clients!
    let mut ordered: Vec<(Entity, u64)> = query.iter().map(|(ent, exec)| (ent, exec.id)).collect();
    ordered.sort_by_key(|&(_, id)| id);

    for (entity, _) in ordered {
        let Ok((_, mut execution)) = query.get_mut(entity) else { continue };

        if execution.retreating {
            let refund = execution.troops.max(0.0);
            if refund > 0.0 && refund.is_finite() {
                let factor = if execution.target_owner == 0 {
                    1.0
                } else {
                    1.0 - RETREAT_PENALTY_VS_PLAYER
                };
                let back = refund * factor;
                if let Some(player) = game
                    .players
                    .iter_mut()
                    .find(|p| p.id == execution.owner_id)
                {
                    player.troops = (player.troops + back).min(player.max_troops);
                }
            }
            commands.entity(entity).despawn();
            continue;
        }

        let mut max_tiles_f32 = if execution.target_owner == 0 {
            // Neutral expansion speed: grows purely with attack troops
            let base_speed = execution.troops as f32 / 50.0;
            base_speed.max(1.0).min(50.0) // At least 1, max 50
        } else {
            // PvP expansion speed: ratio based
            let defender_troops = game
                .player(execution.target_owner)
                .map(|p| p.troops)
                .unwrap_or(1.0)
                .max(1.0) as f32;
            let ratio = (execution.troops as f32) / defender_troops;
            let adjacent = execution.to_conquer.len() as f32;

            let power = (ratio * 2.0).clamp(0.01, 5.0); // Bounded relative power, lower bounds allow very slow advances
            (power * adjacent * 0.5).max(0.1).min(100.0) // At least 0.1 (10% chance per tick), max 100 tiles/tick
        };

        // Determine actual integer number of tiles to process this tick (Fractional determinism)
        let mut tiles_to_conquer = max_tiles_f32.floor() as u32;
        let p = max_tiles_f32.fract();
        if p > 0.0 && (execution.rng.next_int(0, 1000) as f32) / 1000.0 < p {
            tiles_to_conquer += 1;
        }

        let mut expanded_this_tick = 0u32;

        loop {
            if expanded_this_tick >= tiles_to_conquer {
                break;
            }

            if execution.troops < config::EXPAND_COST || execution.troops.is_nan() {
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
                execution.troops = execution.troops.max(0.0);
                expanded_this_tick += 1;

                // Send event
                game.events.push(GameEvent::TileCaptured {
                    x: target_tile.x,
                    y: target_tile.y,
                    new_owner: execution.owner_id,
                });

                // Troops were reserved on intent; only drain execution pool per tile (OpenFront).

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
                let refund = execution.troops.max(0.0);
                if refund > 0.0 && refund.is_finite() {
                    if let Some(player) = game
                        .players
                        .iter_mut()
                        .find(|p| p.id == execution.owner_id)
                    {
                        player.troops = (player.troops + refund).min(player.max_troops);
                    }
                }
                commands.entity(entity).despawn();
                break;
            }
        }
    }
}

pub fn system_game_tick(game_opt: Option<ResMut<GameState>>) {
    let Some(mut game) = game_opt else { return };
    game.tick();
}
