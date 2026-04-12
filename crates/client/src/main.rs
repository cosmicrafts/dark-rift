use bevy::prelude::*;
use bevy::sprite::Sprite;

use dark_rift_shared::config;
use dark_rift_shared::game::GameState;
use dark_rift_shared::player::Player;

const TILE_SIZE: f32 = 8.0;

// ─── Resources ──────────────────────────────────────────────────────────────

/// Local copy of the game state for our sandbox.
#[derive(Resource)]
struct DarkRiftGame {
    state: GameState,
}

#[derive(Resource, Default)]
struct HoveredTile(Option<(u32, u32)>);

#[derive(Resource)]
struct GameTickTimer(Timer);

// ─── Components ─────────────────────────────────────────────────────────────

#[derive(Component)]
struct TileNode {
    x: u32,
    y: u32,
}

#[derive(Component)]
struct HudText;

#[derive(Component, Debug, Clone)]
pub struct ExpansionExecution {
    pub player_id: u16,
    pub target_x: u32,
    pub target_y: u32,
}

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
        .insert_resource(DarkRiftGame { state: GameState::new(42) })
        .insert_resource(GameTickTimer(Timer::from_seconds(
            1.0 / config::TICK_RATE as f32,
            TimerMode::Repeating,
        )))
        .add_systems(Startup, setup)
        .add_systems(Update, (
            hover_system,
            player_click,
            update_tile_visuals,
            update_hud,
        ))
        // Server mechanics executed locally:
        .add_systems(Update, (
            game_tick,
            system_player_execution,
            system_process_expansions,
        ))
        .run();
}

// ─── Startup ────────────────────────────────────────────────────────────────

fn setup(mut commands: Commands, mut game: ResMut<DarkRiftGame>) {
    // 2D camera centered on the grid
    commands.spawn((
        Camera2d,
        Transform::from_translation(Vec3::new(
            (game.state.map.width as f32 * TILE_SIZE) / 2.0,
            (game.state.map.height as f32 * TILE_SIZE) / 2.0,
            999.0,
        )),
    ));

    // Spawn tile sprites for the grid
    for y in 0..game.state.map.height {
        for x in 0..game.state.map.width {
            commands.spawn((
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
            ));
        }
    }

    // HUD text
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

    // Instantly spawn Player 1 for local testing
    let pid = 1;
    let player = Player::new_human(pid, "Local Player".to_string(), [0.0, 0.8, 0.8]);
    let (sx, sy) = (game.state.map.width / 2, game.state.map.height / 2); // Start at center
    game.state.spawn_player(player, sx, sy);
    game.state.phase = dark_rift_shared::game::GamePhase::Playing;
}

// ─── UI & Input Systems ─────────────────────────────────────────────────────

fn hover_system(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut hovered: ResMut<HoveredTile>,
    game: Res<DarkRiftGame>,
) {
    hovered.0 = None;
    let Ok(window) = windows.single() else { return };
    let Ok((camera, cam_transform)) = camera_q.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(cam_transform, cursor_pos) else { return };

    let grid_x = (world_pos.x / TILE_SIZE).round() as i32;
    let grid_y = (world_pos.y / TILE_SIZE).round() as i32;

    if game.state.map.is_valid_coord(grid_x, grid_y) {
        hovered.0 = Some((grid_x as u32, grid_y as u32));
    }
}

/// Send clicks directly into Bevy Local ECS queue for mechanics testing!
fn player_click(
    mouse: Res<ButtonInput<MouseButton>>,
    hovered: Res<HoveredTile>,
    mut commands: Commands,
) {
    if mouse.just_pressed(MouseButton::Left) {
        if let Some((x, y)) = hovered.0 {
            // Hardcode our Local Player ID (1)
            commands.spawn(ExpansionExecution {
                player_id: 1,
                target_x: x,
                target_y: y,
            });
        }
    }
}

// ─── Server Mechanics Systems (Running Locally) ─────────────────────────────

fn game_tick(
    mut game: ResMut<DarkRiftGame>,
    mut timer: ResMut<GameTickTimer>,
    time: Res<Time>,
) {
    timer.0.tick(time.delta());
    if timer.0.just_finished() {
        game.state.tick();
    }
}

