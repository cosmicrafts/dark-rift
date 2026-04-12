use bevy::prelude::*;
use bevy::sprite::Sprite;
use std::collections::HashMap;

use dark_rift_shared::config;
use dark_rift_shared::game::{GameState, GameEvent};
use dark_rift_shared::player::Player;
use dark_rift_shared::execution::{AttackExecution, PrioritizedTile, system_attack_execution, system_player_income};
use dark_rift_shared::pseudo_random::PseudoRandom;

const TILE_SIZE: f32 = 8.0;

// ─── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource, Default)]
struct HoveredTile(Option<(u32, u32)>);

#[derive(Resource)]
struct GameTickTimer(Timer);

#[derive(Resource, Default)]
struct GridEntities(HashMap<(u32, u32), Entity>);

// ─── Components ─────────────────────────────────────────────────────────────

#[derive(Component)]
struct TileNode {
    x: u32,
    y: u32,
}

#[derive(Component)]
struct HudText;

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Dark Rift — Local Mechanics Sandbox".to_string(),
                resolution: (1280u32, 720u32).into(),
                ..default()
            }),
            ..default()
        }).disable::<bevy::audio::AudioPlugin>())
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.04)))
        .init_resource::<HoveredTile>()
        .init_resource::<GridEntities>()
        .insert_resource(GameState::new(42)) // Inject the true shared core Resource directly!
        .insert_resource(GameTickTimer(Timer::from_seconds(
            1.0 / config::TICK_RATE as f32,
            TimerMode::Repeating,
        )))
        .add_systems(Startup, setup)
        .add_systems(Update, (
            hover_system,
            hover_visuals,
            player_click,
            update_hud,
        ))
        // Run identical shared mechanics natively! True simulation locally!
        .add_systems(Update, (
            game_tick,
            handle_tile_change,
            system_player_income,
            system_attack_execution,
        ))
        .run();
}

// ─── Startup ────────────────────────────────────────────────────────────────

