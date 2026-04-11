//! Dark Rift — Game Client (Bevy 0.18).
//!
//! Renders the star map, handles camera controls, and processes player input.
//! Single-player local mode for Phase 1–2. Networking (Lightyear) in Phase 3.

use bevy::prelude::*;
use bevy::input::mouse::{MouseScrollUnit, AccumulatedMouseScroll};
use dark_rift_shared::config;
use dark_rift_shared::game::{GamePhase, GameState};
use dark_rift_shared::map::SystemId;
use dark_rift_shared::attack::ActionResult;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use lightyear::netcode::NetcodeClient;
use lightyear::websocket::client::{WebSocketClientIo, WebSocketTarget};
use lightyear::prelude::client::NetcodeConfig;
use dark_rift_shared::protocol::{SharedProtocolPlugin, SyncMapState};
use std::net::{Ipv4Addr, SocketAddr};

// ─── App States ─────────────────────────────────────────────────────────────

#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
enum AppState {
    #[default]
    MainMenu,
    SpawnSelect,
    Playing,
    GameOver,
}

// ─── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource)]
struct DarkRiftGame {
    state: GameState,
    local_player_id: Option<u32>,
}

#[derive(Resource, Default)]
struct HoveredSystem(Option<SystemId>);

#[derive(Resource)]
struct GameTickTimer(Timer);

/// Camera drag state for panning (right-click drag).
#[derive(Resource, Default)]
struct CameraDrag {
    active: bool,
    last_pos: Vec2,
}

// ─── Components ─────────────────────────────────────────────────────────────

#[derive(Component)]
struct StarSystemNode { system_id: SystemId }

#[derive(Component)] struct ConnectionLine;
#[derive(Component)] struct TooltipText;
#[derive(Component)] struct ShipCounterText;
#[derive(Component)] struct LeaderboardText;
#[derive(Component)] struct MessageText;
#[derive(Component)] struct MainMenuUI;
#[derive(Component)] struct GameOverOverlay;

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Dark Rift — Galactic Conquest".to_string(),
                resolution: (1280u32, 720u32).into(),
                canvas: Some("#bevy-canvas".to_string()),
                ..default()
            }),
            ..default()
        }).disable::<bevy::audio::AudioPlugin>())
        .init_state::<AppState>()
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.06)))
        .init_resource::<HoveredSystem>()
        .init_resource::<CameraDrag>()
        .insert_resource(GameTickTimer(Timer::from_seconds(
            1.0 / config::TICK_RATE as f32,
            TimerMode::Repeating,
        )))
        .add_plugins(SharedProtocolPlugin)
        .add_plugins(ClientPlugins::default())
        // Startup
        .add_systems(Startup, setup)
        // Main Menu
        .add_systems(OnEnter(AppState::MainMenu), setup_main_menu)
        .add_systems(Update, main_menu_input.run_if(in_state(AppState::MainMenu)))
        .add_systems(OnExit(AppState::MainMenu), cleanup_main_menu)
        // Spawn Select
        .add_systems(OnEnter(AppState::SpawnSelect), setup_spawn_select)
        .add_systems(
            Update,
            (hover_system, spawn_select_click, update_system_visuals, update_tooltip)
                .chain()
                .run_if(in_state(AppState::SpawnSelect)),
        )
        // Playing
        .add_systems(OnEnter(AppState::Playing), setup_playing_hud)
        .add_systems(
            Update,
            (game_tick, hover_system, player_click, camera_controls, handle_network_sync)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            (update_system_visuals, update_tooltip, update_hud, check_game_over)
                .run_if(in_state(AppState::Playing)),
        )
        // Game Over
        .add_systems(OnEnter(AppState::GameOver), setup_game_over)
        .add_systems(Update, game_over_input.run_if(in_state(AppState::GameOver)))
        .run();
}

// ─── Startup ────────────────────────────────────────────────────────────────

