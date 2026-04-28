//! Library tests (run with `cargo test -p dark-rift-shared --lib`).

use bevy::prelude::*;

use crate::execution::{system_warp_fleet_execution, AttackExecution};
use crate::game::{GameEvent, GamePhase, GameState};
use crate::map::{GameMap, MapTile};
use crate::pathfinding::WaterAStar;
use crate::intent::build_attack_frontier;
use crate::player::Player;
use bevy_prng::WyRand;
use rand_core::SeedableRng;
use std::collections::VecDeque;

use crate::pathfinding::WaterPathfinderScratch;
use crate::warp_fleet::{
    closest_neutral_shore_on_components, closest_target_shore_for_player,
    player_water_components, resolve_fleet_route, FleetLaunchError, FLEET_RETREAT_SHORE_MALUS,
    WarpFleet,
};
use crate::water_components::WaterComponents;

fn map_strip_two_islands() -> GameMap {
    let w = 4u32;
    let h = 1u32;
    let mut m = GameMap::new(w, h);
    for t in m.terrain.iter_mut() {
        *t = MapTile::from_byte(0b1000_0000);
    }
    m.terrain[1] = MapTile::from_byte(0);
    m
}

/// Wide map: P1 left, short river `11..=13`, P2 right — spawn radii (3) never overlap.
fn map_wide_river() -> GameMap {
    let w = 24u32;
    let h = 1u32;
    let mut m = GameMap::new(w, h);
    for t in m.terrain.iter_mut() {
        *t = MapTile::from_byte(0b1000_0000);
    }
    for i in 11..=13 {
        m.terrain[i as usize] = MapTile::from_byte(0);
    }
    m
}

fn setup_wide_river_game() -> GameState {
    let mut game = GameState::new(42, 24, 1, crate::game_config::GameConfig::default());
    game.phase = GamePhase::Playing;
    game.map = map_wide_river();
    let p1 = Player::new_human(1, "a".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default());
    let p2 = Player::new_human(2, "b".into(), [0.0, 1.0, 0.0], &crate::game_config::GameConfig::default());
    game.spawn_player(p1, 2, 0);
    game.spawn_player(p2, 21, 0);
    // Script territory so the river is neutral and tile 14 is a clean P2 beachhead.
    for x in 0..game.map.width {
        if game.map.terrain[x as usize].is_water() {
            game.set_tile_owner(x, 0, 0);
            continue;
        }
        if x <= 10 {
            game.set_tile_owner(x, 0, 1);
        } else if x >= 14 {
            game.set_tile_owner(x, 0, 2);
        } else {
            game.set_tile_owner(x, 0, 0);
        }
    }
    game
}

#[test]
fn frontier_nonempty_after_manual_landing_tile() {
    let mut game = setup_wide_river_game();
    game.set_tile_owner(14, 0, 1);
    let mut rng = WyRand::seed_from_u64(1);
    let mut seq = 0u32;
    let border: Vec<_> = game.player(1).unwrap().border_coords(game.map.width).collect();
    assert!(border.contains(&(14, 0)));
    let heap = build_attack_frontier(&game, 1, 2, &border, &mut rng, &mut seq);
    assert!(!heap.is_empty(), "expected attack frontier toward player 2");
}

#[test]
fn water_components_labels_two_disjoint_bodies() {
    // Layout: land(0) water(1) land(2) water(3) land(4) — two separate 1-tile
    // water bodies; tiles 0/2 share water body A, tile 2/4 share body B.
    let mut m = GameMap::new(5, 1);
    let shore = MapTile::from_byte(0b1100_0000);
    let water = MapTile::from_byte(0);
    m.terrain[0] = shore;
    m.terrain[1] = water;
    m.terrain[2] = shore;
    m.terrain[3] = water;
    m.terrain[4] = shore;

    let wc = WaterComponents::compute(&m);
    assert_eq!(wc.count, 2, "two disjoint water cells = two components");
    assert_ne!(wc.component_of(1), wc.component_of(3));
    assert_eq!(wc.component_of(0), wc.component_of(1));
    assert_eq!(wc.component_of(4), wc.component_of(3));
    // Shore tile between two components picks the smallest (deterministic).
    assert_eq!(wc.component_of(2), wc.component_of(1).min(wc.component_of(3)));
}

#[test]
fn closest_target_shore_respects_water_component() {
    // 5-tile strip: P1 owns shore at x=0, separate lake at x=1, P2 owns shore at
    // x=2 and x=4 (second is on a different water component). A boat from P1
    // can only land on x=2 — same water body as P1's shore.
    let mut m = GameMap::new(5, 1);
    let land_shore = MapTile::from_byte(0b1100_0000);
    let water = MapTile::from_byte(0);
    m.terrain[0] = land_shore;
    m.terrain[1] = water;
    m.terrain[2] = land_shore;
    m.terrain[3] = water;
    m.terrain[4] = land_shore;
    m.state[0] = 1;
    m.state[2] = 2;
    m.state[4] = 2;

    let wc = WaterComponents::compute(&m);
    let my_comps = player_water_components(&m, &wc, 1, &[0u32]);
    assert_eq!(my_comps.len(), 1);

    // Click on the far P2 shore (x=4) — landing must snap to x=2 because that's
    // the only P2 shore connected to P1's water.
    let got = closest_target_shore_for_player(&m, &wc, &my_comps, 2, &[2u32, 4u32], 4)
        .expect("expected a reachable shore");
    assert_eq!(got, 2);
}