fn system_player_execution(
    mut game: ResMut<DarkRiftGame>,
    timer: Res<GameTickTimer>,
) {
    if !timer.0.just_finished() || game.state.phase != dark_rift_shared::game::GamePhase::Playing {
        return;
    }

    let GameState { map, players, .. } = &mut game.state;

    for player in players.iter_mut() {
        if !player.alive {
            continue;
        }

        let tiles_owned = map.tiles_owned_by(player.id);
        if tiles_owned == 0 {
            player.alive = false;
            continue;
        }

        player.max_troops = config::MAX_TROOPS_BASE
            + (tiles_owned as f64).powf(0.6) * config::MAX_TROOPS_SCALE;

        let raw_income = config::TROOP_BASE_INCOME
            + (player.troops.powf(0.73) / 4.0);
        let ratio = 1.0 - (player.troops / player.max_troops).min(1.0);
        let income = raw_income * ratio;

        player.troops = (player.troops + income).min(player.max_troops);
    }
}

fn system_process_expansions(
    mut commands: Commands,
    mut game: ResMut<DarkRiftGame>,
    timer: Res<GameTickTimer>,
    query: Query<(Entity, &ExpansionExecution)>,
) {
    if !timer.0.just_finished() || game.state.phase != dark_rift_shared::game::GamePhase::Playing {
        return;
    }

    let GameState { map, players, .. } = &mut game.state;

    for (entity, execution) in query.iter() {
        let pid = execution.player_id;
        
        // Fast exit drop to bypass borrow issues
        let player = match players.iter_mut().find(|p| p.id == pid) {
            Some(p) if p.alive => p,
            _ => {
                commands.entity(entity).despawn();
                continue;
            }
        };

        if map.owner_id(execution.target_x, execution.target_y) == pid {
            commands.entity(entity).despawn();
            continue;
        }

        if player.troops < config::EXPAND_COST {
            continue;
        }

        let mut candidates: Vec<(u32, u32, f64)> = Vec::new();
        for y in 0..map.height {
            for x in 0..map.width {
                let owner = map.owner_id(x, y);
                if owner != 0 {
                    continue; // Not neutral
                }
                if !map.is_adjacent_to_player(x, y, pid) {
                    continue; // Not adjacent to our territory
                }
                let dx = x as f64 - execution.target_x as f64;
                let dy = y as f64 - execution.target_y as f64;
                let dist = (dx * dx + dy * dy).sqrt();
                candidates.push((x, y, dist));
            }
        }

        candidates.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());

        let mut expanded_this_tick = 0u32;
        for (cx, cy, _dist) in &candidates {
            if expanded_this_tick >= config::EXPAND_TILES_PER_TICK {
                break;
            }
            if player.troops < config::EXPAND_COST {
                break;
            }

            map.set_owner_id(*cx, *cy, pid);
            player.troops -= config::EXPAND_COST;
            expanded_this_tick += 1;
        }
    }
}

// ─── Visual Updates ─────────────────────────────────────────────────────────

fn update_tile_visuals(
    game: Res<DarkRiftGame>,
    hovered: Res<HoveredTile>,
    mut tiles_q: Query<(&mut Sprite, &TileNode)>,
) {
    if !game.is_changed() && !hovered.is_changed() { return; }

    for (mut sprite, node) in tiles_q.iter_mut() {
        let owner = game.state.map.owner_id(node.x, node.y);

        let base_color = if owner == 0 {
            Color::srgb(0.08, 0.08, 0.12)
        } else {
            match game.state.players.iter().find(|p| p.id == owner) {
                Some(player) => Color::srgb(player.color[0], player.color[1], player.color[2]),
                None => Color::srgb(0.3, 0.3, 0.3),
            }
        };

        let final_color = if Some((node.x, node.y)) == hovered.0 {
            let c = base_color.to_srgba();
            Color::srgb((c.red + 0.25).min(1.0), (c.green + 0.25).min(1.0), (c.blue + 0.25).min(1.0))
        } else {
            base_color
        };

        sprite.color = final_color;
    }
}

fn update_hud(
    game: Res<DarkRiftGame>,
    mut hud_q: Query<&mut Text, With<HudText>>,
) {
    if !game.is_changed() { return; }

    for mut text in hud_q.iter_mut() {
        if game.state.players.is_empty() {
            **text = "Sandbox Loading...".to_string();
        } else {
            let mut lines = Vec::new();
            lines.push(format!("Sandbox Tick: {}", game.state.tick));
            for p in &game.state.players {
                let tiles = game.state.map.tiles_owned_by(p.id);
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
}
