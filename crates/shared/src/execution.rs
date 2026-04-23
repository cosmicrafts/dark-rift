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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrioritizedTile {
    pub priority: i64, // Lower is processed first
    pub insert_seq: u32, // Deterministic BFS tie-breaker
    pub x: u32,
    pub y: u32,
}

impl Ord for PrioritizedTile {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse order so lowest priority behaves as max in BinaryHeap
        match other.priority.cmp(&self.priority) {
            Ordering::Equal => {
                // Return exact Insertion Time inverse!
                // Older items (smallest insert_seq) pop FIRST, guaranteeing deterministic BFS organicity!
                match other.insert_seq.cmp(&self.insert_seq) {
                    Ordering::Equal => {
                        // Flawless Secondary Tie-Breaker: Coordinate space is immutable and unique per tile
                        match other.y.cmp(&self.y) {
                            Ordering::Equal => other.x.cmp(&self.x),
                            ord => ord,
                        }
                    },
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
    pub insert_seq_counter: u32,
    pub rng: PseudoRandom,
    /// Player cancelled via HUD; next tick refunds remaining troops (with penalty vs players).
    pub retreating: bool,
}

impl AttackExecution {
    pub fn calc_priority(&mut self, num_owned_by_me: u32, terrain: TerrainType, tick: u64) -> i64 {
        let mag_x2 = match terrain {
            TerrainType::Land => 2,
            TerrainType::Highland => 3,
            TerrainType::Mountain => 4,
            TerrainType::Water | TerrainType::Lake => 3, // Won't happen normally, but just in case
        };
        let r = self.rng.next_int(0, 7) as i64;
        // Formula scaled by 4 to maintain OpenFront quartiles in integer space
        (r + 10) * (4 - (num_owned_by_me as i64 * 2) + mag_x2) + (tick as i64 * 4)
    }
}

// System analogous to PlayerExecution.tick() in OpenFront
pub fn system_player_income(game_opt: Option<ResMut<GameState>>) {
    let Some(mut game) = game_opt else { return };
    if game.phase != GamePhase::Playing {
        return;
    }

    let GameState { map: _, players, .. } = &mut *game;

    for player in players.iter_mut().filter(|p| p.alive) {
        let tiles_owned = player.tile_count;
        if tiles_owned == 0 {
            player.alive = false; // Dead
            continue;
        }

        player.max_troops =
            config::MAX_TROOPS_BASE + (tiles_owned as f64).powf(0.6) * config::MAX_TROOPS_SCALE;

        // Defensive mathematical bound: if player troops violently drain below zero during massive intent execution bursts, 
        // the calculation mathematically breaks and introduces desyncing NaN generations universally. We clamp to safely avoid this!
        let safe_troops = player.troops.max(0.0);
        
        // Use strictly deterministic IEEE-754 sqrt instead of libm powf!
        // tiles_owned^0.625 = tiles_owned^(1/2) * tiles_owned^(1/8)
        let t_f64 = tiles_owned as f64;
        let t_half = t_f64.sqrt();
        let t_quarter = t_half.sqrt();
        let t_eighth = t_quarter.sqrt();
        let max_troops_bonus = t_half * t_eighth;
        
        player.max_troops = config::MAX_TROOPS_BASE + max_troops_bonus * config::MAX_TROOPS_SCALE;

        // safe_troops^0.75 = safe_troops^(1/2) * safe_troops^(1/4)
        let s_half = safe_troops.sqrt();
        let s_quarter = s_half.sqrt();
        let s_75 = s_half * s_quarter;
        
        let raw_income = config::TROOP_BASE_INCOME + (s_75 / 4.6);
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
    mut scratch_ordered: Local<Vec<(Entity, u64)>>,
) {
    let Some(mut game) = game_opt else { return };
    if game.phase != GamePhase::Playing {
        return;
    }

    let tick_now = game.tick;
    // Collect all AttackExecution entities and sort them by their perfectly coherent monotonic ID 
    // to strictly enforce identical processing execution order across clients!
    scratch_ordered.clear();
    scratch_ordered.extend(query.iter().map(|(ent, exec)| (ent, exec.id)));
    scratch_ordered.sort_by_key(|&(_, id)| id);

    for &(entity, _) in scratch_ordered.iter() {
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

        let max_tiles_f32 = if execution.target_owner == 0 {
            // Neutral expansion speed: grows purely with attack troops
            let base_speed = execution.troops as f32 / 50.0;
            base_speed.max(1.0).min(3.0) // At least 1, max 50
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
        
        // UNCONDITIONAL RNG ADVANCEMENT to strictly preserve deterministic state sequence
        let roll = (execution.rng.next_int(0, 1000) as f32) / 1000.0;
        if p > 0.0 && roll < p {
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

                // Terrain Check: Water and Mountains are impassable for basic ground attacks
                let terrain_type = game.map.terrain_type(target_tile.x, target_tile.y);
                if terrain_type == TerrainType::Water {
                    continue; // Skip, impassable terrain
                }

                // Check adjacency: ensure it still borders the attacker
                if !game.map.is_adjacent_to_player(target_tile.x, target_tile.y, execution.owner_id) {
                    continue; // Skip, edge was severed
                }

                // Troops were reserved on intent; only drain execution pool per tile (OpenFront).
                execution.troops -= crate::config::EXPAND_COST;
                execution.troops = execution.troops.max(0.0);
                expanded_this_tick += 1;

                // Troops were reserved on intent; only drain execution pool per tile (OpenFront).

                // Enqueue new neutral/enemy neighbors that touch our newly acquired tile
                game.map.for_each_neighbor(target_tile.x, target_tile.y, |nx, ny| {
                    // It must belong to target
                    if game.map.owner_id(nx, ny) != execution.target_owner {
                        return;
                    }

                    // How many tiles bordering (nx, ny) are owned by the attacker?
                    let mut num_owned_by_me = 0;
                    game.map.for_each_neighbor(nx, ny, |nnx, nny| {
                        if game.map.owner_id(nnx, nny) == execution.owner_id {
                            num_owned_by_me += 1;
                        }
                    });

                    let terrain = game.map.terrain_type(nx, ny);
                    let prio = execution.calc_priority(num_owned_by_me, terrain, tick_now);
                    let seq = execution.insert_seq_counter;
                    execution.insert_seq_counter = execution.insert_seq_counter.wrapping_add(1);

                    execution.to_conquer.push(PrioritizedTile {
                        priority: prio,
                        insert_seq: seq,
                        x: nx,
                        y: ny,
                    });
                });

                // VALID CONQUEST
                // Apply change AFTER enqueuing neighbors, mimicking OpenFront's `this._owner.conquer`
                // This ensures new neighbors don't artificially lower their priority by counting this tile as friendly yet!
                // Guaranteed BFS spread without DFS spikes!
                game.set_tile_owner(target_tile.x, target_tile.y, execution.owner_id);

                // Send event
                game.events.push(GameEvent::TileCaptured {
                    x: target_tile.x,
                    y: target_tile.y,
                    new_owner: execution.owner_id,
                });
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
