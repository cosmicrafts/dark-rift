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
use bevy::sprite::Sprite;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use lightyear::netcode::{NetcodeClient, Key};
use lightyear::netcode::prelude::client::NetcodeConfig;
use lightyear::netcode::auth::Authentication;
use lightyear::websocket::client::{WebSocketClientIo, WebSocketScheme};
use aeronet_io::connection::PeerAddr;
use std::collections::HashMap;
use std::net::SocketAddr;

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
struct GridEntities(HashMap<(u32, u32), Entity>);

#[derive(Resource, Default)]
struct PendingTicks(u32);

#[derive(Resource, Default)]
struct TurnQueue(std::collections::VecDeque<Turn>);

// ─── Components ─────────────────────────────────────────────────────────────

#[derive(Component)]
struct TileNode { x: u32, y: u32 }

#[derive(Component)]
struct HudText;

#[derive(clap::Parser, Resource, Clone)]
struct Cli {
    #[arg(long)]
    replay: Option<String>,
    #[arg(long, default_value_t = false)]
    observer: bool,
}

#[derive(Resource)]
struct ReplayDirector {
    turn_index: usize,
    record: dark_rift_shared::protocol::ReplayFile,
    timer: Timer,
}

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let cli = <Cli as clap::Parser>::parse();

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
    .add_plugins(SharedProtocolPlugin)
    .init_resource::<GameReady>()
    .init_resource::<MyPlayerId>()
    .init_resource::<HoveredTile>()
    .init_resource::<GridEntities>()
    .init_resource::<PendingTicks>()
    .init_resource::<TurnQueue>();

    if let Some(path) = cli.replay.clone() {
        let json = std::fs::read_to_string(&path).expect("Failed to read replay file");
        let record: dark_rift_shared::protocol::ReplayFile = serde_json::from_str(&json).expect("Failed to deserialize ReplayFile");
        
        app.insert_resource(ReplayDirector {
            turn_index: 0,
            record,
            timer: Timer::from_seconds(50.0 / 1000.0, TimerMode::Repeating),
        });

        app.add_systems(Startup, setup_offline);
        app.add_systems(Update, system_replay_pulse);
    } else {
        // Only load live Networking plugins
        app.add_plugins(ClientPlugins::default());
        app.insert_resource(cli); // Pass to setup
        app.add_systems(Startup, setup_live);
        app.add_systems(Update, (
            handle_server_start,
            handle_server_turns,
            handle_connection,
        ));
    }

    app.add_systems(Update, (
        hover_system,
        player_click,
    ).run_if(game_is_ready))
    .add_systems(Update, (
        pop_turn,
        system_player_income,
        system_attack_execution,
        handle_tile_change,
        update_hud,
        system_game_tick,
        decrement_pending,
    ).chain().run_if(has_pending_ticks))
    .run();
}

fn game_is_ready(ready: Res<GameReady>) -> bool { ready.0 }

fn has_pending_ticks(ready: Res<GameReady>, pending: Res<PendingTicks>) -> bool {
    ready.0 && pending.0 > 0
}

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

// ─── Replay Offline & Live Handlers ──────────────────────────────────────────

fn system_replay_pulse(
    time: Res<Time>,
    mut director: ResMut<ReplayDirector>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
) {
    if director.timer.tick(time.delta()).just_finished() {
        if director.turn_index < director.record.turns.len() {
            queue.0.push_back(director.record.turns[director.turn_index].clone());
            pending.0 += 1;
            director.turn_index += 1;
        }
    }
}

fn setup_offline(
    mut commands: Commands,
    mut game_ready: ResMut<GameReady>,
    mut grid: ResMut<GridEntities>,
    director: Res<ReplayDirector>,
) {
    let cx = config::MAP_WIDTH as f32 * TILE_SIZE / 2.0;
    let cy = config::MAP_HEIGHT as f32 * TILE_SIZE / 2.0;
    commands.spawn((Camera2d, Transform::from_xyz(cx, cy, 999.0)));
    commands.spawn((
        Text::new("Replay Viewer"),
        TextFont { font_size: 20.0, ..default() },
        TextColor(Color::srgb(0.7, 0.9, 0.7)),
        Node { position_type: PositionType::Absolute, left: Val::Px(10.0), top: Val::Px(10.0), ..default() },
        HudText,
    ));

    // Construct the explicit offline game configuration.
    let mut game = GameState::new(director.record.seed);
    for info in &director.record.players {
        let player = Player::new_human(info.id, info.name.clone(), info.color);
        game.spawn_player(player, info.spawn_x, info.spawn_y);
    }

    // Spawn identical Map Grid tiles locally
    for y in 0..game.map.height {
        for x in 0..game.map.width {
            let owner = game.map.owner_id(x, y);
            let color = color_for_owner(&game, owner);
            let entity = commands.spawn((
                Sprite {
                    color,
                    custom_size: Some(Vec2::new(TILE_SIZE - 0.5, TILE_SIZE - 0.5)),
                    ..default()
                },
                Transform::from_xyz(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE, 0.0),
                TileNode { x, y },
            )).id();
            grid.0.insert((x, y), entity);
        }
    }

    // Inject offline GameState + trigger systems
    commands.insert_resource(game);
    game_ready.0 = true;
}

