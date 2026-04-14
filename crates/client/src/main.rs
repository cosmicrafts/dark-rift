//! Dark Rift Client — Networked (Lightyear WebSocket + Netcode)
//!
//! Flow:
//!   1. Spawn a Client entity: NetcodeClient + WebSocketClientIo → ws://127.0.0.1:5000
//!   2. Trigger Connect to start handshake
//!   3. MessageReceiver<ServerStartMessage>: init local GameState, spawn tile grid
//!   4. On player click: send ClientGameplayMessage via MessageSender
//!   5. MessageReceiver<ServerTurnMessage>: apply intents via apply_stamped_intent()
//!   6. Shared ECS systems run locally (system_player_income, system_attack_execution)
//!   7. Tile visuals updated from game.events (event-driven, O(events) not O(map))

use bevy::prelude::*;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::input::touch::Touches;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::text::FontWeight;
use bevy::mesh::prelude::{Mesh2d, Meshable};
use bevy_lunex::prelude::*;
use bevy_lunex::{NoLunexPicking, RecomputeUiLayout};
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d, TileData, TilemapChunk, TilemapChunkTileData};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::{Bloom, BloomCompositeMode};
use bevy_enoki::prelude::*;
use std::collections::{HashMap, HashSet};
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
use std::time::{Duration, Instant};
use clap::Parser;

use dark_rift_shared::config;
use dark_rift_shared::game::{GameState, GameEvent};
use dark_rift_shared::player::{bot_territory_color, Player};
use dark_rift_shared::execution::{
    system_attack_execution, system_player_income, system_game_tick, AttackExecution,
};
use dark_rift_shared::intent::{apply_stamped_intent, system_bot_think};
use dark_rift_shared::protocol::*;
use dark_rift_shared::pseudo_random::PseudoRandom;

mod settings;
mod ui_lunex_hud;
mod ui_theme;
use settings::*;
use ui_lunex_hud::*;
use ui_theme::*;

const TILE_SIZE: f32 = 8.0;

/// Perf overlay: refresh FPS readout at most ~4×/s (avoids jittery every-frame text).
const PERF_OVERLAY_FPS_INTERVAL: Duration = Duration::from_millis(250);
/// Latency / turn-cadence line: update ~every 1.5s (handshake + EMA are cheap; layout is not).
const PERF_OVERLAY_NET_INTERVAL: Duration = Duration::from_millis(1600);

/// Simulation ticks applied per Bevy frame while draining `PendingTicks`.
/// Previously up to 100/frame caused multi‑second stalls and huge `GameEvent` batches.
const CLIENT_CATCHUP_MAX_TICKS_PER_FRAME: u32 = 8;

// ─── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource, Default)]
struct GameReady(bool);

#[derive(Resource, Default)]
pub(crate) struct MyPlayerId(pub u16);

#[derive(Resource, Default)]
pub(crate) struct CameraDragState {
    pub accumulated_drag: f32,
    pub is_dragging: bool,
}

#[derive(Resource, Default)]
struct HoveredTile(Option<(u32, u32)>);

#[derive(Resource, Default)]
struct PendingTicks(u32);

#[derive(Resource, Default)]
struct TurnQueue(std::collections::VecDeque<Turn>);

// ─── Components ─────────────────────────────────────────────────────────────

// Custom grid hashmap component track mapping Entity ID to spatial location.

#[derive(Resource)]
struct OverlayDiagnostics {
    status_line: String,
    fps_value: String,
    ping_value: String,
}

impl Default for OverlayDiagnostics {
    fn default() -> Self {
        Self {
            status_line: "Connecting to server...".to_string(),
            fps_value: "…".to_string(),
            ping_value: "…".to_string(),
        }
    }
}

/// Throttles perf overlay text updates (FPS vs ping lines at different rates).
#[derive(Default)]
struct PerfOverlayUiState {
    last_fps_refresh: Option<Instant>,
    last_net_refresh: Option<Instant>,
}

/// Client-side network timing: handshake (Connect → first `ServerStartMessage`) and EMA spacing
/// between `ServerTurnMessage` arrivals (rough “server bundle” cadence vs load).
#[derive(Resource, Default)]
struct NetTiming {
    connect_started: Option<Instant>,
    handshake_ms: Option<f32>,
    last_turn_at: Option<Instant>,
    turn_spacing_ema_ms: f32,
}

#[derive(Component)]
pub(crate) struct PlayerLabel(pub u16);

/// Last applied `TextFont.font_size` for this label; avoids redundant layout when unchanged.
#[derive(Component)]
struct PlayerLabelRenderCache(f32);

/// Ephemeral “command ping” mesh when issuing an attack (player-colored, pop-in scale).
#[derive(Component)]
struct AttackCommandRing {
    elapsed: f32,
    /// Territory RGB for the ring tint (matches `Player::color`).
    rgb: [f32; 3],
}

#[derive(Resource)]
struct AttackRingMesh(Handle<Mesh>);

#[derive(Resource, Default)]
struct FreshlyPaintedTiles {
    tiles: HashMap<(u32, u32), f32>,
}

/// Troop fraction committed on each attack click (OpenFront `uiState.attackRatio` + ControlPanel slider 1–100%).
#[derive(Resource)]
struct AttackRatio(f32);

impl Default for AttackRatio {
    fn default() -> Self {
        Self(0.2)
    }
}

#[derive(Resource, Default)]
struct CombatVfxCache(Handle<Particle2dEffect>);

/// An independent entity that tracks an attack and emits Enoki particles
#[derive(Component)]
struct CombatVfx {
    target_exec: Entity,
}

#[derive(Resource, Default)]
struct OutgoingAttackRowMap(std::collections::HashMap<u64, (Entity, Entity)>);