fn setup(mut commands: Commands) {
    // 2D camera centered on the map.
    commands.spawn((
        Camera2d,
        Transform::from_translation(Vec3::new(
            config::MAP_WIDTH / 2.0,
            config::MAP_HEIGHT / 2.0,
            999.0,
        )),
    ));

    let game = GameState::new(42);
    commands.insert_resource(DarkRiftGame {
        state: game,
        local_player_id: None,
    });
    
    // Lightyear Client Networking Setup
    let server_addr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 5000);
    // Bind the client to an ephemeral local port
    let client_addr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
    
    let client_id = rand::random::<u64>();
    let auth = Authentication::Manual {
        server_addr,
        client_id,
        private_key: [0; 32],
        protocol_id: 0,
    };
    
    let client_entity = commands.spawn((
        Name::from("ClientConnection"),
        Client::default(),
        PeerAddr(server_addr),
        LocalAddr(client_addr),
        NetcodeClient::new(auth, NetcodeConfig::default()).unwrap(),
        WebSocketClientIo {
            config: Default::default(), // Removed With_no_cert_validation to use aeronet's default
            target: WebSocketTarget::Addr(Default::default()),
        },
    )).id();
    
    commands.trigger(Connect { entity: client_entity });
}

// ─── Shared: hover detection ─────────────────────────────────────────────────

/// Runs every frame to update which system the cursor is over.
fn hover_system(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    game: Res<DarkRiftGame>,
    mut hovered: ResMut<HoveredSystem>,
) {
    let Ok(window) = windows.single() else { return };
    let Ok((camera, cam_transform)) = camera_q.single() else { return };

    hovered.0 = None;

    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(cam_transform, cursor_pos) else { return };

    let mut closest: Option<(SystemId, f32)> = None;
    for sys in &game.state.map.systems {
        let dist = sys.position.distance(world_pos);
        if dist < 15.0 {
            if closest.map_or(true, |(_, d)| dist < d) {
                closest = Some((sys.id, dist));
            }
        }
    }
    hovered.0 = closest.map(|(id, _)| id);
}

// ─── Main Menu ──────────────────────────────────────────────────────────────

fn setup_main_menu(mut commands: Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(20.0),
                ..default()
            },
            MainMenuUI,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("DARK RIFT"),
                TextFont { font_size: 80.0, ..default() },
                TextColor(Color::srgb(0.0, 0.8, 1.0)),
            ));
            p.spawn((
                Text::new("Galactic Conquest"),
                TextFont { font_size: 28.0, ..default() },
                TextColor(Color::srgb(0.5, 0.5, 0.7)),
            ));
            p.spawn(Node { height: Val::Px(40.0), ..default() });
            p.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(40.0), Val::Px(15.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BorderColor::all(Color::srgb(0.0, 0.8, 1.0)),
                BackgroundColor(Color::srgba(0.0, 0.2, 0.4, 0.8)),
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("[ ENTER THE RIFT ]"),
                    TextFont { font_size: 24.0, ..default() },
                    TextColor(Color::srgb(0.0, 0.9, 1.0)),
                ));
            });
            p.spawn((
                Text::new("Press SPACE or click to begin"),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.3, 0.3, 0.5)),
            ));
        });
}

fn main_menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut next_state: ResMut<NextState<AppState>>,
    mut game: ResMut<DarkRiftGame>,
) {
    if keys.just_pressed(KeyCode::Space) || mouse.just_pressed(MouseButton::Left) {
        let player_id = game.state.add_human_player("Commander".to_string());
        game.local_player_id = Some(player_id);
        game.state.start_spawn_phase();
        next_state.set(AppState::SpawnSelect);
    }
}

fn cleanup_main_menu(mut commands: Commands, q: Query<Entity, With<MainMenuUI>>) {
    for e in q.iter() {
        commands.entity(e).try_despawn();
    }
}

// ─── Spawn Select ────────────────────────────────────────────────────────────