#[test]
fn resolve_fleet_route_ok_same_water_component() {
    let mut m = GameMap::new(5, 1);
    let land_shore = MapTile::from_byte(0b1100_0000);
    let water = MapTile::from_byte(0);
    m.terrain[0] = land_shore;
    m.terrain[1] = water;
    m.terrain[2] = land_shore;
    m.terrain[3] = water;
    m.terrain[4] = land_shore;
    m.state[0] = 1;
    m.state[2] = 2;
    m.state[4] = 2;
    let wc = WaterComponents::compute(&m);
    let mut scratch = WaterPathfinderScratch::default();
    let route = resolve_fleet_route(
        &m,
        &wc,
        &mut scratch,
        1,
        2,
        4,
        &[0u32],
        Some(&[2u32, 4u32] as &[u32]),
        true,
    )
    .expect("route");
    assert_eq!(route.landing_tile, 2);
    assert_eq!(route.src_tile, 0);
    assert!(route.path.len() >= 2);
    assert_eq!(*route.path.first().unwrap(), 0);
    assert_eq!(*route.path.last().unwrap(), 2);
}

#[test]
fn resolve_fleet_route_err_no_landing_wrong_component() {
    let mut m = GameMap::new(5, 1);
    let land_shore = MapTile::from_byte(0b1100_0000);
    let water = MapTile::from_byte(0);
    m.terrain[0] = land_shore;
    m.terrain[1] = water;
    m.terrain[2] = land_shore;
    m.terrain[3] = water;
    m.terrain[4] = land_shore;
    m.state[0] = 1;
    m.state[4] = 2; // P2 only on far shore (different water body than P1)
    let wc = WaterComponents::compute(&m);
    let mut scratch = WaterPathfinderScratch::default();
    let err = resolve_fleet_route(
        &m,
        &wc,
        &mut scratch,
        1,
        2,
        4,
        &[0u32],
        Some(&[4u32] as &[u32]),
        true,
    )
    .unwrap_err();
    assert_eq!(err, FleetLaunchError::NoLandingShore);
}

#[test]
fn resolve_fleet_route_ok_neutral_shore() {
    // P1 shore at 0, one water body at 1, neutral shoreline at 2, neutral inland click at 3.
    let mut m = GameMap::new(5, 1);
    let shore = MapTile::from_byte(0b1100_0000);
    let water = MapTile::from_byte(0);
    let inland = MapTile::from_byte(0b1000_0000);
    m.terrain[0] = shore;
    m.terrain[1] = water;
    m.terrain[2] = shore;
    m.terrain[3] = inland;
    m.terrain[4] = inland;
    m.state[0] = 1;
    m.state[2] = 0;
    m.state[3] = 0;
    let wc = WaterComponents::compute(&m);
    let mut scratch = WaterPathfinderScratch::default();
    let route = resolve_fleet_route(
        &m,
        &wc,
        &mut scratch,
        1,
        0,
        3,
        &[0u32],
        None,
        true,
    )
    .expect("neutral route");
    assert_eq!(route.landing_tile, 2);
    assert_eq!(route.src_tile, 0);
}

#[test]
fn resolve_fleet_route_err_no_port() {
    let mut m = GameMap::new(5, 1);
    let land_shore = MapTile::from_byte(0b1100_0000);
    let water = MapTile::from_byte(0);
    m.terrain[0] = land_shore;
    m.terrain[1] = water;
    m.terrain[2] = land_shore;
    m.terrain[3] = water;
    m.terrain[4] = land_shore;
    m.state[0] = 1;
    m.state[2] = 2;
    m.state[4] = 2;
    let wc = WaterComponents::compute(&m);
    let mut scratch = WaterPathfinderScratch::default();
    let err = resolve_fleet_route(
        &m,
        &wc,
        &mut scratch,
        1,
        2,
        4,
        &[0u32],
        Some(&[2u32, 4u32] as &[u32]),
        false,
    )
    .unwrap_err();
    assert_eq!(err, FleetLaunchError::NoPort);
}