#[derive(Parser, Resource)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Start as an observer
    #[arg(short, long)]
    observer: bool,

    /// Play a replay file locally
    #[arg(short, long)]
    replay: Option<String>,

    /// Server address
    #[arg(short, long, default_value = "127.0.0.1:5005")]
    server: String,
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
    .add_plugins(UiLunexPlugins)
    .add_plugins(FrameTimeDiagnosticsPlugin::default())
    .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.04)))
    .add_systems(Startup, setup_attack_ring_mesh)
    .insert_resource(cli)
    .init_resource::<GameReady>()
    .init_resource::<MyPlayerId>()
    .init_resource::<HoveredTile>()
    .init_resource::<PendingTicks>()
    .init_resource::<TurnQueue>()
    .init_resource::<HighestReceivedTurn>()
    .init_resource::<CameraDragState>()
    .init_resource::<FreshlyPaintedTiles>()
    .init_resource::<AttackRatio>()
    .init_resource::<OutgoingAttackRowMap>()
    .init_resource::<NetTiming>()
    .init_resource::<OverlayDiagnostics>()
    .init_resource::<UiShellState>();

    if let Some(path) = is_replay {
        let file = std::fs::read_to_string(&path)
            .expect("Failed to read replay file");
        let record: ReplayFile = serde_json::from_str(&file)
            .expect("Failed to parse Replay json schema");
            
        app.insert_non_send_resource(ReplayDirector {
            record,
            turn_index: 0,
        });

        app.add_systems(
            Startup,
            (
                load_client_settings,
                init_attack_ratio_from_ui_settings,
                setup_offline,
            )
                .chain(),
        )
           .add_systems(Update, system_replay_pulse);
    } else {
        app.add_plugins(SharedProtocolPlugin)
           .add_plugins(ClientPlugins::default())
           .add_plugins(EnokiPlugin)
           .init_resource::<CombatVfxCache>()
            .add_systems(
                Startup,
                (
                    load_client_settings,
                    init_attack_ratio_from_ui_settings,
                    setup_live,
                    setup_combat_vfx,
                )
                    .chain(),
            )
           .add_systems(Update, (
               handle_server_start,
               handle_server_turns,
               handle_connection,
           ).chain());
    }

    app.add_systems(Update, (system_update_ui_shell_state, system_update_perf_overlay));

    app.add_systems(Update, (
        hover_system,
        system_attack_ratio_keyboard,
        system_attack_ratio_slider_drag,
        player_click,
        handle_tile_change,
        system_animate_tile_colors,
        update_hud,
        system_apply_responsive_lunex_hud,
        system_sync_bottom_combat_hud,
        system_sync_outgoing_attack_rows,
    ).run_if(game_is_ready))
    .add_systems(
        Update,
        system_animate_attack_command_rings
            .after(player_click)
            .run_if(game_is_ready),
    );
    
    app.add_systems(Update, (
        camera_movement_and_zoom,
        system_spawn_attack_vfx.run_if(|settings: Res<ClientSettings>| settings.config.visuals.enable_combat_particles),
        system_sync_attack_vfx_position.run_if(|settings: Res<ClientSettings>| settings.config.visuals.enable_combat_particles),
        system_sync_player_labels.run_if(|settings: Res<ClientSettings>| settings.config.visuals.enable_floating_labels),
    ).run_if(game_is_ready));

    app.add_systems(Update, clear_inputs_on_focus_loss);

    let mut sim_schedule = Schedule::new(SimulationSchedule);
    sim_schedule.add_systems((
        pop_turn,
        system_player_income,
        system_bot_think,
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
    
    let process_count = pending
        .min(CLIENT_CATCHUP_MAX_TICKS_PER_FRAME)
        .max(1) as usize;
    
    for _ in 0..process_count {
        if world.resource::<PendingTicks>().0 == 0 { break; }
        let _ = world.try_run_schedule(SimulationSchedule);
    }
}

fn game_is_ready(ready: Res<GameReady>) -> bool { ready.0 }

// Removed has_pending_ticks as it is obsolete

fn pop_turn(
    mut commands: Commands,
    mut game: ResMut<GameState>,
    mut queue: ResMut<TurnQueue>,
    mut q_exec: Query<(Entity, &mut AttackExecution)>,
) {
    if let Some(turn) = queue.0.pop_front() {
        for (i, stamped) in turn.intents.iter().enumerate() {
            apply_stamped_intent(
                &mut commands,
                &mut *game,
                &mut q_exec,
                stamped,
                i as u32,
            );
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
    mouse_input: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    hover_map: Res<HoverMap>,
    hud_block: Query<(), With<HudBlocksMapClicks>>,
    mut scroll_evr: MessageReader<MouseWheel>,
    mut motion_evr: MessageReader<bevy::input::mouse::MouseMotion>,
    mut drag_state: ResMut<CameraDragState>,
) {
    let press_on_hud = mouse_input.just_pressed(MouseButton::Left)
        && hover_map_has_blocker(&hover_map, PointerId::Mouse, &hud_block);
    let touch_on_hud = touches.iter().any(|t| {
        touches.just_pressed(t.id())
            && hover_map_has_blocker(&hover_map, PointerId::Touch(t.id()), &hud_block)
    });
    if (mouse_input.just_pressed(MouseButton::Left) || touches.any_just_pressed())
        && !press_on_hud
        && !touch_on_hud
    {
        drag_state.is_dragging = true;
        drag_state.accumulated_drag = 0.0;
    }
    if mouse_input.just_released(MouseButton::Left) || touches.any_just_released() {
        drag_state.is_dragging = false;
    }

    // Touch dragging (if available) - calculate average delta
    let mut touch_delta = Vec2::ZERO;
    let mut touch_count = 0;
    for touch in touches.iter() {
        touch_delta += touch.delta();
        touch_count += 1;
    }
    if touch_count > 0 {
        touch_delta /= touch_count as f32;
    }

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
        
        // Mouse/Touch Drag Pan
        let mut drag_delta = Vec2::ZERO;
        
        if drag_state.is_dragging {
            // Process Mouse
            for ev in motion_evr.read() {
                drag_delta += ev.delta;
            }
            // Add Touch if mouse delta is null
            if drag_delta == Vec2::ZERO && touch_delta != Vec2::ZERO {
                drag_delta = touch_delta;
            }
            
            drag_state.accumulated_drag += drag_delta.length();
            
            if drag_delta != Vec2::ZERO {
                // Screen coordinates: +X is right, +Y is down
                // World coordinates: +X is right, +Y is up
                // So dragging mouse Right: delta.x > 0 -> Move camera Left (subtract)
                // Dragging mouse Down: delta.y > 0 -> Move camera Up (add)
                pan.x -= drag_delta.x * ortho.scale;
                pan.y += drag_delta.y * ortho.scale;
            }
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
    settings: Res<ClientSettings>,
    mut overlay: ResMut<OverlayDiagnostics>,
) {
    let cam = commands
        .spawn((
            Camera2d,
            Tonemapping::TonyMcMapface,
            Bloom {
                intensity: 0.15,
                composite_mode: BloomCompositeMode::Additive,
                ..default()
            },
            UiSourceCamera::<0>,
            Transform::from_xyz(
                config::MAP_WIDTH as f32 * TILE_SIZE / 2.0,
                config::MAP_HEIGHT as f32 * TILE_SIZE / 2.0,
                999.0,
            ),
        ))
        .id();

    overlay.status_line = "Replay Mode".to_string();
    overlay.ping_value = "offline".to_string();

    let hud = spawn_lunex_hud_offline(
        &mut commands,
        cam,
        &asset_server,
        settings.font.clone(),
        "Replay Mode\nTick: 0\nFPS: …\nPing: offline",
    );
    commands.insert_resource(hud);

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
        Transform::from_xyz(
            config::MAP_WIDTH  as f32 * TILE_SIZE / 2.0,
            config::MAP_HEIGHT as f32 * TILE_SIZE / 2.0,
            0.0
        ),
    ));

    commands.insert_resource(game);
    ready.0 = true;
}

fn setup_live(
    mut commands: Commands,
    cli: Res<Cli>,
    settings: Res<ClientSettings>,
    asset_server: Res<AssetServer>,
    mut overlay: ResMut<OverlayDiagnostics>,
) {
    let mut cam_ec = commands.spawn((
        Camera2d,
        Tonemapping::TonyMcMapface,
        UiSourceCamera::<0>,
        Transform::from_xyz(0.0, 0.0, 999.0),
    ));

    if settings.config.visuals.hdr_bloom_enabled() {
        cam_ec.insert(Bloom {
            intensity: 0.15,
            composite_mode: BloomCompositeMode::Additive,
            ..default()
        });
    }
    let cam = cam_ec.id();

    overlay.status_line = "Connecting to server...".to_string();
    overlay.ping_value = "connecting…".to_string();

    let hud = spawn_lunex_hud_live(
        &mut commands,
        cam,
        &asset_server,
        settings.font.clone(),
        "Connecting to server...\nTick: 0\nFPS: …\nPing: connecting…",
    );
    commands.insert_resource(hud);

    let server_addr: SocketAddr = cli.server.parse().expect("Invalid server address format");
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

    commands.insert_resource(NetTiming {
        connect_started: Some(Instant::now()),
        ..default()
    });

    // Trigger Netcode connection (which internally initiates LinkStart)
    commands.trigger(lightyear::prelude::client::Connect { entity: client_entity });

    info!("🔌 Connecting to ws://{}... (client_id={}), observer={}", cli.server, client_id, cli.observer);
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
    let tick_len = 0.040; // 40ms turn interval
    
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

fn init_attack_ratio_from_ui_settings(ui: Res<ClientSettings>, mut ratio: ResMut<AttackRatio>) {
    ratio.0 = ui.config.ui.attack_ratio.clamp(0.01, 1.0);
}

fn system_update_perf_overlay(
    diagnostics: Res<DiagnosticsStore>,
    net: Res<NetTiming>,
    cli: Res<Cli>,
    mut overlay: ResMut<OverlayDiagnostics>,
    mut throttle: Local<PerfOverlayUiState>,
) {
    let now = Instant::now();

    let refresh_fps = throttle
        .last_fps_refresh
        .map(|t| now.duration_since(t) >= PERF_OVERLAY_FPS_INTERVAL)
        .unwrap_or(true);
    let refresh_net = throttle
        .last_net_refresh
        .map(|t| now.duration_since(t) >= PERF_OVERLAY_NET_INTERVAL)
        .unwrap_or(true);

    if refresh_fps {
        throttle.last_fps_refresh = Some(now);
        overlay.fps_value = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|v| v.value())
            .map(|f| format!("{f:.1}"))
            .unwrap_or_else(|| "—".to_string());
    }

    if refresh_net {
        throttle.last_net_refresh = Some(now);
        overlay.ping_value = if cli.replay.is_some() {
            "offline".to_string()
        } else if net.handshake_ms.is_none() && net.connect_started.is_some() {
            "connecting…".to_string()
        } else if let Some(hs) = net.handshake_ms {
            if net.turn_spacing_ema_ms > 1.0 {
                format!("{hs:.0} ms hs · ~{:.0} ms/turn", net.turn_spacing_ema_ms)
            } else {
                format!("{hs:.0} ms hs")
            }
        } else {
            "…".to_string()
        };
    }

    if !(refresh_fps || refresh_net) {
        return;
    }
}

/// Receive ServerStartMessage → build GameState and spawn tile grid.
/// Mirrors OpenFront's message.type === "start" handler in ClientGameRunner.
fn handle_server_start(
    mut receivers: Query<&mut MessageReceiver<ServerStartMessage>>,
    mut ready:     ResMut<GameReady>,
    mut my_id:     ResMut<MyPlayerId>,
    mut cam_q:     Query<&mut Transform, With<Camera2d>>,
    mut commands:  Commands,
    mut queue:     ResMut<TurnQueue>,
    mut pending:   ResMut<PendingTicks>,
    mut highest:   ResMut<HighestReceivedTurn>,
    mut net:       ResMut<NetTiming>,
    mut overlay:   ResMut<OverlayDiagnostics>,
    lunex_root:    Res<LunexHudRoot>,
    asset_server:  Res<AssetServer>,
    settings:      Res<ClientSettings>,
) {
    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            if ready.0 { continue; }

            if net.handshake_ms.is_none() {
                if let Some(t0) = net.connect_started.take() {
                    net.handshake_ms = Some(t0.elapsed().as_secs_f32() * 1000.0);
                }
            }

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

            // Spawn deterministic bots (Tribes)
            let mut rng = PseudoRandom::new(msg.seed);
            for i in 0..config::BOT_COUNT {
                let bot_id = 100 + i as u16;
                let mut tries = 0;
                let (mut sx, mut sy) = (0, 0);
                
                while tries < 1000 {
                    sx = rng.next_int(0, config::MAP_WIDTH as i32) as u32;
                    sy = rng.next_int(0, config::MAP_HEIGHT as i32) as u32;
                    let mut valid = true;
                    
                    // Check radius 15 clearance from map edge and any owned tiles
                    for dy in -15..=15 {
                        for dx in -15..=15 {
                            let nx = sx as i32 + dx;
                            let ny = sy as i32 + dy;
                            if !game.map.is_valid_coord(nx, ny) || game.map.owner_id(nx as u32, ny as u32) != 0 {
                                valid = false;
                                break;
                            }
                        }
                        if !valid { break; }
                    }
                    if valid { break; }
                    tries += 1;
                }

                if tries < 1000 {
                    let player = Player::new_bot(
                        bot_id,
                        format!("Tribe {}", i + 1),
                        bot_territory_color(msg.seed, bot_id),
                    );
                    game.spawn_player(player, sx, sy);
                    info!("🤖 Spawned Tribe {} at {},{}", bot_id, sx, sy);
                } else {
                    warn!("⚠️ Failed to spawn Tribe {} (no space)", bot_id);
                }
            }

            // Replay missed turns (late-join) — identical behavior to OpenFront
            for turn in &msg.missed_turns {
                queue.0.push_back(turn.clone());
                pending.0 += 1;
                highest.0 = Some(highest.0.unwrap_or(0).max(turn.turn_number));
            }

            // Center camera (on the player if playing, else map center)
            let mut cx = config::MAP_WIDTH as f32 * TILE_SIZE / 2.0;
            let mut cy = config::MAP_HEIGHT as f32 * TILE_SIZE / 2.0;

            if my_id.0 != 0 {
                if let Some(me) = msg.players.iter().find(|p| p.id == my_id.0) {
                    cx = me.spawn_x as f32 * TILE_SIZE;
                    cy = me.spawn_y as f32 * TILE_SIZE;
                }
            }

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
                Transform::from_xyz(
                    config::MAP_WIDTH  as f32 * TILE_SIZE / 2.0,
                    config::MAP_HEIGHT as f32 * TILE_SIZE / 2.0,
                    0.0
                ),
            ));

            commands.insert_resource(game);
            ready.0 = true;

            spawn_lunex_combat_hud(
                &mut commands,
                lunex_root.0,
                &asset_server,
                settings.font.clone(),
                msg.my_player_id.is_none(),
            );

            if msg.my_player_id.is_some() {
                overlay.status_line = format!("Connected — Player {:?}", msg.my_player_id);
            } else {
                overlay.status_line = "Connected — Observer".to_string();
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
    mut net:       ResMut<NetTiming>,
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

            let now = Instant::now();
            if let Some(prev) = net.last_turn_at {
                let dt_ms = now.duration_since(prev).as_secs_f32() * 1000.0;
                if net.turn_spacing_ema_ms <= 0.0 {
                    net.turn_spacing_ema_ms = dt_ms;
                } else {
                    net.turn_spacing_ema_ms = net.turn_spacing_ema_ms * 0.92 + dt_ms * 0.08;
                }
            }
            net.last_turn_at = Some(now);

            queue.0.push_back(msg.turn);
            pending.0 += 1;
        }
    }
}