fn setup(mut commands: Commands, mut game: ResMut<GameState>, mut grid: ResMut<GridEntities>) {
    // 2D camera centered on the grid
    commands.spawn((
        Camera2d,
        Transform::from_translation(Vec3::new(
            (game.map.width as f32 * TILE_SIZE) / 2.0,
            (game.map.height as f32 * TILE_SIZE) / 2.0,
            999.0,
        )),
    ));

    // Spawn tiles
    for y in 0..game.map.height {
        for x in 0..game.map.width {
            let entity = commands.spawn((
                Sprite {
                    color: Color::srgb(0.08, 0.08, 0.12),
                    custom_size: Some(Vec2::new(TILE_SIZE - 0.5, TILE_SIZE - 0.5)),
                    ..default()
                },
                Transform::from_xyz(
                    x as f32 * TILE_SIZE,
                    y as f32 * TILE_SIZE,
                    0.0,
                ),
                TileNode { x, y },
            )).id();

            grid.0.insert((x, y), entity);
        }
    }

    commands.spawn((
        Text::new("Loading..."),
        TextFont { font_size: 20.0, ..default() },
        TextColor(Color::srgb(0.7, 0.7, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            top: Val::Px(10.0),
            ..default()
        },
        HudText,
    ));

    let w = game.map.width;
    let h = game.map.height;

    // Instantly spawn Player 1 and Player 2 for testing collision and clashing!
    let player1 = Player::new_human(1, "Player 1".to_string(), [0.0, 0.8, 0.8]);
    game.spawn_player(player1, w / 3, h / 2);
    
    let player2 = Player::new_human(2, "Bot Player 2".to_string(), [0.8, 0.2, 0.2]);
    game.spawn_player(player2, w * 2 / 3, h / 2);

    // Initial event dispatch so their clusters render
    for (x, y) in grid.0.keys() {
        let owner = game.map.owner_id(*x, *y);
        if owner != 0 {
            game.events.push(dark_rift_shared::game::GameEvent::TileCaptured { x: *x, y: *y, new_owner: owner });
        }
    }
}

// ─── Systems ────────────────────────────────────────────────────────────────

fn game_tick(mut game: ResMut<GameState>, mut timer: ResMut<GameTickTimer>, time: Res<Time>) {
    timer.0.tick(time.delta());
    if timer.0.just_finished() {
        game.tick();
    }
}

fn hover_system(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut hovered: ResMut<HoveredTile>,
    game: Res<GameState>,
) {
    hovered.0 = None;
    let Ok(window) = windows.single() else { return };
    let Ok((camera, cam_transform)) = camera_q.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(cam_transform, cursor_pos) else { return };

    let grid_x = (world_pos.x / TILE_SIZE).round() as i32;
    let grid_y = (world_pos.y / TILE_SIZE).round() as i32;

    if game.map.is_valid_coord(grid_x, grid_y) {
        hovered.0 = Some((grid_x as u32, grid_y as u32));
    }
}

fn player_click(
    mouse: Res<ButtonInput<MouseButton>>,
    hovered: Res<HoveredTile>,
    mut commands: Commands,
    game: Res<GameState>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        if let Some((x, y)) = hovered.0 {
            let pid = 1; // You play as Player 1 in sandbox
            
            let player = match game.players.iter().find(|p| p.id == pid) {
                Some(p) => p,
                None => return,
            };

            let target_owner = game.map.owner_id(x, y);

            // Exactly the Openfront intent-based seed logic!
            let mut rng = PseudoRandom::new(game.seed + game.tick);
            let mut to_conquer = std::collections::BinaryHeap::new();

            for (bx, by) in game.map.border_tiles_of(pid) {
                let mut valid_border = false;
                for (nx, ny) in game.map.neighbors(bx, by) {
                    if game.map.owner_id(nx, ny) == target_owner {
                        valid_border = true;
                        
                        let mut num_owned_by_me = 0;
                        for (nnx, nny) in game.map.neighbors(nx, ny) {
                            if game.map.owner_id(nnx, nny) == pid {
                                num_owned_by_me += 1;
                            }
                        }
                        
                        let terrain = game.map.terrain_type(nx, ny);
                        let r = rng.next_int(0, 7) as f32;
                        let mag = match terrain {
                            dark_rift_shared::map::TerrainType::Plains => 1.0,
                            dark_rift_shared::map::TerrainType::Highland => 1.5,
                            dark_rift_shared::map::TerrainType::Mountain => 2.0,
                            _ => 1.0,
                        };
                        let prio = (r + 10.0) * (1.0 - (num_owned_by_me as f32) * 0.5 + mag / 2.0) + (game.tick as f32);

                        to_conquer.push(PrioritizedTile { priority: prio, x: nx, y: ny });
                    }
                }
            }

            if !to_conquer.is_empty() {
                commands.spawn(AttackExecution {
                    owner_id: pid,
                    target_owner,
                    troops: player.troops, // Commit all troops by default for sandbox
                    to_conquer,
                    rng,
                });
            }
        }
    }
}

fn handle_tile_change(
    game: Res<GameState>,
    grid: Res<GridEntities>,
    mut sprites: Query<&mut Sprite>,
) {
    if !game.is_changed() { return; }

    for ev in &game.events {
        if let GameEvent::TileCaptured { x, y, new_owner } = ev {
            if let Some(&entity) = grid.0.get(&(*x, *y)) {
                if let Ok(mut sprite) = sprites.get_mut(entity) {
                    if *new_owner == 0 {
                        sprite.color = Color::srgb(0.08, 0.08, 0.12);
                    } else {
                        if let Some(player) = game.players.iter().find(|p| p.id == *new_owner) {
                            sprite.color = Color::srgb(player.color[0], player.color[1], player.color[2]);
                        } else {
                            sprite.color = Color::srgb(0.3, 0.3, 0.3);
                        }
                    }
                }
            }
        }
    }
}

fn hover_visuals(
    hovered: Res<HoveredTile>,
    grid: Res<GridEntities>,
    mut sprites: Query<&mut Sprite>,
) {
    // Basic implementation that brightens hovered tile.
    // For MVP sandbox this is fine, skipping un-hovering previous tile to keep logic simple
    // since we do TileOwnerChanged observers now.
    if !hovered.is_changed() { return; }
    // ...
}

fn update_hud(
    game: Res<GameState>,
    mut hud_q: Query<&mut Text, With<HudText>>,
) {
    if !game.is_changed() { return; }

    for mut text in hud_q.iter_mut() {
        let mut lines = Vec::new();
        lines.push(format!("Sandbox Tick: {}", game.tick));
        for p in &game.players {
            let tiles = game.map.tiles_owned_by(p.id);
            lines.push(format!(
                "{}: {} troops | {} tiles",
                p.name,
                p.troops as u32,
                tiles,
            ));
        }
        **text = lines.join("\n");
    }
}
