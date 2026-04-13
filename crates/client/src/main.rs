//! Dark Rift Client — Networked (Lightyear WebSocket + Netcode)
//!
//! Flow:
//!   1. Spawn a Client entity: NetcodeClient + WebSocketClientIo → ws://127.0.0.1:5000
//!   2. Trigger Connect to start handshake
//!   3. MessageReceiver<ServerStartMessage>: init local GameState, spawn tile grid
//!   4. On player click: send ClientIntentMessage via MessageSender
//!   5. MessageReceiver<ServerTurnMessage>: apply intents via apply_stamped_intent()
//!   6. Shared ECS systems run locally (system_player_income, system_attack_execution)
//!   7. Tile visuals updated from game.events (event-driven, O(events) not O(map))

use bevy::prelude::*;
use bevy::sprite_render::{TileData, TilemapChunk, TilemapChunkTileData};
use bevy::image::{ImageArrayLayout, ImageLoaderSettings};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use lightyear::netcode::{NetcodeClient, Key};
use lightyear::netcode::prelude::client::NetcodeConfig;
use lightyear::netcode::auth::Authentication;
use lightyear::websocket::client::{WebSocketClientIo, WebSocketScheme};
use aeronet_io::connection::PeerAddr;
use std::net::SocketAddr;
use clap::Parser;

use dark_rift_shared::config;
use dark_rift_shared::game::{GameState, GameEvent};
use dark_rift_shared::player::Player;
use dark_rift_shared::execution::{system_attack_execution, system_player_income, system_game_tick};
use dark_rift_shared::intent::apply_stamped_intent;
use dark_rift_shared::protocol::*;

const TILE_SIZE: f32 = 8.0;
const SERVER_ADDR: &str = "127.0.0.1:5005";

// ─── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource, Default)]
struct GameReady(bool);

#[derive(Resource, Default)]
struct MyPlayerId(u16);

#[derive(Resource, Default)]
struct HoveredTile(Option<(u32, u32)>);

#[derive(Resource, Default)]
struct PendingTicks(u32);

#[derive(Resource, Default)]
struct TurnQueue(std::collections::VecDeque<Turn>);

// ─── Components ─────────────────────────────────────────────────────────────

// Custom grid hashmap component track mapping Entity ID to spatial location.

#[derive(Component)]
struct HudText;

#[derive(Parser, Resource)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Start as an observer
    #[arg(short, long)]
    observer: bool,

    /// Play a replay file locally
    #[arg(short, long)]
    replay: Option<String>,
}

struct ReplayDirector {
    record: ReplayFile,
    turn_index: usize,
}

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    let is_replay = cli.replay.clone();

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Dark Rift — Multiplayer".to_string(),
            resolution: (1280u32, 720u32).into(),
            ..default()
        }),
        ..default()
    }).disable::<bevy::audio::AudioPlugin>())
    .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.04)))
    .insert_resource(cli)
    .init_resource::<GameReady>()
    .init_resource::<MyPlayerId>()
    .init_resource::<HoveredTile>()
    .init_resource::<PendingTicks>()
    .init_resource::<TurnQueue>()
    .init_resource::<HighestReceivedTurn>();

    if let Some(path) = is_replay {
        let file = std::fs::read_to_string(&path)
            .expect("Failed to read replay file");
        let record: ReplayFile = serde_json::from_str(&file)
            .expect("Failed to parse Replay json schema");
            
        app.insert_non_send_resource(ReplayDirector {
            record,
            turn_index: 0,
        });

        app.add_systems(Startup, setup_offline)
           .add_systems(Update, system_replay_pulse);
    } else {
        app.add_plugins(SharedProtocolPlugin)
           .add_plugins(ClientPlugins::default())
           .add_systems(Startup, setup_live)
           .add_systems(Update, (
               handle_server_start,
               handle_server_turns,
               handle_connection,
           ).chain());
    }

    app.add_systems(Update, (
        hover_system,
        player_click,
        handle_tile_change,
        update_hud,
        camera_movement_and_zoom,
    ).run_if(game_is_ready));

    app.add_systems(Update, clear_inputs_on_focus_loss);

    let mut sim_schedule = Schedule::new(SimulationSchedule);
    sim_schedule.add_systems((
        pop_turn,
        system_player_income,
        system_attack_execution,
        system_game_tick,
        decrement_pending,
    ).chain());
    app.add_schedule(sim_schedule);

    app.add_systems(Update, catchup_fast_forward);

    app.run();
}