// ─── Input ────────────────────────────────────────────────────────────────────

fn setup_attack_ring_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let mesh = Annulus::new(5.0, 8.8)
        .mesh()
        .resolution(56)
        .build();
    commands.insert_resource(AttackRingMesh(meshes.add(mesh)));
}

/// Overshoot ease 0→1 in `t ∈ [0,1]` (landing exactly at 1).
fn ease_out_back_scaled(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.701_58_f32;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

fn clear_inputs_on_focus_loss(
    mut events: MessageReader<bevy::window::WindowFocused>,
    mut kb:     ResMut<ButtonInput<KeyCode>>,
) {
    for ev in events.read() {
        if !ev.focused {
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
    mut commands: Commands,
    ring_mesh: Res<AttackRingMesh>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    drag_state: Res<CameraDragState>,
    hovered: Res<HoveredTile>,
    game_opt: Option<Res<GameState>>,
    my_id: Res<MyPlayerId>,
    attack_ratio: Res<AttackRatio>,
    hover_map: Res<HoverMap>,
    hud_block: Query<(), With<HudBlocksMapClicks>>,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
) {
    // We trigger ONLY on release to support dragging.
    // Additionally, if the user accumulated too much drag, we abort the intent payload.
    if !mouse.just_released(MouseButton::Left) && !touches.any_just_released() {
        return;
    }
    if drag_state.accumulated_drag > 10.0 {
        return;
    } // It was a drag, not a click

    // Clicks/releases over the Lunex HUD must not issue map attacks (OpenFront: UI eats pointer).
    if mouse.just_released(MouseButton::Left) {
        if hover_map_has_blocker(&hover_map, PointerId::Mouse, &hud_block) {
            return;
        }
    } else if touches.any_just_released()
        && touches.iter_just_released().any(|t| {
            hover_map_has_blocker(&hover_map, PointerId::Touch(t.id()), &hud_block)
        })
    {
        return;
    }

    let Some((x, y)) = hovered.0 else { return };
    let Some(game) = game_opt else { return };

    let pid = my_id.0;
    let Some(player) = game.players.iter().find(|p| p.id == pid) else { return };
    if !player.alive || player.troops < config::EXPAND_COST { return; }

    let target_owner = game.map.owner_id(x, y);
    if target_owner == pid { return; }

    let wx = (x as f32 + 0.5) * TILE_SIZE;
    let wy = (y as f32 + 0.5) * TILE_SIZE;
    let [r, g, b] = player.color;
    let mat = materials.add(ColorMaterial::from_color(Color::srgba(r, g, b, 0.97)));
    commands.spawn((
        AttackCommandRing {
            elapsed: 0.0,
            rgb: player.color,
        },
        Mesh2d(ring_mesh.0.clone()),
        MeshMaterial2d(mat),
        Transform::from_xyz(wx, wy, 260.0).with_scale(Vec3::ZERO),
    ));

    let ratio = attack_ratio.0.clamp(0.01, 1.0) as f64;
    let send_floor = (player.troops * ratio).floor();
    let send = send_floor.max(0.0).min(player.troops);
    if send < config::EXPAND_COST {
        return;
    }

    for mut sender in senders.iter_mut() {
        let _ = sender.send::<ReliableChannel>(ClientGameplayMessage {
            intent: GameplayIntent::Attack(AttackIntent {
                target_owner,
                troops: Some(send),
            }),
        });
    }
}

/// Quick 0→full scale pop (rest of lifetime = settle + fade).
const ATTACK_RING_POP_SECS: f32 = 0.1;
const ATTACK_COMMAND_RING_LIFETIME: f32 = 0.38;

fn system_animate_attack_command_rings(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(
        Entity,
        &mut AttackCommandRing,
        &mut Transform,
        &MeshMaterial2d<ColorMaterial>,
    )>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let dt = time.delta_secs();
    for (entity, mut ring, mut transform, mat_h) in &mut q {
        ring.elapsed += dt;
        let t = ring.elapsed;
        if t >= ATTACK_COMMAND_RING_LIFETIME {
            commands.entity(entity).despawn();
            continue;
        }

        let life = t / ATTACK_COMMAND_RING_LIFETIME;
        let damp = (1.0 - life).clamp(0.0, 1.0);

        let pop_u = (t / ATTACK_RING_POP_SECS).min(1.0);
        let pop = ease_out_back_scaled(pop_u);

        let wobble = 1.0 + 0.055 * damp * (t * std::f32::consts::TAU * 4.0).sin();
        transform.scale = Vec3::splat(pop * wobble);

        let fade = damp.powf(1.15);
        let [r, g, b] = ring.rgb;
        if let Some(mat) = materials.get_mut(&mat_h.0) {
            mat.color = Color::srgba(r, g, b, 0.97 * fade);
        }
    }
}

// ─── Visual ──────────────────────────────────────────────────────────────────

fn handle_tile_change(
    game_opt:    Option<ResMut<GameState>>,
    mut fresh_tiles: ResMut<FreshlyPaintedTiles>,
    mut chunks:  Query<&mut TilemapChunkTileData>,
) {
    let Some(mut game) = game_opt else { return };
    if !game.is_changed() { return; }

    for ev in &game.events {
        if let GameEvent::TileCaptured { x, y, new_owner: _ } = ev {
            // Push white flash maximum lerp state
            fresh_tiles.tiles.insert((*x, *y), 1.0);

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
                        let mut t_data = openfront_tile_color(&game, cx, cy);
                        
                        // Boost freshly-painted tiles with their own core color (not white)
                        if let Some(&val) = fresh_tiles.tiles.get(&(cx as u32, cy as u32)) {
                            let srgba = t_data.color.to_srgba();
                            let t = val.clamp(0.0, 1.0);
                            let eased = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t); // cubic ease-out
                            let boost = 1.0 + 4.0 * eased; // multiply core color up to 5x for HDR bloom
                            
                            let owner = game.map.owner_id(cx as u32, cy as u32);
                            let is_border = [(cx, cy - 1), (cx + 1, cy), (cx, cy + 1), (cx - 1, cy)]
                                .iter()
                                .any(|&(nx, ny)| {
                                    game.map.is_valid_coord(nx, ny)
                                        && game.map.owner_id(nx as u32, ny as u32) != owner
                                });

                            let white_flash = if is_border { 2.5 * eased } else { 0.0 };

                            t_data.color = Color::srgba(
                                srgba.red * boost + white_flash,
                                srgba.green * boost + white_flash,
                                srgba.blue * boost + white_flash,
                                srgba.alpha
                            );
                        }

                        chunk_data[idx] = Some(t_data);
                    }
                }
            }
        }
    }
    
    // Clear out events so we effectively operate visually in 'Batched' chunks per-frame independently of tick iterations
    game.events.clear();
}

