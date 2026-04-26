use std::cmp::Ordering;
use std::collections::BinaryHeap;
use bevy::prelude::*;
use crate::building::{
    aggregate_buildings_per_player, defense_post_priority_bonus, Building,
};
use crate::config;
use crate::game::{GameEvent, GamePhase, GameState};
use crate::intent::spawn_or_merge_attack_for_fleet_arrival;
use crate::map::TerrainType;
use crate::pathfinding::WaterPathfinderScratch;
use crate::pseudo_random::PseudoRandom;
use crate::warp_fleet::{best_shore_spawn_for_transport, WarpFleet};
use crate::water_components::WaterComponents;

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

#[inline]
fn fractional_extra_tiles_milli(max_tiles_f64: f64, roll_milli: u32) -> u32 {
    let frac = max_tiles_f64.fract().clamp(0.0, 0.999_999_999_999);
    let threshold_milli = (frac * 1000.0).floor() as u32; // 0..=999
    if threshold_milli > 0 && roll_milli < threshold_milli {
        1
    } else {
        0
    }
}

// System analogous to PlayerExecution.tick() in OpenFront
pub fn system_player_income(
    game_opt: Option<ResMut<GameState>>,
    q_building: Query<&Building>,
) {
    let Some(mut game) = game_opt else { return };
    if game.phase != GamePhase::Playing {
        return;
    }

    let max_pid = game.players.iter().map(|p| p.id as usize).max().unwrap_or(0);
    let buildings: Vec<Building> = q_building.iter().copied().collect();
    let aggs = aggregate_buildings_per_player(buildings.into_iter(), max_pid);

    let GameState { map: _, players, .. } = &mut *game;

    for player in players.iter_mut().filter(|p| p.alive) {
        let tiles_owned = player.tile_count;
        if tiles_owned == 0 {
            player.alive = false; // Dead
            continue;
        }

        let agg = aggs
            .get(player.id as usize)
            .copied()
            .unwrap_or_default();

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
        
        player.max_troops = config::MAX_TROOPS_BASE
            + max_troops_bonus * config::MAX_TROOPS_SCALE
            + agg.city_levels as f64 * config::CITY_MAX_TROOPS_PER_LEVEL;

        // safe_troops^0.75 = safe_troops^(1/2) * safe_troops^(1/4)
        let s_half = safe_troops.sqrt();
        let s_quarter = s_half.sqrt();
        let s_75 = s_half * s_quarter;
        
        let raw_income = config::TROOP_BASE_INCOME + (s_75 / 4.6);
        let ratio = 1.0 - (safe_troops / player.max_troops).min(1.0);
        let factory_extra = (agg.factory_levels as f64 * config::FACTORY_INCOME_BONUS_PER_LEVEL)
            .min(config::FACTORY_INCOME_BONUS_CAP - 1.0);
        let factory_mult = 1.0 + factory_extra;
        let income = raw_income * ratio * factory_mult;

        player.troops = (safe_troops + income).min(player.max_troops);

        let safe_gold = player.gold.max(0.0);
        let gold_income = config::GOLD_BASE_INCOME
            + agg.city_levels as f64 * config::GOLD_INCOME_PER_CITY_LEVEL;
        player.gold = safe_gold + gold_income;
    }
}