#[derive(bevy::ecs::schedule::ScheduleLabel, Clone, Hash, Eq, PartialEq, Debug)]
struct SimulationSchedule;

fn catchup_fast_forward(world: &mut World) {
    let ready = world.resource::<GameReady>().0;
    if !ready { return; }
    
    let pending = world.resource::<PendingTicks>().0;
    if pending == 0 { return; }
    
    let process_count = if pending > 5 { 100.min(pending) } else { 1 };
    
    for _ in 0..process_count {
        if world.resource::<PendingTicks>().0 == 0 { break; }
        let _ = world.try_run_schedule(SimulationSchedule);
    }
}

fn game_is_ready(ready: Res<GameReady>) -> bool { ready.0 }

// Removed has_pending_ticks as it is obsolete

fn pop_turn(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    mut queue: ResMut<TurnQueue>,
) {
    let Some(game) = game_opt else { return };
    if let Some(turn) = queue.0.pop_front() {
        for (i, stamped) in turn.intents.iter().enumerate() {
            apply_stamped_intent(&mut commands, &game, stamped, i as u32);
        }
    }
}

fn decrement_pending(mut pending: ResMut<PendingTicks>) {
    pending.0 = pending.0.saturating_sub(1);
}

// ─── Camera Controls ────────────────────────────────────────────────────────

fn camera_movement_and_zoom(
    mut q_camera: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut scroll_evr: MessageReader<MouseWheel>,
) {
    for (mut transform, mut projection) in q_camera.iter_mut() {
        let bevy::prelude::Projection::Orthographic(ref mut ortho) = *projection else { continue };
        
        // Zoom
        let mut zoom = 0.0;
        for ev in scroll_evr.read() {
            match ev.unit {
                MouseScrollUnit::Line => zoom -= ev.y * 0.1,
                MouseScrollUnit::Pixel => zoom -= ev.y * 0.005,
            }
        }
        
        if zoom != 0.0 {
            ortho.scale = (ortho.scale * (1.0 + zoom)).clamp(0.1, 10.0);
        }
        
        // Pan
        let mut pan = Vec2::ZERO;
        let speed = 20.0 * ortho.scale;
        
        if keyboard_input.pressed(KeyCode::KeyW) || keyboard_input.pressed(KeyCode::ArrowUp) {
            pan.y += speed;
        }
        if keyboard_input.pressed(KeyCode::KeyS) || keyboard_input.pressed(KeyCode::ArrowDown) {
            pan.y -= speed;
        }
        if keyboard_input.pressed(KeyCode::KeyA) || keyboard_input.pressed(KeyCode::ArrowLeft) {
            pan.x -= speed;
        }
        if keyboard_input.pressed(KeyCode::KeyD) || keyboard_input.pressed(KeyCode::ArrowRight) {
            pan.x += speed;
        }
        
        transform.translation.x += pan.x;
        transform.translation.y += pan.y;
    }
}

// ─── Startup ────────────────────────────────────────────────────────────────