fn system_animate_tile_colors(
    time: Res<Time>,
    game_opt: Option<Res<GameState>>,
    mut fresh_tiles: ResMut<FreshlyPaintedTiles>,
    mut chunks: Query<&mut TilemapChunkTileData>,
    settings: Res<ClientSettings>,
) {
    if !settings.config.visuals.territory_expansion_animations_enabled() {
        fresh_tiles.tiles.clear();
        return;
    }
    if fresh_tiles.tiles.is_empty() { return; }
    let Some(game) = game_opt else { return };
    
    let dt = time.delta_secs();
    let lerp_speed = 0.85; // Lowered from 3.33 -> 0.85 to leave a much longer "burning" tail over the territory
    let mut finished = vec![];
    
    for (&(x, y), val) in fresh_tiles.tiles.iter_mut() {
        *val -= lerp_speed * dt;
        if *val <= 0.0 {
            *val = 0.0;
            finished.push((x, y));
        }
    }
    
    for mut chunk_data in chunks.iter_mut() {
        for (&(x, y), val) in fresh_tiles.tiles.iter() {
            let cx = x as i32;
            let cy = y as i32;
            let mut t_data = openfront_tile_color(&game, cx, cy);
            
            let srgba = t_data.color.to_srgba();
            let t = val.clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t); // cubic ease-out
            let boost = 1.0 + 4.0 * eased; // multiply core color up to 5x for HDR bloom
            
            let owner = game.map.owner_id(cx as u32, cy as u32);
            let is_border = [(cx, cy - 1), (cx + 1, cy), (cx, cy + 1), (cx - 1, cy)]
                .iter()
                .any(|&(nx, ny)| {
                    game.map.is_valid_coord(nx, ny)
                        && game.map.owner_id(nx as u32, ny as u32) != owner
                });

            let white_flash = if is_border { 2.5 * eased } else { 0.0 };

            t_data.color = Color::srgba(
                srgba.red * boost + white_flash,
                srgba.green * boost + white_flash,
                srgba.blue * boost + white_flash,
                srgba.alpha
            );
            
            let idx = (cy as u32 * config::MAP_WIDTH as u32 + cx as u32) as usize;
            chunk_data[idx] = Some(t_data);
        }
    }
    
    for key in finished {
        fresh_tiles.tiles.remove(&key);
    }
}