fn setup_spawn_select(mut commands: Commands, game: Res<DarkRiftGame>) {
    spawn_star_map(&mut commands, &game.state);

    commands.spawn((
        Text::new("Click an unclaimed star system to spawn. Choose wisely."),
        TextFont { font_size: 20.0, ..default() },
        TextColor(Color::srgb(0.7, 0.7, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(20.0),
            left: Val::Px(20.0),
            ..default()
        },
        MessageText,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 14.0, ..default() },
        TextColor(Color::srgb(0.8, 0.8, 0.8)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(20.0),
            left: Val::Px(20.0),
            ..default()
        },
        TooltipText,
    ));
}

fn spawn_select_click(
    mouse: Res<ButtonInput<MouseButton>>,
    hovered: Res<HoveredSystem>,
    mut game: ResMut<DarkRiftGame>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        if let (Some(system_id), Some(player_id)) = (hovered.0, game.local_player_id) {
            if game.state.player_spawn(player_id, system_id) {
                game.state.start_playing();
                next_state.set(AppState::Playing);
            }
        }
    }
}

// ─── Playing ─────────────────────────────────────────────────────────────────

fn setup_playing_hud(mut commands: Commands) {
    commands.spawn((
        Text::new("Ships: 0"),
        TextFont { font_size: 22.0, ..default() },
        TextColor(Color::srgb(0.0, 0.9, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(15.0),
            left: Val::Px(15.0),
            ..default()
        },
        ShipCounterText,
    ));

    commands.spawn((
        Text::new("Leaderboard"),
        TextFont { font_size: 16.0, ..default() },
        TextColor(Color::srgb(0.7, 0.7, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(15.0),
            right: Val::Px(15.0),
            ..default()
        },
        LeaderboardText,
    ));

    commands.spawn((
        Text::new("Click to expand (unclaimed) or attack (enemy)"),
        TextFont { font_size: 16.0, ..default() },
        TextColor(Color::srgb(0.5, 0.5, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(15.0),
            left: Val::Px(15.0),
            ..default()
        },
        MessageText,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 14.0, ..default() },
        TextColor(Color::srgb(0.7, 0.7, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(40.0),
            left: Val::Px(15.0),
            ..default()
        },
        TooltipText,
    ));
}

fn game_tick(
    time: Res<Time>,
    mut timer: ResMut<GameTickTimer>,
    mut game: ResMut<DarkRiftGame>,
) {
    timer.0.tick(time.delta());
    if timer.0.just_finished() {
        game.state.tick();
    }
}

fn player_click(
    mouse: Res<ButtonInput<MouseButton>>,
    hovered: Res<HoveredSystem>,
    mut game: ResMut<DarkRiftGame>,
    mut msg_q: Query<&mut Text, With<MessageText>>,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }

    let (Some(system_id), Some(player_id)) = (hovered.0, game.local_player_id) else { return };

    let result = game.state.player_action(player_id, system_id);

    let msg = match &result {
        ActionResult::Expanded { system_id } => {
            let name = game.state.map.system(*system_id).name.clone();
            format!("✦ Expanded into {name}")
        }
        ActionResult::Conquered { system_id, .. } => {
            let name = game.state.map.system(*system_id).name.clone();
            format!("⚔ Conquered {name}!")
        }
        ActionResult::AttackFailed { system_id, .. } => {
            let name = game.state.map.system(*system_id).name.clone();
            format!("✗ Attack on {name} failed — defenders held!")
        }
        ActionResult::Invalid { reason } => format!("⚠ {reason}"),
    };

    for mut text in msg_q.iter_mut() {
        **text = msg.clone();
    }
}

fn camera_controls(
    mut camera_q: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    scroll: Res<AccumulatedMouseScroll>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut drag: ResMut<CameraDrag>,
) {
    let Ok(window) = windows.single() else { return };

    // Zoom: accumulated scroll this frame adjusts the orthographic scale.
    if scroll.delta.y != 0.0 {
        let Ok((_, mut projection)) = camera_q.single_mut() else { return };
        if let Projection::Orthographic(ref mut ortho) = *projection {
            let zoom_speed = match scroll.unit {
                MouseScrollUnit::Line  => 0.1,
                MouseScrollUnit::Pixel => 0.001,
            };
            ortho.scale = (ortho.scale - scroll.delta.y * zoom_speed).clamp(0.2, 4.0);
        }
    }

    // Pan: right-click drag.
    if mouse.just_pressed(MouseButton::Right) {
        if let Some(pos) = window.cursor_position() {
            drag.active = true;
            drag.last_pos = pos;
        }
    }
    if mouse.just_released(MouseButton::Right) {
        drag.active = false;
    }
    if drag.active {
        if let Some(pos) = window.cursor_position() {
            let delta = pos - drag.last_pos;
            if let Ok((mut transform, projection)) = camera_q.single_mut() {
                let scale = if let Projection::Orthographic(ref o) = *projection {
                    o.scale
                } else {
                    1.0
                };
                transform.translation.x -= delta.x * scale;
                transform.translation.y += delta.y * scale;
            }
            drag.last_pos = pos;
        }
    }
}

fn check_game_over(game: Res<DarkRiftGame>, mut next_state: ResMut<NextState<AppState>>) {
    if game.state.phase == GamePhase::GameOver {
        next_state.set(AppState::GameOver);
    }
}

// ─── Game Over ───────────────────────────────────────────────────────────────

fn setup_game_over(mut commands: Commands, game: Res<DarkRiftGame>) {
    let winner_name = game
        .state
        .winner
        .and_then(|id| game.state.player(id))
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let is_local_win = game.state.winner == game.local_player_id;

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(20.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
            GameOverOverlay,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(if is_local_win { "VICTORY" } else { "DEFEATED" }),
                TextFont { font_size: 72.0, ..default() },
                TextColor(if is_local_win {
                    Color::srgb(0.0, 1.0, 0.5)
                } else {
                    Color::srgb(1.0, 0.2, 0.2)
                }),
            ));
            p.spawn((
                Text::new(format!("{winner_name} dominates the galaxy!")),
                TextFont { font_size: 24.0, ..default() },
                TextColor(Color::srgb(0.7, 0.7, 0.9)),
            ));
            p.spawn((
                Text::new("Press SPACE to return to menu"),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.4, 0.4, 0.6)),
            ));
        });
}