fn setup_offline(
    mut commands: Commands,
    mut ready: ResMut<GameReady>,
    director: NonSend<ReplayDirector>,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((
        Camera2d,
        Transform::from_xyz(
            config::MAP_WIDTH  as f32 * TILE_SIZE / 2.0,
            config::MAP_HEIGHT as f32 * TILE_SIZE / 2.0,
            999.0
        ),
    ));

    commands.spawn((
        Text::new("Replay Mode"),
        TextFont { font_size: 20.0, ..default() },
        TextColor(Color::srgb(0.7, 0.9, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            top: Val::Px(10.0),
            ..default()
        },
        HudText,
    ));

    let mut game = GameState::new(director.record.seed);
    for info in &director.record.players {
        let player = Player::new_human(info.id, info.name.clone(), info.color);
        game.spawn_player(player, info.spawn_x, info.spawn_y);
    }

    let tileset = asset_server.load_with_settings(
        "white_array.png",
        |settings: &mut ImageLoaderSettings| {
            settings.array_layout = Some(ImageArrayLayout::RowCount { rows: 2 });
        },
    );

    let chunk_size = UVec2::new(config::MAP_WIDTH as u32, config::MAP_HEIGHT as u32);
    let tile_display_size = UVec2::splat(TILE_SIZE as u32);
    let mut tile_data: Vec<Option<TileData>> = vec![None; (chunk_size.x * chunk_size.y) as usize];

    for y in 0..game.map.height {
        for x in 0..game.map.width {
            let idx = (y * game.map.width + x) as usize;
            tile_data[idx] = Some(openfront_tile_color(&game, x as i32, y as i32));
        }
    }

    commands.spawn((
        TilemapChunk {
            chunk_size,
            tile_display_size,
            tileset,
            ..default()
        },
        TilemapChunkTileData(tile_data),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    commands.insert_resource(game);
    ready.0 = true;
}

fn setup_live(mut commands: Commands, cli: Res<Cli>) {
    commands.spawn((
        Camera2d,
        Transform::from_xyz(0.0, 0.0, 999.0),
    ));

    commands.spawn((
        Text::new("Connecting to server..."),
        TextFont { font_size: 20.0, ..default() },
        TextColor(Color::srgb(0.7, 0.9, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            top: Val::Px(10.0),
            ..default()
        },
        HudText,
    ));

    let server_addr: SocketAddr = SERVER_ADDR.parse().unwrap();
    let client_id = rand_client_id();

    let mut netcode_cfg = NetcodeConfig::default();
    netcode_cfg.client_timeout_secs = 120;

    let netcode = NetcodeClient::new(
        Authentication::Manual {
            server_addr,
            client_id,
            private_key: Key::default(),
            protocol_id: 0,
        },
        netcode_cfg,
    ).expect("Failed to build NetcodeClient");

    let client_entity = commands.spawn((
        Name::from("DarkRiftClient"),
        netcode,
        PeerAddr(server_addr),
        WebSocketClientIo::from_addr(
            aeronet_websocket::client::ClientConfig::default(),
            WebSocketScheme::Plain,
        ),
    )).id();

    // Trigger Netcode connection (which internally initiates LinkStart)
    commands.trigger(lightyear::prelude::client::Connect { entity: client_entity });

    info!("🔌 Connecting to ws://{}... (client_id={}), observer={}", SERVER_ADDR, client_id, cli.observer);
}

fn handle_connection(
    mut client_q: Query<&mut MessageSender<ClientJoinMessage>, Added<lightyear::prelude::client::Client>>,
    cli: Res<Cli>,
) {
    for mut sender in client_q.iter_mut() {
        let msg = ClientJoinMessage { name: format!("Rig-{}", rand_client_id() % 1000), is_observer: cli.observer };
        info!("🤝 Dispatching ClientJoinMessage (Observer: {})...", cli.observer);
        let _ = sender.send::<ReliableChannel>(msg);
    }
}

// ─── Replay Offline Iteration ────────────────────────────────────────────────

fn system_replay_pulse(
    mut timer: Local<f32>,
    time:      Res<Time>,
    mut director: NonSendMut<ReplayDirector>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
) {
    *timer += time.delta_secs();
    let tick_len = (1.0 / config::TICK_RATE) as f32;
    
    while *timer >= tick_len {
        *timer -= tick_len;
        if director.turn_index < director.record.turns.len() {
            queue.0.push_back(director.record.turns[director.turn_index].clone());
            pending.0 += 1;
            director.turn_index += 1;
        }
    }
}

// ─── Network Handlers ────────────────────────────────────────────────────────

/// Receive ServerStartMessage → build GameState and spawn tile grid.
/// Mirrors OpenFront's message.type === "start" handler in ClientGameRunner.
fn handle_server_start(
    mut receivers: Query<&mut MessageReceiver<ServerStartMessage>>,
    mut ready:     ResMut<GameReady>,
    mut my_id:     ResMut<MyPlayerId>,
    mut cam_q:     Query<&mut Transform, With<Camera2d>>,
    mut hud_q:     Query<&mut Text, With<HudText>>,
    mut commands:  Commands,
    mut queue:     ResMut<TurnQueue>,
    mut pending:   ResMut<PendingTicks>,
    mut highest:   ResMut<HighestReceivedTurn>,
    asset_server:  Res<AssetServer>,
) {
    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            if ready.0 { continue; }

            if let Some(pid) = msg.my_player_id {
                info!("🎮 Game starting! I am player {}", pid);
                my_id.0 = pid;
            } else {
                info!("👁  Game starting! I am an observer");
                my_id.0 = 0;
            }

            let mut game = GameState::new(msg.seed);
            for info in &msg.players {
                let player = Player::new_human(info.id, info.name.clone(), info.color);
                game.spawn_player(player, info.spawn_x, info.spawn_y);
            }

            // Replay missed turns (late-join) — identical behavior to OpenFront
            for turn in &msg.missed_turns {
                queue.0.push_back(turn.clone());
                pending.0 += 1;
                highest.0 = Some(highest.0.unwrap_or(0).max(turn.turn_number));
            }

            // Center camera on map
            let cx = config::MAP_WIDTH as f32 * TILE_SIZE / 2.0;
            let cy = config::MAP_HEIGHT as f32 * TILE_SIZE / 2.0;
            if let Ok(mut cam) = cam_q.single_mut() {
                cam.translation = Vec3::new(cx, cy, 999.0);
            }

            let tileset = asset_server.load_with_settings(
                "white_array.png",
                |settings: &mut ImageLoaderSettings| {
                    settings.array_layout = Some(ImageArrayLayout::RowCount { rows: 2 });
                },
            );

            let chunk_size = UVec2::new(config::MAP_WIDTH as u32, config::MAP_HEIGHT as u32);
            let tile_display_size = UVec2::splat(TILE_SIZE as u32);
            let mut tile_data: Vec<Option<TileData>> = vec![None; (chunk_size.x * chunk_size.y) as usize];

            for y in 0..game.map.height {
                for x in 0..game.map.width {
                    let idx = (y * game.map.width + x) as usize;
                    tile_data[idx] = Some(openfront_tile_color(&game, x as i32, y as i32));
                }
            }

            commands.spawn((
                TilemapChunk {
                    chunk_size,
                    tile_display_size,
                    tileset,
                    ..default()
                },
                TilemapChunkTileData(tile_data),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ));

            commands.insert_resource(game);
            ready.0 = true;

            if let Ok(mut text) = hud_q.single_mut() {
                if msg.my_player_id.is_some() {
                    **text = format!("Connected — Player {:?}", msg.my_player_id);
                } else {
                    **text = "Connected — Observer".to_string();
                }
            }
        }
    }
}

#[derive(Resource, Default)]
struct HighestReceivedTurn(Option<u64>);

/// Receive ServerTurnMessage → apply each StampedIntent locally.
/// Mirrors OpenFront: message.type === "turn" → worker.sendTurn(turn)
///           → Executor.createExecs() per intent → AttackExecution.tick().
fn handle_server_turns(
    mut receivers: Query<&mut MessageReceiver<ServerTurnMessage>>,
    ready:         Res<GameReady>,
    mut queue:     ResMut<TurnQueue>,
    mut pending:   ResMut<PendingTicks>,
    mut highest:   ResMut<HighestReceivedTurn>,
) {
    if !ready.0 { return; }

    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            if let Some(h) = highest.0 {
                if msg.turn.turn_number <= h {
                    continue; // Drop overlapping buffered turns already processed via missed_turns!
                }
            }
            highest.0 = Some(msg.turn.turn_number);

            queue.0.push_back(msg.turn);
            pending.0 += 1;
        }
    }
}