fn update_hud(
    game_opt: Option<Res<GameState>>,
    overlay: Res<OverlayDiagnostics>,
    mut hud_q: Query<&mut Text2d, With<HudText>>,
) {
    let tick = game_opt.map(|g| g.tick).unwrap_or(0);
    let Ok(mut text) = hud_q.single_mut() else {
        return;
    };

    let final_str = format!(
        "{}\nTick: {}\nFPS: {}\nPing: {}",
        overlay.status_line, tick, overlay.fps_value, overlay.ping_value
    );

    if text.0 != final_str {
        text.0 = final_str;
    }
}

/// OpenFront `InputHandler` KeyT / KeyY with `attackRatioIncrement` default 10 → ±0.10.
const ATTACK_RATIO_KEYSTEP: f32 = 0.10;

fn system_attack_ratio_keyboard(
    kb: Res<ButtonInput<KeyCode>>,
    my_id: Res<MyPlayerId>,
    mut ratio: ResMut<AttackRatio>,
) {
    if my_id.0 == 0 {
        return;
    }
    if kb.just_pressed(KeyCode::KeyT) {
        ratio.0 = (ratio.0 - ATTACK_RATIO_KEYSTEP).max(0.01);
    }
    if kb.just_pressed(KeyCode::KeyY) {
        ratio.0 = (ratio.0 + ATTACK_RATIO_KEYSTEP).min(1.0);
    }
}