fn setup_live(mut commands: Commands, cli: Res<Cli>) {
    commands.spawn((Camera2d, Transform::from_xyz(0.0, 0.0, 999.0)));
    commands.spawn((
        Text::new("Connecting to server..."),
        TextFont { font_size: 20.0, ..default() },
        TextColor(Color::srgb(0.7, 0.9, 0.7)),
        Node { position_type: PositionType::Absolute, left: Val::Px(10.0), top: Val::Px(10.0), ..default() },
        HudText,
    ));

    let server_addr: SocketAddr = SERVER_ADDR.parse().unwrap();
    let client_id = rand_client_id();

    let netcode = NetcodeClient::new(
        Authentication::Manual {
            server_addr,
            client_id,
            private_key: Key::default(),
            protocol_id: 0,
        },
        NetcodeConfig::default(),
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
    cli: Res<Cli>,
    mut senders: Query<&mut MessageSender<ClientJoinMessage>>,
    mut sent: Local<bool>,
) {
    if *sent { return; }
    for mut sender in senders.iter_mut() {
        let name = format!("Rig-{}", rand_client_id() % 1000);
        let msg = ClientJoinMessage { name: name.clone(), is_observer: cli.observer };
        info!("🤝 Dispatching ClientJoinMessage (Observer: {})...", cli.observer);
        let _ = sender.send::<ReliableChannel>(msg);
        *sent = true;
    }
}

// ─── Network Handlers ────────────────────────────────────────────────────────

/// Receive ServerStartMessage → build GameState and spawn tile grid.
/// Mirrors OpenFront's message.type === "start" handler in ClientGameRunner.
fn handle_server_start(
    mut receivers: Query<&mut MessageReceiver<ServerStartMessage>>,
    mut ready:     ResMut<GameReady>,
    mut my_id:     ResMut<MyPlayerId>,
    mut grid:      ResMut<GridEntities>,
    mut cam_q:     Query<&mut Transform, With<Camera2d>>,
    mut hud_q:     Query<&mut Text, With<HudText>>,
    mut commands:  Commands,
    mut queue:     ResMut<TurnQueue>,
    mut pending:   ResMut<PendingTicks>,
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
            }

            // Center camera on map
            let cx = config::MAP_WIDTH as f32 * TILE_SIZE / 2.0;
            let cy = config::MAP_HEIGHT as f32 * TILE_SIZE / 2.0;
            if let Ok(mut cam) = cam_q.single_mut() {
                cam.translation = Vec3::new(cx, cy, 999.0);
            }

            // Spawn tile sprites
            for y in 0..game.map.height {
                for x in 0..game.map.width {
                    let owner = game.map.owner_id(x, y);
                    let color = color_for_owner(&game, owner);
                    let entity = commands.spawn((
                        Sprite {
                            color,
                            custom_size: Some(Vec2::new(TILE_SIZE - 0.5, TILE_SIZE - 0.5)),
                            ..default()
                        },
                        Transform::from_xyz(x as f32 * TILE_SIZE, y as f32 * TILE_SIZE, 0.0),
                        TileNode { x, y },
                    )).id();
                    grid.0.insert((x, y), entity);
                }
            }

            commands.insert_resource(game);
            ready.0 = true;

            if let Ok(mut text) = hud_q.single_mut() {
                if let Some(pid) = msg.my_player_id {
                    **text = format!("Connected — Player {}", pid);
                } else {
                    **text = "Connected — Observer".to_string();
                }
            }
        }
    }
}

/// Receive ServerTurnMessage → apply each StampedIntent locally.
/// Mirrors OpenFront: message.type === "turn" → worker.sendTurn(turn)
///           → Executor.createExecs() per intent → AttackExecution.tick().
fn handle_server_turns(
    mut receivers: Query<&mut MessageReceiver<ServerTurnMessage>>,
    ready:         Res<GameReady>,
    mut queue:     ResMut<TurnQueue>,
    mut pending:   ResMut<PendingTicks>,
) {
    if !ready.0 { return; }

    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            queue.0.push_back(msg.turn);
            pending.0 += 1;
        }
    }
}

// ─── Input ────────────────────────────────────────────────────────────────────

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
    game_opt:    Option<Res<GameState>>,
    grid:        Res<GridEntities>,
    mut sprites: Query<&mut Sprite>,
) {
    let Some(game) = game_opt else { return };
    if !game.is_changed() { return; }

    for ev in &game.events {
        if let GameEvent::TileCaptured { x, y, new_owner } = ev {
            if let Some(&entity) = grid.0.get(&(*x, *y)) {
                if let Ok(mut sprite) = sprites.get_mut(entity) {
                    sprite.color = color_for_owner(&game, *new_owner);
                }
            }
        }
    }
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
    **text = lines.join("\n");
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn color_for_owner(game: &GameState, owner: u16) -> Color {
    if owner == 0 {
        Color::srgb(0.08, 0.08, 0.12)
    } else {
        game.players.iter()
            .find(|p| p.id == owner)
            .map(|p| Color::srgb(p.color[0], p.color[1], p.color[2]))
            .unwrap_or(Color::srgb(0.3, 0.3, 0.3))
    }
}

fn rand_client_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    nanos as u64
}