// ─── Input ────────────────────────────────────────────────────────────────────

fn clear_inputs_on_focus_loss(
    mut events: MessageReader<bevy::window::WindowFocused>,
    mut mouse:  ResMut<ButtonInput<MouseButton>>,
    mut kb:     ResMut<ButtonInput<KeyCode>>,
) {
    for ev in events.read() {
        if !ev.focused {
            mouse.clear();
            kb.clear();
        }
    }
}

fn hover_system(
    windows:     Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera_q:    Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut hovered: ResMut<HoveredTile>,
    game_opt:    Option<Res<GameState>>,
) {
    hovered.0 = None;
    let Some(game) = game_opt else { return };
    let Ok(window) = windows.single() else { return };
    let Ok((camera, cam_t)) = camera_q.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok(world) = camera.viewport_to_world_2d(cam_t, cursor) else { return };

    let gx = (world.x / TILE_SIZE).round() as i32;
    let gy = (world.y / TILE_SIZE).round() as i32;
    if game.map.is_valid_coord(gx, gy) {
        hovered.0 = Some((gx as u32, gy as u32));
    }
}

/// On click: send intent to server via MessageSender.
/// Server stamps player_id → bundles into next Turn → broadcasts → we apply next frame.
fn player_click(
    mouse:       Res<ButtonInput<MouseButton>>,
    hovered:     Res<HoveredTile>,
    game_opt:    Option<Res<GameState>>,
    my_id:       Res<MyPlayerId>,
    mut senders: Query<&mut MessageSender<ClientIntentMessage>>,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }
    let Some((x, y)) = hovered.0 else { return };
    let Some(game) = game_opt else { return };

    let pid = my_id.0;
    let Some(player) = game.players.iter().find(|p| p.id == pid) else { return };
    if !player.alive || player.troops < config::EXPAND_COST { return; }

    let target_owner = game.map.owner_id(x, y);
    if target_owner == pid { return; }

    for mut sender in senders.iter_mut() {
        let _ = sender.send::<ReliableChannel>(ClientIntentMessage {
            intent: AttackIntent { target_owner, troops: None },
        });
    }
}