/// OpenFront `ControlPanel.handleRatioSliderInput`: map pointer X along rail to 1–100 %.
///
/// Updates while the pointer is held, and **once on release** while over the track so the final
/// position applies even though `pressed` is false that frame (and so `player_click` sees the new ratio).
fn slider_cursor_to_ratio(cursor_world: Vec2, track_gt: &GlobalTransform, dim: &Dimension) -> Option<f32> {
    let inv = track_gt.affine().inverse();
    let local = inv.transform_point3(cursor_world.extend(0.0)).xy();
    let rect = Rect::from_center_size(Vec2::ZERO, **dim);
    if !rect.contains(local) {
        return None;
    }
    let w = dim.x.max(1.0);
    let t = (local.x / w + 0.5).clamp(0.0, 1.0);
    Some((0.01 + t * 0.99).clamp(0.01, 1.0))
}

fn system_attack_ratio_slider_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut ratio: ResMut<AttackRatio>,
    my_id: Res<MyPlayerId>,
    q_track: Query<(&GlobalTransform, &Dimension), With<AttackRatioTrackHit>>,
    mut commands: Commands,
) {
    if my_id.0 == 0 {
        return;
    }
    let dragging =
        mouse.pressed(MouseButton::Left) || touches.iter().next().is_some();
    let released = mouse.just_released(MouseButton::Left) || touches.any_just_released();
    if !dragging && !released {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((cam, cam_gt)) = camera_q.single() else {
        return;
    };
    let Ok((track_gt, dim)) = q_track.single() else {
        return;
    };

    let mut changed = false;
    let mut apply_screen = |screen: Vec2| {
        if let Ok(world) = cam.viewport_to_world_2d(cam_gt, screen) {
            if let Some(r) = slider_cursor_to_ratio(world, track_gt, dim) {
                ratio.0 = r;
                changed = true;
            }
        }
    };

    if mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left) {
        if let Some(p) = window.cursor_position() {
            apply_screen(p);
        }
    }
    for finger in touches.iter() {
        apply_screen(finger.position());
    }
    for finger in touches.iter_just_released() {
        apply_screen(finger.position());
    }

    if changed {
        commands.trigger(RecomputeUiLayout);
    }
}

fn hud_trunc_name(s: &str, max_ch: usize) -> String {
    let count = s.chars().count();
    if count <= max_ch {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_ch).collect::<String>())
    }
}

/// Incoming attacks + tips. Outgoing rows live in [`OutgoingRowsContainer`] (cápsulas + cancel).
fn system_sync_bottom_combat_hud(
    game_opt: Option<Res<GameState>>,
    my_id: Res<MyPlayerId>,
    attack_ratio: Res<AttackRatio>,
    q_exec: Query<&AttackExecution>,
    mut q_summary: Query<&mut Text2d, (With<AttackSummaryText>, Without<AttackRatioReadout>)>,
    mut q_readout: Query<&mut Text2d, (With<AttackRatioReadout>, Without<AttackSummaryText>)>,
    mut q_fill: Query<&mut UiLayout, With<AttackRatioFill>>,
    mut commands: Commands,
    mut last_fill_ratio: Local<Option<f32>>,
) {
    let Some(game) = game_opt else { return };
    let (Ok(mut summary), Ok(mut readout), Ok(mut fill)) = (
        q_summary.single_mut(),
        q_readout.single_mut(),
        q_fill.single_mut(),
    ) else {
        return;
    };

    let r = attack_ratio.0.clamp(0.01, 1.0);
    let fill_changed = last_fill_ratio.map(|p| (p - r).abs() > 1e-5).unwrap_or(true);
    if fill_changed {
        *last_fill_ratio = Some(r);
        if let Some(w) = fill.get_mut_window(UiBase::id()) {
            w.set_width(Rl(r * 100.0));
        }
    }

    if my_id.0 == 0 {
        summary.0.clear();
        readout.0.clear();
        if fill_changed {
            commands.trigger(RecomputeUiLayout);
        }
        return;
    }

    let preview_troops = game
        .player(my_id.0)
        .map(|p| (p.troops * r as f64).floor().max(0.0))
        .unwrap_or(0.0);
    let readout_str = format!("⚔ {:.0}% ({:.0})", r * 100.0, preview_troops);
    if readout.0 != readout_str {
        readout.0 = readout_str;
    }

    let mut incoming: Vec<&AttackExecution> = q_exec
        .iter()
        .filter(|e| {
            e.target_owner == my_id.0 && e.owner_id != my_id.0 && !e.retreating
        })
        .collect();
    incoming.sort_by_key(|e| e.id);

    fn target_name(game: &GameState, owner_id: u16) -> String {
        if owner_id == 0 {
            "Neutral".to_string()
        } else {
            game.player(owner_id)
                .map(|p| hud_trunc_name(&p.name, 16))
                .unwrap_or_else(|| format!("{}", owner_id))
        }
    }

    let mut lines = Vec::new();
    if !incoming.is_empty() {
        lines.push("↓ Te atacan".to_string());
        for e in &incoming {
            let who = target_name(&game, e.owner_id);
            lines.push(format!(
                "  ▸ {}  {:.0}/{:.0} trp  ·{} cola",
                who,
                e.troops.max(0.0),
                e.initial_troops.max(0.0),
                e.to_conquer.len()
            ));
        }
    } else {
        lines.push("Sin ataques entrantes.".to_string());
    }
    lines.push(String::new());
    lines.push("↑ Tus salientes: filas debajo (✕ cancelar).".to_string());
    lines.push("T/Y — subir/bajar %.".to_string());
    let body = lines.join("\n");

    if summary.0 != body {
        summary.0 = body;
    }

    if fill_changed {
        commands.trigger(RecomputeUiLayout);
    }
}