// System analogous to AttackExecution.tick() in OpenFront
pub fn system_attack_execution(
    mut commands: Commands,
    game_opt: Option<ResMut<GameState>>,
    mut query: Query<(Entity, &mut AttackExecution)>,
    q_building: Query<&Building>,
    mut scratch_ordered: Local<Vec<(Entity, u64)>>,
    mut buildings_scratch: Local<Vec<Building>>,
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

                if execution.target_owner == 0 {
                    // Neutral: attacker pays constant base cost
                    execution.troops -= crate::config::EXPAND_COST;
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
                    let dp_bonus = crate::building::defense_post_priority_bonus(
                        buildings,
                        execution.target_owner,
                        target_tile.x,
                        target_tile.y,
                        map_w,
                    );
                    let dp_multiplier = 1.0 + (dp_bonus as f64 / 10.0); // Approximation of defense buff

                    let atk_loss = crate::config::ATTACK_COST * terrain_multiplier * dp_multiplier;

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
                    prio += defense_post_priority_bonus(
                        buildings,
                        execution.target_owner,
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

pub fn system_game_tick(game_opt: Option<ResMut<GameState>>) {
    let Some(mut game) = game_opt else { return };
    game.tick();
}

fn refund_fleet_troops_to_player(game: &mut GameState, owner_id: u16, troops: f64) {
    if let Some(p) = game.player_mut(owner_id) {
        p.troops = (p.troops + troops.max(0.0)).min(p.max_troops);
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
mod player_income_building_tests {
    use super::*;
    use bevy::prelude::{App, MinimalPlugins};

    use crate::building::Building;
    use crate::game::{BuildingKind, GamePhase, GameState};
    use crate::player::Player;

    #[test]
    fn city_levels_raise_max_troops_via_income_system() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut game = GameState::new(42, 8, 8);
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p".into(), [1.0, 0.0, 0.0]));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = 50;
            p.troops = 10.0;
        }
        app.insert_resource(game);
        app.world_mut().spawn(Building {
            id: 1,
            owner_id: 1,
            tile_idx: 0,
            kind: BuildingKind::City,
            level: 4,
            under_construction: false,
            ticks_until_complete: 0,
        });
        app.add_systems(Update, system_player_income);
        app.update();
        let game = app.world().resource::<GameState>();
        let p = game.player(1).unwrap();
        let expected_floor =
            config::MAX_TROOPS_BASE + 4.0 * config::CITY_MAX_TROOPS_PER_LEVEL;
        assert!(
            p.max_troops + 0.01 >= expected_floor,
            "max_troops={} expected>={}",
            p.max_troops,
            expected_floor
        );
    }

    #[test]
    fn factory_income_multiplier_respects_cap() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut game = GameState::new(43, 8, 8);
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p".into(), [1.0, 0.0, 0.0]));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = 50;
            p.troops = 5.0;
        }
        app.insert_resource(game);
        for i in 0u32..20 {
            app.world_mut().spawn(Building {
                id: u64::from(i + 1),
                owner_id: 1,
                tile_idx: i,
                kind: BuildingKind::Factory,
                level: 1,
                under_construction: false,
                ticks_until_complete: 0,
            });
        }
        app.add_systems(Update, system_player_income);
        app.update();
        let game = app.world().resource::<GameState>();
        let p = game.player(1).unwrap();
        let low = 5.0_f64;
        let s_half = low.sqrt();
        let s_quarter = s_half.sqrt();
        let s_75 = s_half * s_quarter;
        let raw_income = config::TROOP_BASE_INCOME + (s_75 / 4.6);
        let ratio = 1.0 - (low / p.max_troops).min(1.0);
        let uncapped = raw_income * ratio * (1.0 + 20.0 * config::FACTORY_INCOME_BONUS_PER_LEVEL);
        let actual_gain = p.troops - low;
        assert!(
            actual_gain <= uncapped + 0.001,
            "income should be capped by FACTORY_INCOME_BONUS_CAP"
        );
        assert!(
            actual_gain <= raw_income * ratio * config::FACTORY_INCOME_BONUS_CAP + 0.02,
            "gain {} exceeds cap-scaled income",
            actual_gain
        );
    }

    #[test]
    fn gold_increments_each_income_tick() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut game = GameState::new(44, 8, 8);
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p".into(), [1.0, 0.0, 0.0]));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = 10;
            p.gold = 100.0;
        }
        app.insert_resource(game);
        app.add_systems(Update, system_player_income);
        app.update();
        let game = app.world().resource::<GameState>();
        let p = game.player(1).unwrap();
        let delta = p.gold - 100.0;
        assert!(
            (delta - config::GOLD_BASE_INCOME).abs() < 0.001,
            "gold delta {} expected {}",
            delta,
            config::GOLD_BASE_INCOME
        );
    }

    #[test]
    fn gold_income_scales_with_ready_city_levels() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut game = GameState::new(45, 8, 8);
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p".into(), [1.0, 0.0, 0.0]));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = 20;
            p.gold = 0.0;
        }
        app.insert_resource(game);
        app.world_mut().spawn(Building {
            id: 1,
            owner_id: 1,
            tile_idx: 0,
            kind: BuildingKind::City,
            level: 3,
            under_construction: false,
            ticks_until_complete: 0,
        });
        app.add_systems(Update, system_player_income);
        app.update();
        let game = app.world().resource::<GameState>();
        let p = game.player(1).unwrap();
        let expected = config::GOLD_BASE_INCOME + 3.0 * config::GOLD_INCOME_PER_CITY_LEVEL;
        assert!((p.gold - expected).abs() < 0.001, "gold={}", p.gold);
    }

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