// ─── Visual ──────────────────────────────────────────────────────────────────

fn handle_tile_change(
    game_opt:    Option<ResMut<GameState>>,
    mut chunks:  Query<&mut TilemapChunkTileData>,
) {
    let Some(mut game) = game_opt else { return };
    if !game.is_changed() { return; }

    for ev in &game.events {
        if let GameEvent::TileCaptured { x, y, new_owner: _ } = ev {
            // OpenFront: update the tile AND its 4 cardinal neighbors (they may flip border/interior)
            let coords = [
                (*x as i32, *y as i32),
                (*x as i32, *y as i32 - 1),
                (*x as i32, *y as i32 + 1),
                (*x as i32 - 1, *y as i32),
                (*x as i32 + 1, *y as i32),
            ];
            for mut chunk_data in chunks.iter_mut() {
                for (cx, cy) in coords {
                    if game.map.is_valid_coord(cx, cy) {
                        let idx = (cy as u32 * config::MAP_WIDTH as u32 + cx as u32) as usize;
                        chunk_data[idx] = Some(openfront_tile_color(&game, cx, cy));
                    }
                }
            }
        }
    }
    
    // Clear out events so we effectively operate visually in 'Batched' chunks per-frame independently of tick iterations
    game.events.clear();
}

fn update_hud(
    game_opt:  Option<Res<GameState>>,
    my_id:     Res<MyPlayerId>,
    mut hud_q: Query<&mut Text, With<HudText>>,
) {
    let Some(game) = game_opt else { return };
    if !game.is_changed() { return; }
    let Ok(mut text) = hud_q.single_mut() else { return };

    let mut lines = vec![format!("Tick: {}", game.tick)];
    for p in &game.players {
        let tiles = game.map.tiles_owned_by(p.id);
        let you = if p.id == my_id.0 { " ◀ YOU" } else { "" };
        lines.push(format!("{}: {}t | {}tiles{}", p.name, p.troops as u32, tiles, you));
    }
    
    let final_str = lines.join("\n");
    if **text != final_str {
        **text = final_str;
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Returns the raw [r, g, b] (0..1) for a player, or dark void for unowned.
fn player_base_color(game: &GameState, owner: u16) -> [f32; 3] {
    if owner == 0 {
        return [0.08, 0.08, 0.12];
    }
    game.players.iter()
        .find(|p| p.id == owner)
        .map(|p| p.color)
        .unwrap_or([0.3, 0.3, 0.3])
}

/// OpenFront-style TileData:
/// - Border (any cardinal neighbor has different owner) → darken(0.125), alpha 100%
/// - Interior → base color, alpha 150/255
fn openfront_tile_color(game: &GameState, cx: i32, cy: i32) -> TileData {
    let owner = game.map.owner_id(cx as u32, cy as u32);
    let base = player_base_color(game, owner);

    let is_border = [(cx, cy - 1), (cx + 1, cy), (cx, cy + 1), (cx - 1, cy)]
        .iter()
        .any(|&(nx, ny)| {
            game.map.is_valid_coord(nx, ny)
                && game.map.owner_id(nx as u32, ny as u32) != owner
        });

    // Unowned tiles: always paint as dark void, no transparency effect needed
    let color = if owner == 0 {
        Color::srgba(base[0], base[1], base[2], 1.0)
    } else if is_border {
        // borderColor = territoryColor.darken(0.125), alpha = 255
        Color::srgba(base[0] * 0.875, base[1] * 0.875, base[2] * 0.875, 1.0)
    } else {
        // territoryColor, alpha = 150/255
        Color::srgba(base[0], base[1], base[2], 150.0 / 255.0)
    };

    TileData { tileset_index: 0, color, ..default() }
}

fn rand_client_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    nanos as u64
}