fn game_over_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<AppState>>,
    mut commands: Commands,
    overlay_q: Query<Entity, With<GameOverOverlay>>,
    map_q: Query<Entity, Or<(With<StarSystemNode>, With<ConnectionLine>)>>,
    hud_q: Query<Entity, Or<(
        With<ShipCounterText>,
        With<LeaderboardText>,
        With<MessageText>,
        With<TooltipText>,
    )>>,
) {
    if !keys.just_pressed(KeyCode::Space) { return; }

    for e in overlay_q.iter() { commands.entity(e).try_despawn(); }
    for e in map_q.iter()     { commands.entity(e).try_despawn(); }
    for e in hud_q.iter()     { commands.entity(e).try_despawn(); }

    // Fresh game with a random seed.
    commands.insert_resource(DarkRiftGame {
        state: GameState::new(rand::random()),
        local_player_id: None,
    });

    next_state.set(AppState::MainMenu);
}

// ─── Shared rendering ────────────────────────────────────────────────────────

fn spawn_star_map(commands: &mut Commands, game_state: &GameState) {
    let map = &game_state.map;

    // Connection lines (draw behind nodes).
    for sys in &map.systems {
        for &conn in &sys.connections {
            if conn <= sys.id { continue; }
            let a = sys.position;
            let b = map.system(conn).position;
            let mid = (a + b) / 2.0;
            let diff = b - a;
            let angle = diff.y.atan2(diff.x);

            commands.spawn((
                Sprite {
                    color: Color::srgba(0.15, 0.15, 0.3, 0.4),
                    custom_size: Some(Vec2::new(diff.length(), 1.5)),
                    ..default()
                },
                Transform::from_translation(mid.extend(0.1))
                    .with_rotation(Quat::from_rotation_z(angle)),
                ConnectionLine,
            ));
        }
    }

    // Star system nodes.
    for sys in &map.systems {
        let color = owner_color(game_state, sys.owner);
        commands.spawn((
            Sprite {
                color,
                custom_size: Some(Vec2::splat(if sys.owner.is_some() { 12.0 } else { 8.0 })),
                ..default()
            },
            Transform::from_translation(sys.position.extend(1.0)),
            StarSystemNode { system_id: sys.id },
        ));
    }
}