#[test]
fn neutral_shore_lookup_bfs_deterministic() {
    let mut m = GameMap::new(8, 1);
    let land_shore = MapTile::from_byte(0b1100_0000);
    let water = MapTile::from_byte(0);
    for t in m.terrain.iter_mut() {
        *t = MapTile::from_byte(0b1000_0000);
    }
    m.terrain[3] = water;
    m.terrain[2] = land_shore;
    m.terrain[4] = land_shore;
    // Leave ownership at 0 for shores (neutral); the caller supplies its own
    // component list as if they owned the opposite shore.
    let wc = WaterComponents::compute(&m);
    let target_comp = wc.component_of(2);
    let my_comps = vec![target_comp];

    let mut queue = VecDeque::new();
    let mut visited = Vec::new();
    let mut stamp = 0u32;
    let click = 3u32;
    let first = closest_neutral_shore_on_components(
        &m, &wc, &my_comps, click, 50,
        &mut queue, &mut visited, &mut stamp,
    )
    .expect("expected a neutral shore within range");
    for _ in 0..1000 {
        let got = closest_neutral_shore_on_components(
            &m, &wc, &my_comps, click, 50,
            &mut queue, &mut visited, &mut stamp,
        )
        .expect("expected a neutral shore within range");
        assert_eq!(got, first, "BFS scratch reuse must stay deterministic");
    }
}

#[test]
fn water_astar_deterministic_across_runs() {
    let mut map = map_strip_two_islands();
    map.state[0] = 1;
    map.state[1] = 0;
    map.state[2] = 2;
    map.state[3] = 2;

    let mut astar = WaterAStar::new();
    let first = astar
        .find_path(&map, &[1u32], 2u32)
        .expect("path must exist");
    for _ in 0..1000 {
        let p = astar
            .find_path(&map, &[1u32], 2u32)
            .expect("path must exist");
        assert_eq!(p, first, "A* path must be byte-identical every run");
    }
}

#[test]
fn warp_fleet_landing_spawns_attack_execution() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<crate::pathfinding::WaterPathfinderScratch>();

    let mut game = setup_wide_river_game();
    game.player_mut(1).unwrap().troops = 500.0;

    app.insert_resource(WaterComponents::compute(&game.map));
    app.insert_resource(game);
    app.add_systems(Update, system_warp_fleet_execution);

    // Water path 11..13 then land at 14 (enemy).
    app.world_mut().spawn(WarpFleet::new(
        1,
        1,
        2,
        50.0,
        11,
        14,
        vec![11, 12, 13, 14],
    ));

    let mut steps = 0u32;
    while app.world_mut().query::<&WarpFleet>().iter(app.world_mut()).next().is_some() {
        app.update();
        steps += 1;
        assert!(steps < 32, "fleet should reach destination within 32 sim ticks");
    }
    // `Commands` from the fleet system may flush after deferred passes; poll briefly.
    let mut attacks = 0usize;
    for _ in 0..8 {
        app.update();
        attacks = app
            .world_mut()
            .query::<&AttackExecution>()
            .iter(app.world_mut())
            .count();
        if attacks > 0 {
            break;
        }
    }
    assert_eq!(attacks, 1, "AttackExecution should spawn after landing");
    let atk = app
        .world_mut()
        .query::<&AttackExecution>()
        .iter(app.world_mut())
        .next()
        .expect("attack");
    assert_eq!(atk.owner_id, 1);
    assert_eq!(atk.target_owner, 2);
    assert_eq!(atk.troops, 50.0);

    let g = app.world_mut().resource::<GameState>();
    assert_eq!(g.map.owner_id(14, 0), 1);
    assert_eq!(g.map.owner_id(15, 0), 2);
    assert!(
        g.events.iter().any(|e| matches!(
            e,
            GameEvent::TileCaptured { x: 14, y: 0, new_owner: 1 }
        )),
        "TileCaptured event for landing tile"
    );
}

#[test]
fn warp_fleet_retreat_returns_survivors_with_malus() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<crate::pathfinding::WaterPathfinderScratch>();

    let mut game = GameState::new(7, 4, 1, crate::game_config::GameConfig::default());
    game.phase = GamePhase::Playing;
    game.map = map_strip_two_islands();
    let p1 = Player::new_human(1, "a".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default());
    game.spawn_player(p1, 0, 0);
    // Troops were already "loaded" onto the fleet in a real launch; pool starts empty here.
    game.player_mut(1).unwrap().troops = 0.0;

    app.insert_resource(WaterComponents::compute(&game.map));
    app.insert_resource(game);
    app.add_systems(Update, system_warp_fleet_execution);

    app.world_mut().spawn(WarpFleet {
        id: 9,
        owner_id: 1,
        target_owner: 0,
        troops: 100.0,
        src_tile: 1,
        dst_tile: 0,
        retreat_dst: Some(0),
        path: vec![1, 0],
        path_cursor: 0,
        current_tile: 1,
        retreating: true,
    });

    app.update();
    app.update();

    assert!(app
        .world_mut()
        .query::<&WarpFleet>()
        .iter(app.world_mut())
        .next()
        .is_none());
    let expected = 100.0 * (1.0 - FLEET_RETREAT_SHORE_MALUS);
    let got = app
        .world()
        .resource::<GameState>()
        .player(1)
        .unwrap()
        .troops;
    assert!(
        (got - expected).abs() < 1e-9,
        "expected {} survivors refunded to pool, got {}",
        expected,
        got
    );
}