fn system_sync_outgoing_attack_rows(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    game_opt: Option<Res<GameState>>,
    slot_q: Query<Entity, With<OutgoingRowsContainer>>,
    q_exec: Query<&AttackExecution>,
    my_id: Res<MyPlayerId>,
    settings: Res<ClientSettings>,
    mut row_map: ResMut<OutgoingAttackRowMap>,
    mut q_line: Query<&mut Text2d, With<OutgoingAttackLineText>>,
    mut q_row_layout: Query<&mut UiLayout, With<OutgoingAttackRowRoot>>,
    mut last_row_sig: Local<Option<u64>>,
) {
    let Ok(slot) = slot_q.single() else {
        return;
    };

    if my_id.0 == 0 {
        for (_, (row_e, _)) in row_map.0.drain() {
            commands.entity(row_e).despawn();
        }
        return;
    }
    let Some(game) = game_opt else {
        return;
    };

    let mut outgoing: Vec<&AttackExecution> = q_exec
        .iter()
        .filter(|e| e.owner_id == my_id.0 && !e.retreating)
        .collect();
    outgoing.sort_by_key(|e| e.id);

    let desired: HashSet<u64> = outgoing.iter().map(|e| e.id).collect();

    let mut need_recompute = false;

    let stale: Vec<u64> = row_map
        .0
        .keys()
        .copied()
        .filter(|id| !desired.contains(id))
        .collect();
    for id in stale {
        if let Some((row_e, _)) = row_map.0.remove(&id) {
            commands.entity(row_e).despawn();
            need_recompute = true;
        }
    }

    let row_panel = asset_server.load::<Image>(UI_ASSET_PANEL_MENU);
    let cancel_img = asset_server.load::<Image>(UI_ASSET_BUTTON_SLICED);
    let font = settings.font.clone();

    for exec in &outgoing {
        let who = if exec.target_owner == 0 {
            "Neutral".to_string()
        } else {
            game
                .player(exec.target_owner)
                .map(|p| hud_trunc_name(&p.name, 14))
                .unwrap_or_else(|| format!("{}", exec.target_owner))
        };
        let line = format!(
            "↑ {} — {:.0} / {:.0} trp",
            who,
            exec.troops.max(0.0),
            exec.initial_troops.max(0.0),
        );

        if let Some((_, text_e)) = row_map.0.get(&exec.id).copied() {
            if let Ok(mut t) = q_line.get_mut(text_e) {
                if t.0 != line {
                    t.0 = line;
                }
            }
        } else {
            let exec_id = exec.id;
            let row_e = commands
                .spawn((
                    ChildOf(slot),
                    OutgoingAttackRowRoot,
                    UiLayout::window()
                        .anchor(Anchor::TopLeft)
                        .pos(Ab((8.0, 4.0)))
                        .size(Rl((98.0, 16.0)))
                        .pack(),
                    lunex_slice_sprite(row_panel.clone()),
                    UiColor::from(panel_bg().with_alpha(0.9)),
                    Pickable::default(),
                    HudBlocksMapClicks,
                    UiDepth::Add(1.0),
                ))
                .id();

            let text_e = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingAttackLineText,
                    UiLayout::window()
                        .anchor(Anchor::CenterLeft)
                        .pos(Rl((4.0, 50.0)))
                        .size(Rl((78.0, 75.0)))
                        .pack(),
                    UiTextSize::from(Rh(36.0)),
                    Text2d::new(line.clone()),
                    TextFont {
                        font: font.clone(),
                        font_size: 48.0,
                        weight: FontWeight::MEDIUM,
                        ..default()
                    },
                    TextColor(text_primary()),
                    NoLunexPicking,
                    UiDepth::Add(2.0),
                ))
                .id();

            commands
                .spawn((
                    ChildOf(row_e),
                    CancelAttackButton(exec_id),
                    UiLayout::window()
                        .anchor(Anchor::CenterRight)
                        .pos(Rl((97.0, 50.0)))
                        .size(Rw(7.5))
                        .pack(),
                    UiLayout::new(vec![
                        (
                            UiBase::id(),
                            UiLayout::window().anchor(Anchor::Center).full().pack(),
                        ),
                        (
                            UiHover::id(),
                            UiLayout::window()
                                .anchor(Anchor::Center)
                                .size(Rl((112.0, 112.0)))
                                .pack(),
                        ),
                    ]),
                    UiHover::new().forward_speed(18.0).backward_speed(14.0),
                    UiColor::new(vec![
                        (UiBase::id(), accent_danger()),
                        (UiHover::id(), accent_danger_border()),
                    ]),
                    lunex_slice_sprite(cancel_img.clone()),
                    Pickable::default(),
                    HudBlocksMapClicks,
                    UiDepth::Add(3.0),
                ))
                .observe(hover_set::<Pointer<Over>, true>)
                .observe(hover_set::<Pointer<Out>, false>)
                .observe(observe_cancel_attack)
                .with_children(|x| {
                    x.spawn((
                        UiLayout::window()
                            .anchor(Anchor::Center)
                            .size(Rh(40.0))
                            .pack(),
                        UiTextSize::from(Rh(36.0)),
                        Text2d::new("✕"),
                        TextFont {
                            font,
                            font_size: 56.0,
                            weight: FontWeight::BOLD,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        NoLunexPicking,
                        UiDepth::Add(2.0),
                    ));
                });

            row_map.0.insert(exec_id, (row_e, text_e));
            need_recompute = true;
        }
    }

    let mut ordered: Vec<u64> = outgoing.iter().map(|e| e.id).collect();
    ordered.sort_unstable();
    let sig = ordered.iter().fold(ordered.len() as u64, |acc, &id| {
        acc.wrapping_mul(31).wrapping_add(id)
    });
    if *last_row_sig != Some(sig) {
        *last_row_sig = Some(sig);
        let row_step = 17.0_f32;
        for (i, id) in ordered.iter().enumerate() {
            let Some((row_e, _)) = row_map.0.get(id) else {
                continue;
            };
            if let Ok(mut lay) = q_row_layout.get_mut(*row_e) {
                if let Some(w) = lay.get_mut_window(UiBase::id()) {
                    w.set_y(Ab(4.0 + i as f32 * row_step));
                }
            }
        }
        need_recompute = true;
    }

    if need_recompute {
        commands.trigger(RecomputeUiLayout);
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
/// - Border (any cardinal neighbor has different owner) → `base * TERRITORY_BORDER_RGB_MULTIPLIER`, alpha 1
/// - Interior → base color, alpha `config::TERRITORY_INTERIOR_ALPHA`
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
        let m = config::TERRITORY_BORDER_RGB_MULTIPLIER;
        Color::srgba(base[0] * m, base[1] * m, base[2] * m, 1.0)
    } else {
        Color::srgba(
            base[0],
            base[1],
            base[2],
            config::TERRITORY_INTERIOR_ALPHA,
        )
    };

    TileData { tileset_index: 0, color, ..default() }
}

fn rand_client_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    nanos as u64
}

// -------------------------------------------------------------
// UI: Floating Player Identifiers (Center of mass on map)
// -------------------------------------------------------------

fn player_label_scale(tiles_owned: u32, ref_tiles: f32, max_scale: f32) -> f32 {
    let t = tiles_owned.max(1) as f32;
    let r = ref_tiles.max(1.0);
    (t / r).sqrt().max(1.0).min(max_scale.max(1.0))
}