fn update_system_visuals(
    game: Res<DarkRiftGame>,
    hovered: Res<HoveredSystem>,
    mut q: Query<(&StarSystemNode, &mut Sprite)>,
) {
    for (node, mut sprite) in q.iter_mut() {
        let sys = game.state.map.system(node.system_id);
        let is_hovered = hovered.0 == Some(node.system_id);

        sprite.color = if is_hovered {
            Color::WHITE
        } else {
            owner_color(&game.state, sys.owner)
        };

        sprite.custom_size = Some(Vec2::splat(match (sys.owner.is_some(), is_hovered) {
            (true,  true)  => 16.0,
            (true,  false) => 12.0,
            (false, true)  => 13.0,
            (false, false) => 8.0,
        }));
    }
}

fn update_tooltip(
    game: Res<DarkRiftGame>,
    hovered: Res<HoveredSystem>,
    mut q: Query<&mut Text, With<TooltipText>>,
) {
    for mut text in q.iter_mut() {
        if let Some(sid) = hovered.0 {
            let sys = game.state.map.system(sid);
            let owner = match sys.owner {
                None => "Unclaimed".to_string(),
                Some(id) => game.state.player(id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
            };
            **text = format!("{} | {} | {:.0} ships | {} lanes",
                sys.name, owner, sys.ships, sys.connections.len());
        } else {
            **text = String::new();
        }
    }
}

fn update_hud(
    game: Res<DarkRiftGame>,
    mut ship_q: Query<&mut Text, (With<ShipCounterText>, Without<LeaderboardText>)>,
    mut leader_q: Query<&mut Text, (With<LeaderboardText>, Without<ShipCounterText>)>,
) {
    if let Some(pid) = game.local_player_id {
        if let Some(player) = game.state.player(pid) {
            for mut t in ship_q.iter_mut() {
                let systems = game.state.map.systems_owned_by(pid);
                **t = format!("⚓ Ships: {:.0}   ★ Systems: {}", player.reserve_ships, systems);
            }
        }
    }

    let board = game.state.leaderboard();
    let board_str = board.iter().take(5).enumerate()
        .map(|(i, (_, name, n))| format!("{}. {} — {}", i + 1, name, n))
        .collect::<Vec<_>>().join("\n");
    for mut t in leader_q.iter_mut() {
        **t = format!("── TOP 5 ──\n{board_str}");
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn owner_color(game_state: &GameState, owner: Option<u32>) -> Color {
    match owner {
        None => Color::srgba(0.35, 0.35, 0.45, 0.7),
        Some(id) => game_state.players.iter()
            .find(|p| p.id == id)
            .map(|p| Color::srgb(p.color[0], p.color[1], p.color[2]))
            .unwrap_or(Color::srgb(0.5, 0.5, 0.5)),
    }
}

// ─── Networking Systems ───────────────────────────────────────────────────────

fn handle_network_sync(
    mut game: ResMut<DarkRiftGame>,
    mut receivers: Query<&mut MessageReceiver<SyncMapState>>,
) {
    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            if let Some(state) = msg.state {
                game.state = state;
            }
        }
    }
}
