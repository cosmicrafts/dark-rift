use bevy::prelude::*;
use crate::game::GameState;
use crate::protocol::*;
use crate::building::{Building, existing_structure_positions_with_width, structure_kind_enabled, structure_build_cost_gold, resolve_structure_spawn_tile};
use crate::execution::AttackExecution;
use crate::rng::{NextIntExt, WyRand, SeedableRng};
use crate::water_components::WaterComponents;
use crate::intent::apply_stamped_intent;
use crate::warp_fleet::WarpFleet;
use crate::game::BuildingKind;
fn bot_structure_target_count(
    kind: BuildingKind,
    city_equivalent: u32,
    bot_config: &crate::game_config::BotConfig,
) -> u32 {
    match kind {
        BuildingKind::DefensePost => ((city_equivalent as f64) * bot_config.defense_post_ratio).floor() as u32,
        BuildingKind::Port => ((city_equivalent as f64) * bot_config.port_ratio).floor() as u32,
        BuildingKind::Factory => ((city_equivalent as f64) * bot_config.factory_ratio).floor() as u32,
        BuildingKind::SamLauncher => ((city_equivalent as f64) * bot_config.sam_ratio).floor() as u32,
        BuildingKind::MissileSilo => ((city_equivalent as f64) * bot_config.silo_ratio).floor() as u32,
        BuildingKind::City => city_equivalent.saturating_add(1),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum BotDecisionKind {
    Build = 0,
    Attack = 1,
}

#[derive(Clone, Debug)]
struct BotDecision {
    bot_id: u16,
    kind: BotDecisionKind,
    intent: GameplayIntent,
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
    let tick = game.tick;
    let bot_config = &config.bot;
    let attack_interval_base = bot_config.attack_interval_base;

    let mut schedule: Vec<(u16, u64, u64)> = Vec::new();
    let mut any_structures = false;
    let total_bots = game.players.iter().filter(|p| !p.is_human() && p.alive).count() as u64;
    let mut bot_index = 0u64;
    for p in game.players.iter() {
        if p.is_human() || !p.alive {
            continue;
        }
        let bot_id = p.id;
        let mut sched_rng = WyRand::seed_from_u64(
            game.seed.wrapping_add(bot_id as u64).wrapping_add(attack_interval_base as u64)
        );
        let interval = sched_rng
            .next_int(attack_interval_base as i32, (attack_interval_base as i32 * 2).max(1))
            .max(1) as u64;
        // Uniform distribution: space out bots evenly across their interval to prevent spikes
        let offset = (bot_index * interval) / total_bots.max(1);
        bot_index += 1;
        schedule.push((bot_id, interval, offset));
        
        let phase = tick % interval;
        let one_third = (offset + interval / 3) % interval;
        let two_thirds = (offset + (interval * 2) / 3) % interval;
        let do_attack = phase == offset;
        let do_structures = do_attack || phase == one_third || phase == two_thirds;
        if do_structures {
            any_structures = true;
        }
    }
    schedule.sort_unstable_by_key(|(bot_id, _, _)| *bot_id);
    if schedule.is_empty() {
        return;
    }

    let mut buildings_snapshot: Vec<Building> = Vec::new();
    let mut structure_positions = Vec::new();
    if any_structures {
        buildings_snapshot = q_building.iter().map(|(_, b)| *b).collect();
        buildings_snapshot.sort_unstable_by_key(|b| b.id);
        structure_positions =
            existing_structure_positions_with_width(buildings_snapshot.iter(), game.map.width, None);
    }
    let mut decisions: Vec<BotDecision> = Vec::new();

    for (bot_id, interval, offset) in schedule {
        let phase = tick % interval;
        let one_third = (offset + interval / 3) % interval;
        let two_thirds = (offset + (interval * 2) / 3) % interval;
        let do_attack = phase == offset;
        let do_structures = do_attack || phase == one_third || phase == two_thirds;

        if false /* do_structures */ {
            // TODO(Debug): Bot building logic disabled completely
            let Some(player) = game.player(bot_id) else {
                continue;
            };
            let city_count = buildings_snapshot
                .iter()
                .filter(|b| b.owner_id == bot_id && b.kind == BuildingKind::City && !b.under_construction)
                .count() as u32;
            let city_equivalent = city_count.max((player.tile_count / 2000).max(1));
            let build_order = [
                BuildingKind::DefensePost,
                BuildingKind::Port,
                BuildingKind::Factory,
                BuildingKind::SamLauncher,
                BuildingKind::MissileSilo,
                BuildingKind::City,
            ];
            for kind in build_order {
                if !structure_kind_enabled(kind) {
                    continue;
                }
                let owned = buildings_snapshot
                    .iter()
                    .filter(|b| b.owner_id == bot_id && b.kind == kind && !b.under_construction)
                    .count() as u32;
                let target_count =
                    bot_structure_target_count(kind, city_equivalent, bot_config);
                if kind == BuildingKind::MissileSilo && owned >= 3 {
                    continue;
                }
                if owned >= target_count {
                    continue;
                }
                let cost = structure_build_cost_gold(kind, bot_id, &buildings_snapshot);
                let mut tile_choice: Option<u32> = None;
                let map_w = game.map.width;
                let Some(bot_now) = game.player_mut(bot_id) else { continue; };
                if bot_now.gold < cost {
                    continue;
                }

                // PERF: resolve_structure_spawn_tile allocates vec![false; map_area]
                // internally (BFS visited array). On a 2000x1200 map that's 2.4MB per call.
                // With 400 bots × 6 kinds × 256 attempts = OOM / freeze.
                // Fix: try exactly ONE random border tile. If it fails, wait until next tick.
                let border_len = bot_now.border_tiles.len();
                if border_len > 0 {
                    let pick = bot_now.bot_rng.next_int(0, border_len as i32) as usize;
                    let raw_idx = bot_now.border_tiles[pick];
                    let (bx, by) = (raw_idx % map_w, raw_idx / map_w);
                    let idx = by * map_w + bx;
                    if resolve_structure_spawn_tile(&game.map, bot_id, kind, idx, &structure_positions).is_some() {
                        tile_choice = Some(idx);
                    }
                }
                let Some(target_tile) = tile_choice else {
                    continue;
                };
                decisions.push(BotDecision {
                    bot_id,
                    kind: BotDecisionKind::Build,
                    intent: GameplayIntent::BuildStructure { kind, target_tile },
                });
                break;
            }
        }

        if !do_attack {
            continue;
        }
        let (troops, max_troops, border_tiles) = {
            let Some(player) = game.player(bot_id) else {
                continue;
            };
            (player.troops, player.max_troops, player.border_coords(game.map.width).collect::<Vec<_>>())
        };
        if border_tiles.is_empty() {
            continue;
        }
        let mut has_neutral = false;
        let mut player_targets = Vec::new();
        let max_scan = border_tiles.len().min(256);
        let start_idx = {
             let p_mut = game.player_mut(bot_id).unwrap();
             p_mut.bot_rng.next_int(0, border_tiles.len() as i32) as usize
        };
        for i in 0..max_scan {
            let b_idx = (start_idx + i) % border_tiles.len();
            let (bx, by) = border_tiles[b_idx];
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
        player_targets.sort_unstable();
        player_targets.dedup();
        let (target_owner, is_neutral) = if has_neutral {
            (0, true)
        } else if player_targets.is_empty() {
            continue;
        } else {
            let p_mut = game.player_mut(bot_id).unwrap();
            let t = if p_mut.bot_rng.next_int(0, bot_config.random_attack_div) == 0 {
                player_targets[p_mut.bot_rng.next_int(0, player_targets.len() as i32) as usize]
            } else {
                player_targets[0]
            };
            (t, false)
        };
        if !is_neutral && troops < max_troops * bot_config.trigger_ratio {
            continue;
        }
        let reserve = max_troops * if is_neutral { bot_config.expand_ratio } else { bot_config.reserve_ratio };
        let p_send = (troops - reserve).max(0.0);
        if p_send < game.config.attack_cost_neutral {
            continue;
        }
        decisions.push(BotDecision {
            bot_id,
            kind: BotDecisionKind::Attack,
            intent: GameplayIntent::Attack(AttackIntent {
                target_owner,
                troops: Some(p_send),
            }),
        });
    }

    decisions.sort_by_key(|d| (d.bot_id, d.kind));
    for (intent_index, d) in decisions.into_iter().enumerate() {
        let stamped = StampedIntent {
            player_id: d.bot_id,
            intent: d.intent,
        };
        apply_stamped_intent(
            &mut commands,
            &mut game,
            &mut q_exec,
            &mut q_fleet,
            &mut q_building,
            &stamped,
            intent_index as u32,
            &mut *path_scratch,
            &water_components,
        );
    }
}

#[cfg(test)]
mod building_intent_tests {
    use super::*;
    use bevy::app::App;
    use bevy::ecs::system::SystemState;
    use crate::building::Building;
    use crate::game::{BuildingKind, GamePhase, GameState};
    use crate::map::{GameMap, MapTile};
    use crate::player::Player;
    use crate::water_components::WaterComponents;
    use crate::intent::buildings::{apply_build_structure_intent, apply_upgrade_structure_intent};

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

        let mut game = GameState::new(99, 5, 1, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        game.map = m;
        game.players.push(Player::new_human(owner, "t".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default()));
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

        let mut game = GameState::new(101, w, 1, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        game.map = m;
        game.players.push(Player::new_human(owner, "w".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default()));
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

    #[test]
    fn build_intent_rejects_disabled_missile_structures() {
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
                BuildingKind::MissileSilo,
                2,
            );
        }
        state.apply(&mut world);
        world.flush();
        let has_silo = world
            .query::<&Building>()
            .iter(&world)
            .any(|b| b.kind == BuildingKind::MissileSilo);
        assert!(!has_silo);
    }

    fn bot_kind_code(kind: BuildingKind) -> u8 {
        match kind {
            BuildingKind::City => 0,
            BuildingKind::Factory => 1,
            BuildingKind::Port => 2,
            BuildingKind::DefensePost => 3,
            BuildingKind::SamLauncher => 4,
            BuildingKind::MissileSilo => 5,
        }
    }

    fn bot_game_with_order(order: &[u16]) -> GameState {
        let mut game = GameState::new(777, 12, 1, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        for &pid in order {
            game.players
                .push(Player::new_bot(pid, format!("b{pid}"), [0.2, 0.3, 0.4], &crate::game_config::GameConfig::default()));
            let idx = game.players.len() - 1;
            if game.player_lookup.len() <= pid as usize {
                game.player_lookup.resize(pid as usize + 1, None);
            }
            game.player_lookup[pid as usize] = Some(idx);
        }
        for x in 0..4 {
            game.set_tile_owner(x, 0, 2);
        }
        for x in 8..12 {
            game.set_tile_owner(x, 0, 3);
        }
        for p in game.players.iter_mut() {
            p.alive = true;
            p.max_troops = 10_000.0;
            p.troops = 8_000.0;
            p.gold = 10_000.0;
        }
        game
    }

    fn run_bot_think_digest(order: &[u16]) -> (Vec<u16>, Vec<(u16, u8, u32)>, Vec<(u16, u16, i64)>) {
        let mut app = App::new();
        app.insert_resource(bot_game_with_order(order));
        {
            let g = app.world().resource::<GameState>();
            app.insert_resource(WaterComponents::compute(&g.map));
        }
        app.insert_resource(crate::pathfinding::WaterPathfinderScratch::default());
        let cfg = crate::game_config::GameConfig::default();
        app.insert_resource(cfg);
        app.add_systems(bevy::prelude::Update, system_bot_think);

        for t in 0..320u64 {
            app.world_mut().resource_mut::<GameState>().tick = t;
            app.update();
        }

        let game = app.world().resource::<GameState>();
        let map_state = game.map.state.clone();
        let _ = game;

        let mut q_buildings = app.world_mut().query::<&Building>();
        let mut buildings: Vec<(u16, u8, u32)> = q_buildings
            .iter(app.world())
            .map(|b| (b.owner_id, bot_kind_code(b.kind), b.tile_idx))
            .collect();
        buildings.sort_unstable();

        let mut q_attacks = app.world_mut().query::<&AttackExecution>();
        let mut attacks: Vec<(u16, u16, i64)> = q_attacks
            .iter(app.world())
            .map(|a| (a.owner_id, a.target_owner, (a.troops * 1000.0) as i64))
            .collect();
        attacks.sort_unstable();
        (map_state, buildings, attacks)
    }

    #[test]
    fn bot_think_is_stable_across_player_insertion_order() {
        let a = run_bot_think_digest(&[2, 3]);
        let b = run_bot_think_digest(&[3, 2]);
        assert_eq!(a, b);
    }

    #[test]
    fn bot_think_is_repeatable_same_seed_and_order() {
        let a = run_bot_think_digest(&[2, 3]);
        let b = run_bot_think_digest(&[2, 3]);
        assert_eq!(a, b);
    }

    #[test]
    fn attack_intents_use_monotonic_attack_ids() {
        let mut world = World::new();
        let mut game = GameState::new(321, 3, 1, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p1".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default()));
        game.player_lookup = vec![None, Some(0)];
        game.set_tile_owner(0, 0, 1);
        game.set_tile_owner(1, 0, 0);
        game.set_tile_owner(2, 0, 0);
        if let Some(p) = game.player_mut(1) {
            p.troops = 5_000.0;
            p.max_troops = 10_000.0;
        }
        world.insert_resource(game);
        {
            let g = world.resource::<GameState>();
            world.insert_resource(WaterComponents::compute(&g.map));
        }
        world.insert_resource(crate::pathfinding::WaterPathfinderScratch::default());

        let mut state = SystemState::<(
            Commands,
            ResMut<GameState>,
            Query<(Entity, &mut AttackExecution)>,
            Query<(Entity, &mut WarpFleet)>,
            Query<(Entity, &mut Building)>,
            ResMut<crate::pathfinding::WaterPathfinderScratch>,
            Res<WaterComponents>,
        )>::new(&mut world);
        {
            let (
                mut commands,
                mut game,
                mut q_exec,
                mut q_fleet,
                mut q_building,
                mut scratch,
                water,
            ) = state.get_mut(&mut world);
            apply_stamped_intent(
                &mut commands,
                &mut game,
                &mut q_exec,
                &mut q_fleet,
                &mut q_building,
                &StampedIntent {
                    player_id: 1,
                    intent: GameplayIntent::Attack(AttackIntent { target_owner: 0, troops: Some(800.0) }),
                },
                0,
                &mut scratch,
                &water,
            );
            apply_stamped_intent(
                &mut commands,
                &mut game,
                &mut q_exec,
                &mut q_fleet,
                &mut q_building,
                &StampedIntent {
                    player_id: 1,
                    intent: GameplayIntent::Attack(AttackIntent { target_owner: 0, troops: Some(700.0) }),
                },
                1,
                &mut scratch,
                &water,
            );
        }
        state.apply(&mut world);
        world.flush();

        let mut q = world.query::<&AttackExecution>();
        let mut ids: Vec<u64> = q.iter(&world).map(|a| a.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(world.resource::<GameState>().next_attack_id, 3);
    }
}