fn system_sync_player_labels(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    ui_opt: Option<Res<ClientSettings>>,

    mut q_labels: Query<(
        Entity,
        &mut Text2d,
        &mut TextFont,
        &mut Transform,
        &mut TextColor,
        &PlayerLabel,
        &mut PlayerLabelRenderCache,
    )>,
) {
    let Some(game) = game_opt else { return };
    let Some(ui) = ui_opt else { return };

    // 1. Despawn labels for dead players
    for (entity, _, _, _, _, label, _) in &q_labels {
        let alive = game.players.iter().any(|p| p.id == label.0 && p.alive);
        if !alive {
            commands.entity(entity).despawn();
        }
    }

    // 2. Spawn labels for alive players if they don't exist
    for player in game.players.iter().filter(|p| p.alive) {
        let exists = q_labels
            .iter()
            .any(|(_, _, _, _, _, label, _)| label.0 == player.id);
        if !exists {
            let color = Color::WHITE;
            let base_font = ui.config.ui.font_size;
            let tiles = game.map.tiles_owned_by(player.id);
            let mul = player_label_scale(
                tiles,
                ui.config.ui.label_size_ref_tiles,
                ui.config.ui.label_max_scale,
            );
            let font_px = (base_font * mul).round().max(1.0);

            let label_text = format!("{}\n[{:.0}]", player.name, player.troops);
            commands.spawn((
                PlayerLabel(player.id),
                PlayerLabelRenderCache(font_px),
                Text2d::new(label_text),
                TextFont {
                    font: ui.font.clone(),
                    font_size: font_px,
                    // Rajdhani is a vector face: default AA keeps strokes from collapsing to 1px;
                    // slightly heavier weight reads better on noisy tile colors.
                    weight: FontWeight::SEMIBOLD,
                    ..default()
                },
                TextColor(color),
                TextLayout::new_with_justify(Justify::Center),
                Transform::from_xyz(0.0, 0.0, 150.0),
            ));
        }
    }

    // 3. Update positions and text based on territory center of mass
    for (
        _entity,
        mut text,
        mut p_font,
        mut transform,
        mut text_color,
        label,
        mut cache,
    ) in q_labels.iter_mut()
    {
        *text_color = TextColor(Color::WHITE);
        let Some(player) = game.player(label.0) else { continue };

        let new_text = format!("{}\n[{:.0}]", player.name, player.troops);
        if text.0 != new_text {
            text.0 = new_text.clone();
        }

        let mut sum_x = 0;
        let mut sum_y = 0;
        let mut count = 0;
        for (idx, &owner) in game.map.state.iter().enumerate() {
            if owner == player.id {
                let y = idx as u32 / game.map.width;
                let x = idx as u32 % game.map.width;
                sum_x += x;
                sum_y += y;
                count += 1;
            }
        }

        if count > 0 {
            let cx = (sum_x as f32 / count as f32) * TILE_SIZE;
            let cy = (sum_y as f32 / count as f32) * TILE_SIZE;
            
            let target = Vec3::new(cx, cy + ui.config.ui.label_offset_y, 150.0);
            
            // Just jump directly to target if too far, otherwise float smoothly
            if transform.translation.distance(target) > 100.0 {
                transform.translation = target;
            } else {
                transform.translation = transform.translation.lerp(target, 0.2);
            }

            let base_font = ui.config.ui.font_size;
            let mul = player_label_scale(
                count,
                ui.config.ui.label_size_ref_tiles,
                ui.config.ui.label_max_scale,
            );
            let font_px = (base_font * mul).round().max(1.0);
            if cache.0 != font_px {
                cache.0 = font_px;
                p_font.font_size = font_px;
                p_font.weight = FontWeight::SEMIBOLD;
            }
        }
    }
}

// ─── Enoki Combat VFX ────────────────────────────────────────────────────────

fn setup_combat_vfx(mut commands: Commands, asset_server: Res<AssetServer>) {
    let effect = asset_server.load("enoki_impact.ron");
    commands.insert_resource(CombatVfxCache(effect));
}

fn system_spawn_attack_vfx(
    mut commands: Commands,
    q_attacks: Query<(Entity, &AttackExecution)>,
    q_vfx: Query<&CombatVfx>,
    vfx_cache: Res<CombatVfxCache>,
) {
    // Track ones that already have VFX
    let mut tracked = HashSet::new();
    for vfx in &q_vfx {
        tracked.insert(vfx.target_exec);
    }

    for (attack_ent, attack) in &q_attacks {
        if tracked.contains(&attack_ent) { continue; }
        if attack.retreating { continue; }

        let active_target = match attack.to_conquer.peek() {
            Some(t) => t,
            None => continue,
        };

        // Determine target world position
        let wx = (active_target.x as f32 + 0.5) * TILE_SIZE;
        let wy = (active_target.y as f32 + 0.5) * TILE_SIZE;

        let spawn_pos = Vec2::new(wx, wy);

        commands.spawn((
            CombatVfx {
                target_exec: attack_ent,
            },
            Transform::from_xyz(spawn_pos.x, spawn_pos.y, 110.0),
            ParticleSpawner::<ColorParticle2dMaterial>::default(),
            ParticleEffectHandle(vfx_cache.0.clone()),
        ));
    }
}

fn system_sync_attack_vfx_position(
    mut commands: Commands,
    q_attacks: Query<&AttackExecution>,
    mut q_vfx: Query<(Entity, &CombatVfx, &mut Transform)>,
) {
    for (vfx_ent, vfx, mut target_t) in &mut q_vfx {
        if let Ok(attack) = q_attacks.get(vfx.target_exec) {
            // Despawn if attack is retreating or queue is empty
            if attack.retreating || attack.to_conquer.is_empty() {
                commands.entity(vfx_ent).despawn();
                continue;
            }

            // Sync visual position continuously to current target tile logic
            if let Some(front) = attack.to_conquer.peek() {
                let wx = (front.x as f32 + 0.5) * TILE_SIZE;
                let wy = (front.y as f32 + 0.5) * TILE_SIZE;
                
                // Exponential decay interpolation for smooth VFX gliding
                let target_pos = Vec3::new(wx, wy, 110.0);
                target_t.translation = target_t.translation.lerp(target_pos, 0.4);
            }
        } else {
            // Target AttackExecution died/merged/resolved, clean up VFX independent shadow
            commands.entity(vfx_ent).despawn();
        }
    }
}
