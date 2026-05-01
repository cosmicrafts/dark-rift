use bevy::prelude::*;
use crate::building::{Building, DefenseGrid};
use crate::config;
use crate::game::{GameEvent, GamePhase, GameState};
use crate::map::TerrainType;
use crate::pathfinding::WaterPathfinderScratch;
use crate::warp_fleet::{best_shore_spawn_for_transport, WarpFleet};
use crate::water_components::WaterComponents;
use crate::intent::spawn_or_merge_attack_for_fleet_arrival;
use crate::rng::NextIntExt;
use super::{fractional_extra_tiles_milli, refund_fleet_troops_to_player, AttackExecution, PrioritizedTile, RETREAT_PENALTY_VS_PLAYER};

// System analogous to AttackExecution.tick() in OpenFront
pub fn system_attack_execution(
    mut commands: Commands,
    game_opt: Option<ResMut<GameState>>,
    mut query: Query<(Entity, &mut AttackExecution)>,
    q_building: Query<&Building>,
    mut scratch_ordered: Local<Vec<(Entity, u64)>>,
    mut buildings_scratch: Local<Vec<Building>>,
    mut defense_grid: Local<DefenseGrid>,
) {
    let Some(mut game) = game_opt else { return };
    if game.phase != GamePhase::Playing {
        return;
    }

    buildings_scratch.clear();
    buildings_scratch.extend(q_building.iter().copied());
    buildings_scratch.sort_by_key(|b| b.id); // Poka-Yoke: MUST be strictly sorted for determinism
    let buildings = buildings_scratch.as_slice();
    let map_w = game.map.width;

    let tick_now = game.tick;
    // Collect all AttackExecution entities and sort them by their perfectly coherent monotonic ID 
    // to strictly enforce identical processing execution order across clients!
    scratch_ordered.clear();
    scratch_ordered.extend(query.iter().map(|(ent, exec)| (ent, exec.id)));
    scratch_ordered.sort_by_key(|&(_, id)| id);

    for &(entity, _) in scratch_ordered.iter() {
        let Ok((_, mut execution)) = query.get_mut(entity) else { continue };

        // Pre-filter defense posts into a spatial grid to crush the O(N^2) scan down to O(1)
        defense_grid.rebuild(buildings, map_w, game.map.height, 16, execution.target_owner);

        if execution.retreating {
            let refund = execution.troops.max(0.0);
            if refund > 0.0 && refund.is_finite() {
                let factor = if execution.target_owner == 0 {
                    1.0
                } else {
                    1.0 - RETREAT_PENALTY_VS_PLAYER
                };
                let back = refund * factor;
                if let Some(player) = game.player_mut(execution.owner_id) {
                    player.troops = (player.troops + back).min(player.max_troops);
                }
            }
            commands.entity(entity).despawn();
            continue;
        }

        let max_tiles_f64 = if execution.target_owner == 0 {
            // Neutral expansion speed: OpenFront parity (proportional to border size)
            let adjacent = execution.to_conquer.len() as f64;
            (adjacent * 0.4).max(1.0).min(50.0) // Bounded organic growth
        } else {
            // PvP expansion speed: ratio based
            // Defensive clamp: troops CAN go negative when multiple attacks drain
            // simultaneously before income restores them. Negative defender_troops
            // would produce a negative ratio and (via clamp) identical power across
            // platforms, BUT the intermediate float bits can diverge on
            // ARM-WASM vs x86-WASM for negative/zero edge values.  Clamp first.
            let defender_troops = game
                .player(execution.target_owner)
                .map(|p| p.troops.max(0.0))
                .unwrap_or(1.0)
                .max(1.0);
            let ratio = execution.troops / defender_troops;
            let adjacent = execution.to_conquer.len() as f64;

            // OpenFront parity speed curve
            let power = (ratio * 0.5).clamp(0.05, 0.4); 
            (power * adjacent).max(0.1).min(100.0) // At least 0.1 (10% chance per tick), max 100 tiles/tick
        };

        // Determine actual integer number of tiles to process this tick (Fractional determinism)
        let mut tiles_to_conquer = max_tiles_f64.floor() as u32;
        // Unconditional RNG advancement preserves deterministic PRNG consumption order.
        let roll_milli = execution.rng.next_int(0, 1000) as u32; // 0..=999
        tiles_to_conquer += fractional_extra_tiles_milli(max_tiles_f64, roll_milli);

        let mut expanded_this_tick = 0u32;

        loop {
            if expanded_this_tick >= tiles_to_conquer {
                break;
            }

            if execution.troops < game.config.attack_cost_neutral || execution.troops.is_nan() {
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

                if execution.target_owner == 0 {
                    // Neutral: attacker pays constant base cost
                    execution.troops -= game.config.attack_cost_neutral;
                } else {
                    // PvP: Combat resolution (OpenFront Parity)
                    let mut def_loss = 0.0;
                    if let Some(target_player) = game.player(execution.target_owner) {
                        if target_player.tile_count > 0 {
                            // Defender loses proportional to their troop density.
                            // Clamp to 0: if troops went negative from multi-attack drain,
                            // a negative def_loss would ADD troops to the defender on
                            // subtract, compounding per tick and diverging across clients.
                            def_loss = target_player.troops.max(0.0) / target_player.tile_count as f64;
                        }
                    }

                    // Attacker pays base attack cost, scaled by terrain
                    let terrain_multiplier = match terrain_type {
                        TerrainType::Land => 1.0,
                        TerrainType::Highland => 1.25,
                        TerrainType::Mountain => 1.5,
                        _ => 1.0,
                    };

                    // Defense posts increase attacker losses slightly
                    let dp_bonus = defense_grid.priority_bonus(
                        target_tile.x,
                        target_tile.y,
                        map_w,
                    );
                    let dp_multiplier = 1.0 + (dp_bonus as f64 / 10.0); // Approximation of defense buff

                    let atk_loss = game.config.attack_cost_enemy * terrain_multiplier * dp_multiplier;

                    execution.troops -= atk_loss;

                    // Deduct from defender
                    if let Some(target_player) = game.player_mut(execution.target_owner) {
                        target_player.troops = (target_player.troops - def_loss).max(0.0);
                    }
                }
                
                execution.troops = execution.troops.max(0.0);
                expanded_this_tick += 1;

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
                    let mut prio = execution.calc_priority(num_owned_by_me, terrain, tick_now);
                    prio += defense_grid.priority_bonus(
                        nx,
                        ny,
                        map_w,
                    );
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
                    if let Some(player) = game.player_mut(execution.owner_id) {
                        player.troops = (player.troops + refund).min(player.max_troops);
                    }
                }
                commands.entity(entity).despawn();
                break;
            }
        }
    }
}

/// OpenFront `TransportShipExecution`: 1 tile/tick over water, retreat, landing → conquer + `AttackExecution`.
pub fn system_warp_fleet_execution(
    mut commands: Commands,
    game_opt: Option<ResMut<GameState>>,
    mut path_res: ResMut<WaterPathfinderScratch>,
    water_components: Res<WaterComponents>,
    mut q_fleet: Query<(Entity, &mut WarpFleet)>,
    mut q_exec: Query<(Entity, &mut AttackExecution)>,
    mut scratch: Local<Vec<(Entity, u64)>>,
) {
    let Some(mut game) = game_opt else {
        return;
    };
    if game.phase != GamePhase::Playing {
        return;
    }

    scratch.clear();
    scratch.extend(q_fleet.iter().map(|(e, w)| (e, w.id)));
    scratch.sort_by_key(|&(_, id)| id);

    let astar = &mut path_res.astar;

    for &(entity, _) in scratch.iter() {
        let Ok((_, mut fleet)) = q_fleet.get_mut(entity) else {
            continue;
        };

        if fleet.retreating && fleet.retreat_dst.is_none() {
            if let Some(player) = game.player(fleet.owner_id) {
                let here_comp = water_components.component_of(fleet.current_tile);
                let comp_filter = if here_comp == 0 {
                    None
                } else {
                    Some((&*water_components, here_comp))
                };
                if let Some(r_dst) = best_shore_spawn_for_transport(
                    &game.map,
                    fleet.owner_id,
                    &player.border_tiles,
                    fleet.current_tile,
                    comp_filter,
                ) {
                    fleet.retreat_dst = Some(r_dst);
                    if let Some(path) = astar.find_path(&game.map, &[fleet.current_tile], r_dst) {
                        fleet.path = path;
                        fleet.path_cursor = 0;
                    } else {
                        refund_fleet_troops_to_player(&mut game, fleet.owner_id, fleet.troops);
                        commands.entity(entity).despawn();
                        continue;
                    }
                } else {
                    refund_fleet_troops_to_player(&mut game, fleet.owner_id, fleet.troops);
                    commands.entity(entity).despawn();
                    continue;
                }
            } else {
                commands.entity(entity).despawn();
                continue;
            }
        }

        if fleet.path.is_empty() {
            refund_fleet_troops_to_player(&mut game, fleet.owner_id, fleet.troops);
            commands.entity(entity).despawn();
            continue;
        }

        if fleet.path_cursor < fleet.path.len() {
            fleet.current_tile = fleet.path[fleet.path_cursor];
            fleet.path_cursor += 1;
        }

        if fleet.path_cursor < fleet.path.len() {
            continue;
        }

        let w = game.map.width;
        let lx = fleet.current_tile % w;
        let ly = fleet.current_tile / w;
        let owner_here = game.map.owner_id(lx, ly);

        if fleet.retreating {
            if owner_here == fleet.owner_id {
                let deaths = fleet.troops * crate::warp_fleet::FLEET_RETREAT_SHORE_MALUS;
                let survivors = fleet.troops - deaths;
                if let Some(p) = game.player_mut(fleet.owner_id) {
                    p.troops = (p.troops + survivors).min(p.max_troops);
                }
            } else {
                refund_fleet_troops_to_player(&mut game, fleet.owner_id, fleet.troops);
            }
            commands.entity(entity).despawn();
            continue;
        }

        if owner_here == fleet.owner_id {
            let deaths = fleet.troops * crate::warp_fleet::FLEET_RETREAT_SHORE_MALUS;
            let survivors = fleet.troops - deaths;
            if let Some(p) = game.player_mut(fleet.owner_id) {
                p.troops = (p.troops + survivors).min(p.max_troops);
            }
            commands.entity(entity).despawn();
            continue;
        }

        let prev_owner = owner_here;
        game.set_tile_owner(lx, ly, fleet.owner_id);
        game.events.push(GameEvent::TileCaptured {
            x: lx,
            y: ly,
            new_owner: fleet.owner_id,
        });

        spawn_or_merge_attack_for_fleet_arrival(
            &mut commands,
            &mut game,
            &mut q_exec,
            fleet.owner_id,
            prev_owner,
            fleet.troops,
            fleet.id,
        );
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_extra_tile_milli_threshold_is_stable() {
        // 12.499 -> threshold 499 (roll 498 adds one; 499 does not)
        assert_eq!(fractional_extra_tiles_milli(12.499, 498), 1);
        assert_eq!(fractional_extra_tiles_milli(12.499, 499), 0);
        // Very small fraction below one milli should never add.
        assert_eq!(fractional_extra_tiles_milli(7.0009, 0), 0);
        // Near-one fraction clamps to 999 max threshold.
        assert_eq!(fractional_extra_tiles_milli(3.999_999_999, 998), 1);
        assert_eq!(fractional_extra_tiles_milli(3.999_999_999, 999), 0);
    }
}
