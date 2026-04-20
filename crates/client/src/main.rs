//! Dark Rift Client — Networked (Lightyear WebSocket + Netcode)
//!
//! Flow:
//!   1. Spawn a Client entity: NetcodeClient + WebSocketClientIo → native `ws://…:25565`, WASM `wss://…/ws/`
//!   2. Trigger Connect to start handshake
//!   3. MessageReceiver<ServerStartMessage>: init local GameState, spawn tile grid
//!   4. On player click: send ClientGameplayMessage via MessageSender
//!   5. MessageReceiver<ServerTurnMessage>: apply intents via apply_stamped_intent()
//!   6. Shared ECS systems run locally (system_player_income, system_attack_execution)
//!   7. Tile visuals updated from game.events (event-driven, O(events) not O(map))

use bevy::prelude::*;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::text::FontWeight;
use bevy::ui::RelativeCursorPosition;
use bevy::sprite::Sprite;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::{Bloom, BloomCompositeMode};
use bevy_enoki::prelude::*;
use std::collections::{HashMap, HashSet};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use lightyear::netcode::{NetcodeClient, Key};
use lightyear::netcode::prelude::client::NetcodeConfig;
use lightyear::netcode::auth::Authentication;
use lightyear::websocket::client::WebSocketClientIo;
#[cfg(not(target_arch = "wasm32"))]
use lightyear::websocket::client::WebSocketScheme;
use aeronet_io::connection::PeerAddr;
use std::net::SocketAddr;
use std::time::Duration;
use clap::Parser;
use instant::Instant;

use dark_rift_shared::config;
use dark_rift_shared::game::{GameState, GameEvent};
use dark_rift_shared::player::Player;
use dark_rift_shared::execution::{
    system_attack_execution, system_player_income, system_game_tick, AttackExecution,
};
use dark_rift_shared::intent::{apply_stamped_intent, system_bot_think};
use dark_rift_shared::protocol::*;
use dark_rift_shared::pseudo_random::PseudoRandom;

mod map_fetch;
mod settings;
mod ui_layers;
mod ui_shell;
mod ui_theme;
mod ui_lobby_browser;
mod ui_leaderboard;
pub use map_fetch::{MapCache, MapFetchChannel};
use settings::*;
use ui_layers::GAME_OVER;
use ui_shell::*;
use ui_theme::*;
use ui_lobby_browser::*;

const TILE_SIZE: f32 = 8.0;
/// Native client: plain WebSocket to this `SocketAddr` (see `WebSocketScheme::Plain`).
#[cfg(not(target_arch = "wasm32"))]
pub const NATIVE_NETCODE_SOCKET: &str = "127.0.0.1:25565";

/// WASM: browser needs **WSS** through your TLS reverse proxy (Nginx).
#[cfg(target_arch = "wasm32")]
const WASM_MULTIPLAYER_WSS_URL: &str = "wss://darkrift.ai/ws/";

/// WASM: netcode `ConnectToken` / `PeerAddr` must be a **numeric** `IPv4:port` string.
/// `wasm32-unknown-unknown` cannot resolve hostnames at runtime; for production set this to your
/// VPS public IP and the same TCP port Nginx forwards to the game server (often `25565`).
#[cfg(target_arch = "wasm32")]
const WASM_NETCODE_SOCKET: &str = "74.208.246.177:25565";

#[inline]
fn multiplayer_netcode_socket() -> SocketAddr {
    #[cfg(target_arch = "wasm32")]
    {
        WASM_NETCODE_SOCKET
            .parse()
            .expect("WASM_NETCODE_SOCKET must be a numeric IPv4:port (DNS not available on wasm32-unknown-unknown)")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        NATIVE_NETCODE_SOCKET
            .parse()
            .expect("NATIVE_NETCODE_SOCKET must be a valid SocketAddr string")
    }
}

/// Perf overlay: refresh FPS readout at most ~4×/s (avoids jittery every-frame text).
const PERF_OVERLAY_FPS_INTERVAL: Duration = Duration::from_millis(250);
/// Latency / turn-cadence line: update ~every 1.5s (handshake + EMA are cheap; layout is not).
const PERF_OVERLAY_NET_INTERVAL: Duration = Duration::from_millis(1600);

/// Simulation ticks applied per Bevy frame while draining `PendingTicks`.
/// Previously up to 100/frame caused multi‑second stalls and huge `GameEvent` batches.
const CLIENT_CATCHUP_MAX_TICKS_PER_FRAME: u32 = 3;

// ─── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource, Default)]
pub struct GameReady(pub bool);

#[derive(Resource, Default)]
pub struct MyPlayerId(pub u16);

#[derive(Resource)]
pub struct PendingServerStart(pub dark_rift_shared::protocol::ServerStartMessage);


#[derive(Resource)]
pub struct LoadingMapTimeout(pub Timer);

#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)]
pub enum AppState {
    #[default]
    LobbyBrowser,
    LoadingMap,
    InGame,
}

#[derive(Component)]
pub struct InGameEntity;

#[derive(Resource, Default)]
pub struct CameraDragState {
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

#[derive(Component)]
struct HomeButton;

#[derive(Component)]
struct ExitButton;

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct FpsText;

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
struct PlayerLabel(u16);

/// Last applied `TextFont.font_size` for this label; avoids redundant layout when unchanged.
#[derive(Component)]
struct PlayerLabelRenderCache(f32);

/// Root of the attack “ping”: scales as one unit; children are mesh + troop label.
#[derive(Component)]
struct AttackCommandRing {
    elapsed: f32,
    /// Territory RGB for the ring tint (matches `Player::color`).
    rgb: [f32; 3],
}

#[derive(Component)]
struct AttackPingMesh;

#[derive(Component)]
struct AttackPingTroopLabel;

#[derive(Resource)]
struct AttackRingMesh(Handle<Mesh>);

#[derive(Resource, Default)]
struct FreshlyPaintedTiles {
    tiles: HashMap<(u32, u32), f32>,
}


const CHUNK_SIZE: u32 = 256;

/// Spatial texture chunking: the map is split into a grid of 256×256 Images.
/// When a tile changes, only its chunk's Image gets `get_mut` → only ~256 KB
/// is re-uploaded to the GPU instead of the full 8-19 MB monolithic texture.
#[derive(Resource)]
struct MapChunkGrid {
    chunks_x: u32,
    chunks_y: u32,
    map_width: u32,
    map_height: u32,
    /// Flattened grid: chunk (cx, cy) is at index cy * chunks_x + cx.
    handles: Vec<Handle<Image>>,
}

impl MapChunkGrid {
    #[inline]
    fn chunk_index(&self, tile_x: u32, tile_y: u32) -> usize {
        let cx = tile_x / CHUNK_SIZE;
        let cy = tile_y / CHUNK_SIZE;
        (cy * self.chunks_x + cx) as usize
    }
    #[inline]
    fn chunk_handle(&self, tile_x: u32, tile_y: u32) -> &Handle<Image> {
        &self.handles[self.chunk_index(tile_x, tile_y)]
    }
    /// Width of the chunk at column cx (edge chunks may be smaller).
    #[inline]
    fn chunk_width(&self, cx: u32) -> u32 {
        (self.map_width - cx * CHUNK_SIZE).min(CHUNK_SIZE)
    }
}

/// Troop fraction committed on each attack click (OpenFront `uiState.attackRatio` + ControlPanel slider 1–100%).
#[derive(Resource)]
struct AttackRatio(f32);

impl Default for AttackRatio {
    fn default() -> Self {
        Self(0.2)
    }
}

#[derive(Component)]
struct BottomHudRoot;

#[derive(Resource, Default)]
struct CombatVfxCache(Handle<Particle2dEffect>);

/// An independent entity that tracks an attack and emits Enoki particles
#[derive(Component)]
struct CombatVfx {
    target_exec: Entity,
}

/// Multiline panel: incoming (↓) / outgoing (↑) like `AttacksDisplay.ts`.
#[derive(Component)]
struct AttackSummaryText;

/// "⚔ NN% (MMM trp)" readout next to the slider rail.
#[derive(Component)]
struct AttackRatioReadout;

#[derive(Component)]
struct AttackRatioTrack;

#[derive(Component)]
struct AttackRatioFill;

/// Parent node for dynamically rebuilt outgoing attack rows (cancel buttons).
#[derive(Component)]
struct OutgoingRowsContainer;

/// Root UI row for one outgoing attack (stable across frames for click handling).
#[derive(Component)]
struct OutgoingAttackRowRoot;

#[derive(Component)]
struct OutgoingAttackLineText;

#[derive(Component)]
struct CancelAttackButton(u64);

#[derive(Resource, Default)]
struct OutgoingAttackRowMap(std::collections::HashMap<u64, (Entity, Entity)>);

#[derive(Parser, Resource, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Start as an observer
    #[arg(short, long)]
    pub observer: bool,

    /// Play a replay file locally
    #[arg(short, long)]
    pub replay: Option<String>,

    /// Run as a headless bot for network stress testing
    #[arg(short, long)]
    pub bot: bool,

    /// Profile config file inside `assets/bots/` (e.g. `tortuga.ron`)
    #[arg(short, long)]
    pub profile: Option<String>,
}

#[derive(Resource)]
pub struct MockSinglePlayerStart(pub ServerStartMessage);

#[derive(Resource, Default)]
pub struct SinglePlayerState {
    pub timer: Timer,
    pub current_turn: u64,
}

#[derive(Resource, Default)]
pub struct LocalIntentBuffer(pub Vec<StampedIntent>);

#[derive(Resource)]
pub struct HeadlessBotState {
    pub profile: dark_rift_shared::game_config::BotProfile,
    pub timer: Timer,
    pub has_joined_lobby: bool,
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

    if cli.bot {
        // ── HEADLESS MODE ─────────────────────────────────────────────────────
        // Only MinimalPlugins (no AssetPlugin, no RenderPlugin, no Winit).
        // This avoids the Shader asset registration crash.
        let profile_path = cli.profile.as_deref().unwrap_or("tortuga");
        let profile_str = std::fs::read_to_string(format!("assets/bots/{profile_path}.ron"))
            .unwrap_or_else(|_| String::from("(attack_interval_ticks: 240, trigger_ratio: 0.6, reserve_ratio: 0.3, expand_ratio: 0.15)"));
        let profile: dark_rift_shared::game_config::BotProfile = ron::from_str(&profile_str).expect("Failed to parse bot profile");
        info!("🤖 Headless Bot initialized profile='{}' interval_ticks={}", profile_path, profile.attack_interval_ticks);

        app.add_plugins(MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        )));
        app.add_plugins(bevy::log::LogPlugin::default());
        app.add_plugins(bevy::state::app::StatesPlugin);

        app.insert_resource(HeadlessBotState {
            timer: Timer::new(std::time::Duration::from_secs_f64(profile.attack_interval_ticks as f64 / 60.0), TimerMode::Repeating),
            profile,
            has_joined_lobby: false,
        });

        // Shared game resources (no render deps)
        app.insert_resource(cli.clone())
            .init_resource::<GameReady>()
            .init_resource::<MyPlayerId>()
            .init_resource::<PendingTicks>()
            .init_resource::<TurnQueue>()
            .init_resource::<HighestReceivedTurn>()
            .init_resource::<NetTiming>()
            .init_resource::<HoveredTile>()
            .init_resource::<FreshlyPaintedTiles>()
            .init_resource::<AttackRatio>()
            .init_resource::<OutgoingAttackRowMap>();

        // Network stack — same as normal client, but no visual plugins
        app.add_plugins(SharedProtocolPlugin)
           .add_plugins(ClientPlugins::default())
           .init_state::<AppState>()
           .add_systems(Startup, setup_network)
           .add_systems(Update, (
               handle_server_start,
               system_mock_single_player_start,
               handle_connection,
           ).chain())
           .add_systems(Update, (
               handle_server_turns,
               system_evaluate_game_over,
           ).chain().run_if(in_state(AppState::InGame)));
    } else {
        // ── NORMAL (GRAPHICAL) MODE ────────────────────────────────────────────
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Dark Rift — Multiplayer".to_string(),
                resolution: (1280u32, 720u32).into(),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }));

        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        })
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.04)))
        .insert_resource(cli.clone())
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

        // Insert ClientSettings BEFORE app.run() so it exists when OnEnter(LobbyBrowser) fires.
        // Font handle is Handle::default() here; init_client_font patches it in Startup.
        app.world_mut().insert_resource(ClientSettings {
            config: build_client_settings_config(),
            font: Handle::default(),
        });
    }

    if !cli.bot {
    // ── REPLAY / LIVE BLOCK (skipped for headless bots) ───────────────────
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
                init_client_font,
                init_attack_ratio_from_ui_settings,
                setup_hud_shell,
                setup_offline,
            )
                .chain(),
        )
           .add_systems(Update, system_replay_pulse);
    } else {
        // ── LIVE NETWORKING (both normal clients AND headless bots need this) ──
        app.add_plugins(SharedProtocolPlugin)
           .add_plugins(ClientPlugins::default())
           .init_state::<AppState>();

        if !cli.bot {
            // Visual-only plugins — skip entirely in headless mode
            app.add_plugins(EnokiPlugin)
               .init_resource::<CombatVfxCache>();
        }

        app.add_systems(
                Startup,
                (
                    setup_network,
                    setup_attack_ring_mesh,
                    init_client_font,
                    init_attack_ratio_from_ui_settings,
                    setup_hud_shell,
                    setup_live,
                    setup_combat_vfx,
                )
                    .chain()
                    .run_if(|cli: Res<Cli>| !cli.bot),
            )
            .add_systems(OnEnter(AppState::LobbyBrowser), setup_lobby_browser.run_if(|cli: Res<Cli>| !cli.bot))
            .add_systems(OnExit(AppState::LobbyBrowser), system_cleanup_lobby_browser.run_if(|cli: Res<Cli>| !cli.bot))
            .add_systems(OnEnter(AppState::InGame), ui_leaderboard::setup_leaderboard.run_if(|cli: Res<Cli>| !cli.bot))
            .add_systems(OnExit(AppState::InGame), system_cleanup_ingame_entities.run_if(|cli: Res<Cli>| !cli.bot))
            .add_systems(Update, (
                system_handle_join_clicks,
                system_handle_cancel_join,
                ui_lobby_browser::system_handle_coming_soon_click,
                ui_lobby_browser::system_clear_lobby_nickname_focus,
                ui_lobby_browser::system_handle_single_player_click,
                ui_lobby_browser::system_nickname_field_focus_from_click,
                ui_lobby_browser::system_nickname_keyboard_when_focused
                    .run_if(|f: Res<LobbyNicknameFocused>| f.0),
                ui_lobby_browser::system_nickname_autosave_tick,
                system_lobby_main_menu_anim,
                system_update_lobby_browser,
            ).run_if(|state: Res<State<AppState>>, cli: Res<Cli>| *state.get() == AppState::LobbyBrowser && !cli.bot))
            .add_systems(Update, (
                handle_server_start,
                system_mock_single_player_start,
                handle_connection,
            ).chain())
            .add_systems(Update, (
                system_single_player_tick,
            ).run_if(in_state(AppState::InGame)))
            .add_systems(Update, (
               handle_server_turns,
               system_evaluate_game_over,
               system_handle_go_back,
           ).chain().run_if(in_state(AppState::InGame)))
            .add_systems(PreUpdate, auto_inject_visibility)
            .add_systems(Update, map_fetch::system_poll_map_fetch)
            .add_systems(OnEnter(AppState::LoadingMap), setup_loading_map_timeout)
            .add_systems(Update, system_loading_map.run_if(in_state(AppState::LoadingMap)));
    }
    } // end if !cli.bot

    if !cli.bot {
        app.add_systems(Update, (system_update_ui_shell_state, system_apply_ui_shell_layout, system_update_perf_overlay).run_if(|cli: Res<Cli>| !cli.bot));

        app.add_systems(Update, (
            hover_system,
            system_attack_ratio_keyboard,
            system_attack_ratio_slider_drag,
            player_click,
            system_exit_button,
            system_home_button,
            update_hud,
            system_apply_responsive_hud_layout,
            system_sync_bottom_combat_hud,
            system_sync_outgoing_attack_rows,
            system_handle_cancel_click,
        ).run_if(in_state(AppState::InGame)));

        app.add_systems(Update,
            ui_leaderboard::system_update_leaderboard
                .run_if(in_state(AppState::InGame))
                .run_if(bevy::time::common_conditions::on_timer(std::time::Duration::from_secs(1)))
        );

        // ── Tile rendering: direct chunk painting ──
        app.add_systems(Update, (
            handle_tile_change,
            system_animate_tile_colors.after(handle_tile_change),
        ).run_if(in_state(AppState::InGame)));
        
        app.add_systems(
            Update,
            system_animate_attack_command_rings
                .after(player_click)
                .run_if(in_state(AppState::InGame)),
        );
        
        app.add_systems(Update, (
            camera_movement_and_zoom,
            system_spawn_attack_vfx.run_if(|settings: Res<ClientSettings>| settings.config.visuals.enable_combat_particles),
            system_sync_attack_vfx_position.run_if(|settings: Res<ClientSettings>| settings.config.visuals.enable_combat_particles),
            system_sync_player_labels.run_if(|settings: Res<ClientSettings>| settings.config.visuals.enable_floating_labels),
        ).run_if(in_state(AppState::InGame)));

        app.add_systems(Update, clear_inputs_on_focus_loss);
    }

    let mut sim_schedule = Schedule::new(SimulationSchedule);
    sim_schedule.add_systems((
        pop_turn,
        system_player_income,
        system_bot_think,
        system_attack_execution,
        system_game_tick,
        system_headless_bot_engine.run_if(|cli: Res<Cli>| cli.bot),
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

fn auto_inject_visibility(
    mut commands: Commands,
    q: Query<Entity, (Or<(With<Transform>, With<Node>)>, Without<InheritedVisibility>)>,
) {
    for entity in q.iter() {
        commands.entity(entity).insert((
            Visibility::default(),
            InheritedVisibility::default(),
            ViewVisibility::default(),
        ));
    }
}

// ─── Camera Controls ────────────────────────────────────────────────────────

fn camera_movement_and_zoom(
    mut q_camera: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut scroll_evr: MessageReader<MouseWheel>,
    mut motion_evr: MessageReader<bevy::input::mouse::MouseMotion>,
    mut drag_state: ResMut<CameraDragState>,
) {
    if mouse_input.just_pressed(MouseButton::Left) || touches.any_just_pressed() {
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
            ortho.scale = (ortho.scale * (1.0 + zoom)).clamp(0.1, 20.0);
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
    mut images: ResMut<Assets<Image>>,
    settings: Res<ClientSettings>,
    mut overlay: ResMut<OverlayDiagnostics>,
    shell: Res<HudShellRegions>,
) {
    let mut cam = commands.spawn((
        Camera2d,
        Tonemapping::TonyMcMapface,
        Transform::from_xyz(
            director.record.config.map_width as f32 * TILE_SIZE / 2.0,
            director.record.config.map_height as f32 * TILE_SIZE / 2.0,
            999.0
        ),
    ));
    if settings.config.visuals.hdr_bloom_enabled() {
        cam.insert(Bloom {
            intensity: 0.15,
            composite_mode: BloomCompositeMode::Additive,
            ..default()
        });
    }

    overlay.status_line = "Replay Mode".to_string();
    overlay.ping_value = "offline".to_string();

    commands
        .spawn((
            ChildOf(shell.top_bar),
            Node {
                align_self: AlignSelf::FlexStart,
                width: Val::Auto,
                min_width: Val::Px(200.0),
                max_width: Val::Percent(72.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.08, 0.1, 0.85)),
            BorderColor::all(panel_border()),
        ))
        .with_children(|p| {
            p.spawn((
                HudText,
                Text::new("Replay Mode\nTick: 0\nPing: offline"),
                TextFont {
                    font: settings.font.clone(),
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::srgba(0.8, 0.8, 0.8, 1.0)),
            ));
        });

    commands
        .spawn((
            ChildOf(shell.top_bar),
            Node {
                align_self: AlignSelf::FlexStart,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(12.0),
                ..default()
            },
        ))
        .with_children(|right_group| {
            // FPS Pill
            right_group.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.8)),
            )).with_children(|fps_pill| {
                fps_pill.spawn((
                    FpsText,
                    Text::new("… FPS"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.4, 0.9, 0.4, 1.0)),
                ));
            });

            // EXIT Button
            right_group.spawn((
                Button,
                ExitButton,
                Node {
                    width: Val::Px(120.0),
                    height: Val::Px(48.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.8, 0.2, 0.2, 1.0)),
                BorderColor::all(Color::srgba(1.0, 0.3, 0.3, 1.0)),
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("EXIT"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 20.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            });
        });

    let mut game = GameState::new(director.record.seed, director.record.config.map_width, director.record.config.map_height);
    for info in &director.record.players {
        let player = Player::new_human(info.id, info.name.clone(), info.color);
        game.spawn_player(player, info.spawn_x, info.spawn_y);
    }

    let grid = spawn_chunked_texture_tilemap(&mut commands, &game, &mut images, false);
    commands.insert_resource(grid);

    commands.insert_resource(game);
    ready.0 = true;
}

fn setup_live(
    mut commands: Commands,
    _cli: Res<Cli>,
    settings: Res<ClientSettings>,
    mut overlay: ResMut<OverlayDiagnostics>,
    shell: Res<HudShellRegions>,
) {
    let mut cmd = commands.spawn((
        Camera2d,
        Tonemapping::TonyMcMapface,
        Transform::from_xyz(0.0, 0.0, 999.0),
    ));
    
    if settings.config.visuals.hdr_bloom_enabled() {
        cmd.insert(Bloom {
            intensity: 0.15,
            composite_mode: BloomCompositeMode::Additive,
            ..default()
        });
    }

    overlay.status_line = "Connecting to server...".to_string();
    overlay.ping_value = "connecting…".to_string();

    commands
        .spawn((
            ChildOf(shell.top_bar),
            Node {
                align_self: AlignSelf::FlexStart,
                width: Val::Auto,
                min_width: Val::Px(200.0),
                max_width: Val::Percent(72.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.08, 0.1, 0.85)),
            BorderColor::all(panel_border()),
        ))
        .with_children(|p| {
            p.spawn((
                HudText,
                Text::new("Connecting to server...\nTick: 0\nPing: connecting…"),
                TextFont {
                    font: settings.font.clone(),
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::srgba(0.8, 0.8, 0.8, 1.0)),
            ));
        });

    commands
        .spawn((
            ChildOf(shell.top_bar),
            Node {
                align_self: AlignSelf::FlexStart,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(12.0),
                ..default()
            },
        ))
        .with_children(|right_group| {
            // FPS Pill
            right_group.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.8)),
            )).with_children(|fps_pill| {
                fps_pill.spawn((
                    FpsText,
                    Text::new("… FPS"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.4, 0.9, 0.4, 1.0)), // Bright green FPS
                ));
            });

            // EXIT Button
            right_group.spawn((
                Button,
                ExitButton,
                Node {
                    width: Val::Px(120.0),
                    height: Val::Px(48.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.8, 0.2, 0.2, 1.0)),
                BorderColor::all(Color::srgba(1.0, 0.3, 0.3, 1.0)),
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("EXIT"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 20.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            });
        });

    // Home Button (gameplay dock)
    commands.spawn((
        ChildOf(shell.action_dock),
        Node {
            width: Val::Px(120.0),
            height: Val::Px(48.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(12.0)),
            ..default()
        },
        Button,
        HomeButton,
        BackgroundColor(accent_home()),
        BorderColor::all(Color::srgba(0.8, 0.8, 1.0, 1.0)),
    ))
    .with_children(|b| {
        b.spawn((
            Text::new("HOME"),
            TextFont {
                font: settings.font.clone(),
                font_size: 20.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
    });
}

fn setup_network(mut commands: Commands, cli: Res<Cli>) {
    let server_addr: SocketAddr = multiplayer_netcode_socket();
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

    #[cfg(target_arch = "wasm32")]
    let ws_io = WebSocketClientIo::from_url(
        aeronet_websocket::client::ClientConfig::default(),
        WASM_MULTIPLAYER_WSS_URL,
    );
    #[cfg(not(target_arch = "wasm32"))]
    let ws_io = WebSocketClientIo::from_addr(
        aeronet_websocket::client::ClientConfig::default(),
        WebSocketScheme::Plain,
    );

    let client_entity = commands.spawn((
        Name::from("DarkRiftClient"),
        netcode,
        PeerAddr(server_addr),
        ws_io,
    )).id();

    commands.insert_resource(NetTiming {
        connect_started: Some(Instant::now()),
        ..default()
    });

    // Trigger Netcode connection (which internally initiates LinkStart)
    commands.trigger(lightyear::prelude::client::Connect { entity: client_entity });

    #[cfg(target_arch = "wasm32")]
    info!(
        "🔌 Connecting (WASM): transport {} | netcode token {:?} (client_id={}, observer={})",
        WASM_MULTIPLAYER_WSS_URL,
        server_addr,
        client_id,
        cli.observer
    );
    #[cfg(not(target_arch = "wasm32"))]
    info!(
        "🔌 Connecting to ws://{}... (client_id={}, observer={})",
        NATIVE_NETCODE_SOCKET,
        client_id,
        cli.observer
    );
}

fn handle_connection(
    mut client_q: Query<&mut MessageSender<ClientJoinMessage>, Added<lightyear::prelude::client::Connected>>,
    cli: Res<Cli>,
) {
    for mut sender in client_q.iter_mut() {
        if cli.bot {
            info!("🔌 Bot Connected to Transport. Automatically joining Lobby...");
            let msg = ClientJoinMessage { 
                name: format!("HydraBot-{}", rand_client_id() % 1000), 
                is_observer: cli.observer, 
                target_lobby_id: None, // Joins any available lobby or seeds a new one
            };
            let _ = sender.send::<ReliableChannel>(msg);
        } else {
            info!("🔌 Connected to Transport. Presenting Lobby Browser...");
        }
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

/// Bottom-center stack: attack summary (OpenFront `AttacksDisplay`) + ratio slider row (`ControlPanel` row 2).
fn spawn_bottom_combat_hud(commands: &mut Commands, shell: &HudShellRegions, observer: bool) {
    let vis = if observer {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    commands
        .spawn((
            ChildOf(shell.bottom_sheet),
            BottomHudRoot,
            InGameEntity,
            RelativeCursorPosition::default(),
            vis,
            Node {
                width: Val::Percent(100.0),
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(12.0),
                padding: UiRect::bottom(Val::Px(16.0)),
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Percent(96.0),
                    min_width: Val::Px(280.0),
                    max_width: Val::Px(800.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.08, 0.1, 0.9)),
                BorderColor::all(panel_border()),
                AttackSummaryText,
                Text::new(""),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::srgba(0.9, 0.9, 0.9, 1.0)),
            ));

            root.spawn((
                OutgoingRowsContainer,
                Node {
                    width: Val::Percent(96.0),
                    min_width: Val::Px(280.0),
                    max_width: Val::Px(800.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    max_height: Val::Px(280.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
            ));

            root.spawn((
                Node {
                    width: Val::Percent(96.0),
                    max_width: Val::Px(800.0),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(16.0),
                    flex_wrap: FlexWrap::Wrap,
                    padding: UiRect::all(Val::Px(12.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.15, 0.85)),
            ))
            .with_children(|row| {
                row.spawn((
                    AttackRatioReadout,
                    Text::new(""),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Node {
                        width: Val::Px(180.0),
                        ..default()
                    },
                ));

                row.spawn((
                    Button,
                    AttackRatioTrack,
                    RelativeCursorPosition::default(),
                    Node {
                        flex_grow: 1.0,
                        height: Val::Px(32.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Stretch,
                        ..default()
                    },
                ))
                .with_children(|track| {
                    track
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(16.0),
                                flex_direction: FlexDirection::Row,
                                overflow: Overflow::clip(),
                                border_radius: BorderRadius::all(Val::Px(8.0)),
                                ..default()
                            },
                        ))
                        .with_children(|bar| {
                            bar.spawn((
                                AttackRatioFill,
                                Node {
                                    width: Val::Percent(20.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.2, 0.8, 0.2, 1.0)),
                            ));
                            bar.spawn((
                                Node {
                                    flex_grow: 1.0,
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.2, 0.2, 0.25)),
                            ));
                        });
                });
            });
        });
}

/// Receive ServerStartMessage → build GameState and spawn tile grid.
/// Mirrors OpenFront's message.type === "start" handler in ClientGameRunner.
fn handle_server_start(
    mut receivers: Query<&mut MessageReceiver<ServerStartMessage>>,
    mut commands:  Commands,
    mut net:       ResMut<NetTiming>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            commands.insert_resource(PendingServerStart(msg.clone()));
            next_state.set(AppState::LoadingMap);
            
            if net.handshake_ms.is_none() {
                if let Some(t0) = net.connect_started.take() {
                    net.handshake_ms = Some(t0.elapsed().as_secs_f32() * 1000.0);
                }
            }
        }
    }
}

fn setup_loading_map_timeout(mut commands: Commands) {
    commands.insert_resource(LoadingMapTimeout(Timer::from_seconds(15.0, TimerMode::Once)));
}

fn system_loading_map(
    time: Res<Time>,
    mut commands: Commands,
    mut timeout: ResMut<LoadingMapTimeout>,
    mut cache: Option<ResMut<MapCache>>,
    pending_msg: Option<ResMut<PendingServerStart>>,
    mut ready: ResMut<GameReady>,
    mut my_id: ResMut<MyPlayerId>,
    mut cam_q: Query<&mut Transform, With<Camera2d>>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
    mut highest: ResMut<HighestReceivedTurn>,
    mut overlay: Option<ResMut<OverlayDiagnostics>>,
    shell: Option<Res<HudShellRegions>>,
    mut images: Option<ResMut<Assets<Image>>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    timeout.0.tick(time.delta());
    if timeout.0.just_finished() {
        bevy::log::error!("Map load timeout! Returning to lobby.");
        next_state.set(AppState::LobbyBrowser);
        commands.remove_resource::<PendingServerStart>();
        return;
    }

    let Some(mut p_msg) = pending_msg else { return; };

    // Ensure MapCache exists
    if cache.is_none() {
        commands.insert_resource(MapCache::default());
        return; // Wait one frame for it to be available
    }
    let cache = cache.as_mut().unwrap();

    let map_name = &p_msg.0.config.map_name;

    // If cache doesn't have the right map, request it
    if !cache.is_ready() || cache.map_name != *map_name {
        if !cache.is_fetching() || cache.map_name != *map_name {
            cache.request(map_name, &mut commands);
        }
        return; // Wait for fetch to complete
    }

    // Map is ready — start the game
    let cached_bytes = cache.take_data();
    p_msg.0.map_data = cached_bytes;

    start_game_core(
        p_msg.0.clone(),
        &mut ready,
        &mut my_id,
        &mut cam_q,
        &mut commands,
        &mut queue,
        &mut pending,
        &mut highest,
        &mut overlay,
        &shell,
        &mut images,
        &mut next_state,
    );

    commands.remove_resource::<PendingServerStart>();
}


fn system_mock_single_player_start(
    mock:      Option<ResMut<MockSinglePlayerStart>>,
    mut ready:     ResMut<GameReady>,
    mut my_id:     ResMut<MyPlayerId>,
    mut cam_q:     Query<&mut Transform, With<Camera2d>>,
    mut commands:  Commands,
    mut queue:     ResMut<TurnQueue>,
    mut pending:   ResMut<PendingTicks>,
    mut highest:   ResMut<HighestReceivedTurn>,
    mut overlay:   Option<ResMut<OverlayDiagnostics>>,
    shell:         Option<Res<HudShellRegions>>,
    mut images:    Option<ResMut<Assets<Image>>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if let Some(m) = mock {
        commands.insert_resource(SinglePlayerState {
            timer: Timer::from_seconds(0.040, TimerMode::Repeating),
            current_turn: 0,
        });
        commands.insert_resource(LocalIntentBuffer::default());
        start_game_core(
            m.0.clone(), &mut ready, &mut my_id, &mut cam_q, &mut commands,
            &mut queue, &mut pending, &mut highest, &mut overlay, &shell, &mut images, &mut next_state
        );
        commands.remove_resource::<MockSinglePlayerStart>();
    }
}

fn system_single_player_tick(
    time: Res<Time>,
    sp_state: Option<ResMut<SinglePlayerState>>,
    buffer: Option<ResMut<LocalIntentBuffer>>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
    mut highest: ResMut<HighestReceivedTurn>,
) {
    if let (Some(mut st), Some(mut buf)) = (sp_state, buffer) {
        st.timer.tick(time.delta());
        if st.timer.just_finished() {
            let turn = dark_rift_shared::protocol::Turn {
                turn_number: st.current_turn,
                intents: std::mem::take(&mut buf.0),
            };
            st.current_turn += 1;
            
            queue.0.push_back(turn);
            pending.0 += 1;
            highest.0 = Some(highest.0.unwrap_or(0).max(st.current_turn));
        }
    }
}

fn start_game_core(
    msg:           ServerStartMessage,
    ready:         &mut ResMut<GameReady>,
    my_id:         &mut ResMut<MyPlayerId>,
    cam_q:         &mut Query<&mut Transform, With<Camera2d>>,
    commands:      &mut Commands,
    queue:         &mut ResMut<TurnQueue>,
    pending:       &mut ResMut<PendingTicks>,
    highest:       &mut ResMut<HighestReceivedTurn>,
    overlay:       &mut Option<ResMut<OverlayDiagnostics>>,
    shell:         &Option<Res<HudShellRegions>>,
    images:        &mut Option<ResMut<Assets<Image>>>,
    next_state:    &mut ResMut<NextState<AppState>>,
) {
            if ready.0 { return; }

            if let Some(pid) = msg.my_player_id {
                info!("🎮 Game starting! I am player {}", pid);
                my_id.0 = pid;
            } else {
                info!("👁  Game starting! I am an observer");
                my_id.0 = 0;
            }

            let mut game = GameState::new(msg.seed, msg.config.map_width, msg.config.map_height);
            map_fetch::apply_map_to_game(&mut game, &msg.config.map_name, msg.map_data.clone());

            for info in &msg.players {
                let player = match info.player_type {
                    dark_rift_shared::player::PlayerType::Human => {
                        Player::new_human(info.id, info.name.clone(), info.color)
                    }
                    dark_rift_shared::player::PlayerType::Bot => {
                        Player::new_bot(info.id, info.name.clone(), info.color)
                    }
                };
                game.spawn_player(player, info.spawn_x, info.spawn_y);
            }

            // Replay missed turns (late-join) — identical behavior to OpenFront
            for turn in &msg.missed_turns {
                queue.0.push_back(turn.clone());
                pending.0 += 1;
                highest.0 = Some(highest.0.unwrap_or(0).max(turn.turn_number));
            }

            commands.insert_resource(msg.config.clone());
            
            
            let map_w = msg.config.map_width as f32;
            let map_h = msg.config.map_height as f32;
            let mut cx = map_w * TILE_SIZE / 2.0;
            let mut cy = map_h * TILE_SIZE / 2.0;

            if my_id.0 != 0 {
                if let Some(me) = msg.players.iter().find(|p| p.id == my_id.0) {
                    cx = me.spawn_x as f32 * TILE_SIZE;
                    cy = (game.map.height as f32 - 1.0 - me.spawn_y as f32) * TILE_SIZE;
                }
            }

            if let Ok(mut cam) = cam_q.single_mut() {
                cam.translation = Vec3::new(cx, cy, 999.0);
            }
            if let Some(images_res) = images.as_mut() {
                let grid = spawn_chunked_texture_tilemap(commands, &game, images_res, true);
                commands.insert_resource(grid);
            }

            commands.insert_resource(game);
            ready.0 = true;
            next_state.set(AppState::InGame);

            if let Some(s) = shell {
                spawn_bottom_combat_hud(commands, s, msg.my_player_id.is_none());
            }

            if let Some(over) = overlay {
                if msg.my_player_id.is_some() {
                    over.status_line = format!("Connected — Player {:?}", msg.my_player_id);
                } else {
                    over.status_line = "Connected — Observer".to_string();
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
    let gy = game.map.height as i32 - 1 - (world.y / TILE_SIZE).round() as i32;
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
    mouse:       Res<ButtonInput<MouseButton>>,
    touches:     Res<Touches>,
    drag_state:  Res<CameraDragState>,
    hovered:     Res<HoveredTile>,
    game_opt:    Option<Res<GameState>>,
    my_id:       Res<MyPlayerId>,
    attack_ratio: Res<AttackRatio>,
    settings:    Res<ClientSettings>,
    bottom_hud_hit: Query<&RelativeCursorPosition, With<BottomHudRoot>>,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
    local_buf: Option<ResMut<LocalIntentBuffer>>,
) {
    // We trigger ONLY on release to support dragging.
    // Additionally, if the user accumulated too much drag, we abort the intent payload.
    if !mouse.just_released(MouseButton::Left) && !touches.any_just_released() { return; }
    if drag_state.accumulated_drag > 10.0 { return; } // It was a drag, not a click

    // Clicks/releases over the combat HUD must not issue map attacks (OpenFront: UI eats pointer).
    if bottom_hud_hit.iter().any(|rcp| rcp.cursor_over) {
        return;
    }

    let Some((x, y)) = hovered.0 else { return };
    let Some(game) = game_opt else { return };

    let pid = my_id.0;
    let Some(player) = game.players.iter().find(|p| p.id == pid) else { return };
    if !player.alive || player.troops < config::EXPAND_COST { return; }

    let target_owner = game.map.owner_id(x, y);
    if target_owner == pid { return; }

    let ratio = attack_ratio.0.clamp(0.01, 1.0) as f64;
    let send_floor = (player.troops * ratio).floor();
    let send = send_floor.max(0.0).min(player.troops);
    if send < config::EXPAND_COST {
        return;
    }

    let wx = (x as f32 + 0.5) * TILE_SIZE;
    let wy = (game.map.height as f32 - y as f32 - 0.5) * TILE_SIZE;
    let [r, g, b] = player.color;
    let mat = materials.add(ColorMaterial::from_color(Color::srgba(r, g, b, 0.97)));
    let label = format!("{:.0}", send);
    let ox = TILE_SIZE * 1.75;
    let oy = TILE_SIZE * 0.35;

    // One root transform (same pop as before): mesh + Text2d as children so the number inherits scale.
    commands
        .spawn((
            AttackCommandRing {
                elapsed: 0.0,
                rgb: player.color,
            },
            InGameEntity,
            Transform::from_xyz(wx, wy, 260.0).with_scale(Vec3::ZERO),
            Visibility::default(),
            InheritedVisibility::default(),
            ViewVisibility::default(),
        ))
        .with_children(|root| {
            root.spawn((
                AttackPingMesh,
                Mesh2d(ring_mesh.0.clone()),
                MeshMaterial2d(mat),
                Transform::default(),
            ));
            root.spawn((
                AttackPingTroopLabel,
                Text2d::new(label),
                TextFont {
                    font: settings.font.clone(),
                    font_size: 28.0,
                    weight: FontWeight::BOLD,
                    ..default()
                },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.98)),
                TextLayout::new_with_justify(Justify::Left),
                Transform::from_xyz(ox, oy, 2.0),
            ));
        });

    let intent = GameplayIntent::Attack(AttackIntent {
        target_owner,
        troops: Some(send),
    });

    if let Some(mut buf) = local_buf {
        // Single player mock
        buf.0.push(StampedIntent {
            player_id: pid,
            intent,
        });
    } else {
        // Multiplayer
        for mut sender in senders.iter_mut() {
            let _ = sender.send::<ReliableChannel>(ClientGameplayMessage {
                intent: intent.clone(),
            });
        }
    }
}

/// Quick 0→full scale pop (rest of lifetime = settle + fade).
const ATTACK_RING_POP_SECS: f32 = 0.1;
const ATTACK_COMMAND_RING_LIFETIME: f32 = 0.38;

fn system_animate_attack_command_rings(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut AttackCommandRing, &mut Transform, &Children)>,
    mesh_q: Query<&MeshMaterial2d<ColorMaterial>, With<AttackPingMesh>>,
    mut text_color_q: Query<&mut TextColor, With<AttackPingTroopLabel>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let dt = time.delta_secs();
    for (entity, mut ring, mut transform, children) in &mut q {
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

        for child in children.iter() {
            if let Ok(mat_h) = mesh_q.get(child) {
                if let Some(mat) = materials.get_mut(&mat_h.0) {
                    mat.color = Color::srgba(r, g, b, 0.97 * fade);
                }
            }
            if let Ok(mut tc) = text_color_q.get_mut(child) {
                *tc = TextColor(Color::srgba(
                    1.0,
                    1.0,
                    1.0,
                    (0.25 + 0.75 * fade).clamp(0.0, 1.0),
                ));
            }
        }
    }
}

fn system_exit_button(
    mut interaction_q: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<ExitButton>),
    >,
    mut next_state: ResMut<NextState<AppState>>,
    mut senders: Query<&mut MessageSender<ClientLeaveMessage>>,
    mut joined_lobby: Option<ResMut<JoinedLobby>>,
) {
    for (interaction, mut color) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *color = Color::srgb(0.28, 0.1, 0.08).into();
                if let Some(jl) = joined_lobby.as_mut() {
                    jl.0 = None;
                }
                for mut sender in senders.iter_mut() {
                    let _ = sender.send::<ReliableChannel>(ClientLeaveMessage);
                }
                next_state.set(AppState::LobbyBrowser);
            }
            Interaction::Hovered => {
                *color = Color::srgb(0.72, 0.28, 0.24).into();
            }
            Interaction::None => {
                *color = accent_danger().into();
            }
        }
    }
}

fn system_home_button(
    mut interaction_q: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<HomeButton>),
    >,
    mut cam_q: Query<&mut Transform, With<Camera2d>>,
    labels_q: Query<(&Transform, &PlayerLabel), Without<Camera2d>>,
    my_id: Res<MyPlayerId>,
) {
    for (interaction, mut color) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *color = accent_home_pressed().into();
                // Home the camera
                let mut target_pos = None;
                for (lbl_trans, lbl) in labels_q.iter() {
                    if lbl.0 == my_id.0 {
                        target_pos = Some(lbl_trans.translation.xy());
                        break;
                    }
                }
                if let Some(pos) = target_pos {
                    for mut cam_t in cam_q.iter_mut() {
                        cam_t.translation.x = pos.x;
                        cam_t.translation.y = pos.y;
                    }
                }
            }
            Interaction::Hovered => {
                *color = accent_home_hover().into();
            }
            Interaction::None => {
                *color = accent_home().into();
            }
        }
    }
}

// ─── Visual ──────────────────────────────────────────────────────────────────

/// Max fresh tiles tracked for expansion glow.
const MAX_FRESH_TILES: usize = 512;

/// Helper: paint a single tile (with optional animation boost) directly into
/// the chunk Image that owns it. Only that chunk (~256 KB) gets re-uploaded.
#[inline]
fn paint_tile_to_chunk(
    images: &mut Assets<Image>,
    grid: &MapChunkGrid,
    game: &GameState,
    x: u32,
    y: u32,
    fresh_tiles: &FreshlyPaintedTiles,
) {
    let wx = x as i32;
    let wy = y as i32;
    if !game.map.is_valid_coord(wx, wy) { return; }

    let mut color = openfront_tile_color(game, wx, wy);

    if let Some(&val) = fresh_tiles.tiles.get(&(x, y)) {
        let srgba = color.to_srgba();
        let t = val.clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);
        let boost = 1.0 + 4.0 * eased;
        let owner = game.map.owner_id(x, y);
        let is_border = [(wx, wy - 1), (wx + 1, wy), (wx, wy + 1), (wx - 1, wy)]
            .iter()
            .any(|&(nx, ny)| {
                game.map.is_valid_coord(nx, ny)
                    && game.map.owner_id(nx as u32, ny as u32) != owner
            });
        let white_flash = if is_border { 2.5 * eased } else { 0.0 };
        color = Color::srgba(
            srgba.red * boost + white_flash,
            srgba.green * boost + white_flash,
            srgba.blue * boost + white_flash,
            srgba.alpha,
        );
    }

    let handle = grid.chunk_handle(x, y);
    let Some(image) = images.get_mut(handle) else { return };
    let Some(data) = &mut image.data else { return };

    let cx = x / CHUNK_SIZE;
    let local_x = x % CHUNK_SIZE;
    let local_y = y % CHUNK_SIZE;
    let cw = grid.chunk_width(cx);
    let idx = ((local_y * cw + local_x) * 4) as usize;
    if idx + 3 >= data.len() { return; }

    let s = color.to_srgba();
    data[idx]     = (s.red   * 255.0).clamp(0.0, 255.0) as u8;
    data[idx + 1] = (s.green * 255.0).clamp(0.0, 255.0) as u8;
    data[idx + 2] = (s.blue  * 255.0).clamp(0.0, 255.0) as u8;
    data[idx + 3] = (s.alpha * 255.0).clamp(0.0, 255.0) as u8;
}

/// Paint tile changes directly into chunk Images. Only touched chunks get
/// re-uploaded to the GPU (~256 KB each vs 19 MB monolithic).
fn handle_tile_change(
    game_opt: Option<ResMut<GameState>>,
    mut fresh_tiles: ResMut<FreshlyPaintedTiles>,
    grid: Option<Res<MapChunkGrid>>,
    mut images: ResMut<Assets<Image>>,
    settings: Res<ClientSettings>,
) {
    let Some(mut game) = game_opt else { return };
    if !game.is_changed() { return; }
    let Some(grid) = grid else { return };

    let animations_on = settings.config.visuals.territory_expansion_animations_enabled();

    for ev in &game.events {
        if let GameEvent::TileCaptured { x, y, new_owner: _ } = ev {
            if animations_on && fresh_tiles.tiles.len() < MAX_FRESH_TILES {
                fresh_tiles.tiles.insert((*x, *y), 1.0);
            }
            let coords = [
                (*x as i32, *y as i32),
                (*x as i32, *y as i32 - 1),
                (*x as i32, *y as i32 + 1),
                (*x as i32 - 1, *y as i32),
                (*x as i32 + 1, *y as i32),
            ];
            for (wx, wy) in coords {
                if game.map.is_valid_coord(wx, wy) {
                    paint_tile_to_chunk(&mut images, &grid, &game, wx as u32, wy as u32, &fresh_tiles);
                }
            }
        }
    }

    game.events.clear();
}

/// Animate expansion glow by decaying and repainting affected tiles in their chunks.
fn system_animate_tile_colors(
    time: Res<Time>,
    game_opt: Option<Res<GameState>>,
    mut fresh_tiles: ResMut<FreshlyPaintedTiles>,
    grid: Option<Res<MapChunkGrid>>,
    mut images: ResMut<Assets<Image>>,
    settings: Res<ClientSettings>,
) {
    if !settings.config.visuals.territory_expansion_animations_enabled() {
        fresh_tiles.tiles.clear();
        return;
    }
    if fresh_tiles.tiles.is_empty() { return; }
    let Some(game) = game_opt else { return };
    let Some(grid) = grid else { return };

    let dt = time.delta_secs();
    let lerp_speed = 0.85;
    let mut finished = vec![];

    for (&(x, y), val) in fresh_tiles.tiles.iter_mut() {
        *val -= lerp_speed * dt;
        if *val <= 0.0 {
            *val = 0.0;
            finished.push((x, y));
        }
    }

    // Repaint animated tiles (only their chunks get marked dirty).
    for (&(x, y), _) in fresh_tiles.tiles.iter() {
        paint_tile_to_chunk(&mut images, &grid, &game, x, y, &fresh_tiles);
    }

    for key in finished {
        fresh_tiles.tiles.remove(&key);
        paint_tile_to_chunk(&mut images, &grid, &game, key.0, key.1, &fresh_tiles);
    }
}

fn update_hud(
    game_opt:  Option<Res<GameState>>,
    overlay: Res<OverlayDiagnostics>,
    mut hud_q: Query<&mut Text, (With<HudText>, Without<FpsText>)>,
    mut fps_q: Query<&mut Text, (With<FpsText>, Without<HudText>)>,
) {
    let tick = game_opt.map(|g| g.tick).unwrap_or(0);
    
    if let Some(mut text) = hud_q.iter_mut().next() {
        let final_str = format!("{}\nTick: {}\nPing: {}", overlay.status_line, tick, overlay.ping_value);
        if **text != final_str {
            **text = final_str;
        }
    }

    if let Some(mut fps_text) = fps_q.iter_mut().next() {
        let fps_str = format!("{} FPS", overlay.fps_value);
        if **fps_text != fps_str {
            **fps_text = fps_str;
        }
    }
}

fn system_apply_responsive_hud_layout(
    ui_shell: Res<UiShellState>,
    mut nodes: ParamSet<(
        Query<&mut Node, With<BottomHudRoot>>,
        Query<&mut Node, With<AttackRatioTrack>>,
        Query<&mut Node, With<OutgoingRowsContainer>>,
        Query<&mut Node, (With<HomeButton>, With<Button>)>,
        Query<&mut Node, (With<ExitButton>, With<Button>)>,
    )>,
) {
    let compact = matches!(ui_shell.breakpoint, UiBreakpoint::Compact);

    {
        let mut root_q = nodes.p0();
        let Ok(mut root_node) = root_q.single_mut() else {
            return;
        };
        root_node.row_gap = Val::Px(if compact { 8.0 } else { 6.0 });
    }

    {
        let mut track_q = nodes.p1();
        let Ok(mut track_node) = track_q.single_mut() else {
            return;
        };
        track_node.height = Val::Px(if compact { 18.0 } else { 12.0 });
    }

    {
        let mut outgoing_q = nodes.p2();
        let Ok(mut outgoing_node) = outgoing_q.single_mut() else {
            return;
        };
        outgoing_node.max_height = Val::Px(if compact { 180.0 } else { 220.0 });
    }

    {
        let mut home_q = nodes.p3();
        if let Ok(mut home_node) = home_q.single_mut() {
            let size = home_button_size_for_breakpoint(ui_shell.breakpoint);
            home_node.width = Val::Px(home_button_width_for_breakpoint(ui_shell.breakpoint));
            home_node.height = Val::Px(size);
            home_node.border_radius = BorderRadius::all(Val::Px(if compact { 10.0 } else { 12.0 }));
        }
    }
    {
        let mut exit_q = nodes.p4();
        if let Ok(mut exit_node) = exit_q.single_mut() {
            let size = home_button_size_for_breakpoint(ui_shell.breakpoint);
            exit_node.width = Val::Px(home_button_width_for_breakpoint(ui_shell.breakpoint));
            exit_node.height = Val::Px(size);
            exit_node.border_radius = BorderRadius::all(Val::Px(if compact { 10.0 } else { 12.0 }));
        }
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
fn system_attack_ratio_slider_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut ratio: ResMut<AttackRatio>,
    my_id: Res<MyPlayerId>,
    q_track: Query<&RelativeCursorPosition, With<AttackRatioTrack>>,
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

    let apply = |out: &mut f32| {
        for rcp in &q_track {
            if !rcp.cursor_over {
                continue;
            }
            let Some(n) = rcp.normalized else { continue };
            let t = (n.x + 0.5).clamp(0.0, 1.0);
            *out = (0.01 + t * 0.99).clamp(0.01, 1.0);
            break;
        }
    };

    if dragging {
        apply(&mut ratio.0);
    } else if released {
        // Release frame: `pressed` is false — still snap ratio if the cursor is on the rail.
        apply(&mut ratio.0);
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
    mut q_summary: Query<&mut Text, (With<AttackSummaryText>, Without<AttackRatioReadout>)>,
    mut q_readout: Query<&mut Text, (With<AttackRatioReadout>, Without<AttackSummaryText>)>,
    mut q_fill: Query<&mut Node, With<AttackRatioFill>>,
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
    fill.width = Val::Percent(r * 100.0);

    if my_id.0 == 0 {
        **summary = String::new();
        **readout = String::new();
        return;
    }

    let preview_troops = game
        .player(my_id.0)
        .map(|p| (p.troops * r as f64).floor().max(0.0))
        .unwrap_or(0.0);
    let readout_str = format!("⚔ {:.0}% ({:.0})", r * 100.0, preview_troops);
    if **readout != readout_str {
        **readout = readout_str;
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

    if **summary != body {
        **summary = body;
    }
}

fn system_sync_outgoing_attack_rows(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    slot_q: Query<Entity, With<OutgoingRowsContainer>>,
    q_exec: Query<&AttackExecution>,
    my_id: Res<MyPlayerId>,
    settings: Res<ClientSettings>,
    ui_shell: Res<UiShellState>,
    mut row_map: ResMut<OutgoingAttackRowMap>,
    mut q_line: Query<&mut Text, With<OutgoingAttackLineText>>,
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

    let stale: Vec<u64> = row_map
        .0
        .keys()
        .copied()
        .filter(|id| !desired.contains(id))
        .collect();
    for id in stale {
        if let Some((row_e, _)) = row_map.0.remove(&id) {
            commands.entity(row_e).despawn();
        }
    }

    for exec in outgoing {
        let (cancel_w, cancel_h) = touch_target_for_breakpoint(ui_shell.breakpoint);
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
                if **t != line {
                    **t = line;
                }
            }
        } else {
            let font = settings.font.clone();
            let exec_id = exec.id;
            let row_e = commands
                .spawn((
                    ChildOf(slot),
                    OutgoingAttackRowRoot,
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: Val::Px(8.0),
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(panel_radius_for_breakpoint(ui_shell.breakpoint))),
                        ..default()
                    },
                    BackgroundColor(panel_bg()),
                    BorderColor::all(panel_border()),
                ))
                .id();

            let text_e = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingAttackLineText,
                    Text::new(line),
                    TextFont {
                        font: font.clone(),
                        font_size: 11.5,
                        ..default()
                    },
                    TextColor(text_primary()),
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                ))
                .id();

            commands
                .spawn((
                    ChildOf(row_e),
                    Button,
                    CancelAttackButton(exec_id),
                    Node {
                        width: Val::Px(cancel_w),
                        height: Val::Px(cancel_h),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(accent_danger()),
                    BorderColor::all(accent_danger_border()),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("✕"),
                        TextFont {
                            font,
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });

            row_map.0.insert(exec_id, (row_e, text_e));
        }
    }
}

fn system_handle_cancel_click(
    interaction_q: Query<
        (&Interaction, &CancelAttackButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
) {
    for (interaction, btn) in &interaction_q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let attack_id = btn.0;
        for mut sender in senders.iter_mut() {
            let _ = sender.send::<ReliableChannel>(ClientGameplayMessage {
                intent: GameplayIntent::CancelAttack { attack_id },
            });
        }
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
fn spawn_chunked_texture_tilemap(
    commands: &mut Commands,
    game: &GameState,
    images: &mut Assets<Image>,
    in_game: bool,
) -> MapChunkGrid {
    let map_w = game.map.width;
    let map_h = game.map.height;
    let chunks_x = (map_w + CHUNK_SIZE - 1) / CHUNK_SIZE;
    let chunks_y = (map_h + CHUNK_SIZE - 1) / CHUNK_SIZE;

    let mut handles = Vec::with_capacity((chunks_x * chunks_y) as usize);

    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            let x0 = cx * CHUNK_SIZE;
            let y0 = cy * CHUNK_SIZE;
            let cw = (map_w - x0).min(CHUNK_SIZE);
            let ch = (map_h - y0).min(CHUNK_SIZE);

            // Build pixel data for this chunk.
            let mut pixel_data = vec![0u8; (cw * ch * 4) as usize];
            for ly in 0..ch {
                for lx in 0..cw {
                    let gx = (x0 + lx) as i32;
                    let gy = (y0 + ly) as i32;
                    let color = openfront_tile_color(game, gx, gy).to_srgba();
                    let idx = ((ly * cw + lx) * 4) as usize;
                    pixel_data[idx]     = (color.red   * 255.0).clamp(0.0, 255.0) as u8;
                    pixel_data[idx + 1] = (color.green * 255.0).clamp(0.0, 255.0) as u8;
                    pixel_data[idx + 2] = (color.blue  * 255.0).clamp(0.0, 255.0) as u8;
                    pixel_data[idx + 3] = (color.alpha * 255.0).clamp(0.0, 255.0) as u8;
                }
            }

            let mut image = Image::new_fill(
                Extent3d { width: cw, height: ch, depth_or_array_layers: 1 },
                TextureDimension::D2,
                &[0, 0, 0, 0],
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
            );
            image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
            image.sampler = bevy::image::ImageSampler::nearest();
            if let Some(data) = &mut image.data {
                data.copy_from_slice(&pixel_data);
            }

            let handle = images.add(image);

            // Position: center of this chunk region in world space.
            let center_x = (2 * x0 + cw - 1) as f32 * TILE_SIZE / 2.0;
            let center_y = (2 * (map_h - 1) - 2 * y0 - ch + 1) as f32 * TILE_SIZE / 2.0;

            let mut ecmd = commands.spawn((
                Sprite {
                    image: handle.clone(),
                    custom_size: Some(Vec2::new(cw as f32 * TILE_SIZE, ch as f32 * TILE_SIZE)),
                    ..default()
                },
                Transform::from_xyz(center_x, center_y, 0.0),
            ));
            if in_game {
                ecmd.insert(InGameEntity);
            }

            handles.push(handle);
        }
    }

    MapChunkGrid { chunks_x, chunks_y, map_width: map_w, map_height: map_h, handles }
}

/// OpenFront-style TileData:
/// - Border (any cardinal neighbor has different owner) → `base * TERRITORY_BORDER_RGB_MULTIPLIER`, alpha 1
/// - Interior → base color, alpha `config::TERRITORY_INTERIOR_ALPHA`
fn openfront_tile_color(game: &GameState, cx: i32, cy: i32) -> Color {
    let owner = game.map.owner_id(cx as u32, cy as u32);
    let base = player_base_color(game, owner);

    let is_border = [(cx, cy - 1), (cx + 1, cy), (cx, cy + 1), (cx - 1, cy)]
        .iter()
        .any(|&(nx, ny)| {
            game.map.is_valid_coord(nx, ny)
                && game.map.owner_id(nx as u32, ny as u32) != owner
        });

    let color = if owner == 0 {
        // Unowned tiles show the true terrain color
        let terrain = game.map.terrain_type(cx as u32, cy as u32);
        match terrain {
            dark_rift_shared::map::TerrainType::Water | dark_rift_shared::map::TerrainType::Lake => Color::srgb(0.1, 0.3, 0.6), // Blue
            dark_rift_shared::map::TerrainType::Land => Color::srgb(0.4, 0.7, 0.3), // Light Green
            dark_rift_shared::map::TerrainType::Highland => Color::srgb(0.2, 0.5, 0.2), // Dark Green
            dark_rift_shared::map::TerrainType::Mountain => Color::srgb(0.6, 0.5, 0.4), // Brown/Gray
        }
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

    color
}

fn rand_client_id() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let millis = js_sys::Date::now() as u64;
        millis
            .wrapping_mul(6364136223846793005)
            .wrapping_add(seq)
            .wrapping_add(js_sys::Math::random().to_bits())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        nanos as u64
    }
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
            let tiles = player.tile_count;
            let mul = player_label_scale(
                tiles,
                ui.config.ui.label_size_ref_tiles,
                ui.config.ui.label_max_scale,
            );
            let font_px = (base_font * mul).round().max(1.0);

            let label_text = format!("{}\n[{:.0}]", player.name, player.troops);
            commands.spawn((
                PlayerLabel(player.id),
                InGameEntity,
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

        if player.tile_count > 0 {
            let cx = (player.sum_x as f32 / player.tile_count as f32) * TILE_SIZE;
            let cy = (game.map.height as f32 - 1.0 - (player.sum_y as f32 / player.tile_count as f32)) * TILE_SIZE;
            
            let target = Vec3::new(cx, cy + ui.config.ui.label_offset_y, 150.0);
            
            // Just jump directly to target if too far, otherwise float smoothly
            if transform.translation.distance(target) > 100.0 {
                transform.translation = target;
            } else {
                transform.translation = transform.translation.lerp(target, 0.2);
            }

            let base_font = ui.config.ui.font_size;
            let mul = player_label_scale(
                player.tile_count,
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

/// Small burst used when an attack tile is hit — defined in Rust so WASM does not fetch `*.ron`.
fn impact_particle_effect() -> Particle2dEffect {
    Particle2dEffect {
        spawn_rate: 80.0,
        spawn_amount: 3,
        emission_shape: EmissionShape::Circle(5.0),
        lifetime: Rval::new(0.28, 0.06),
        linear_speed: Some(Rval(90.0, 0.35)),
        direction: Some(Rval(Vec2::Y, 1.0)),
        scale: Some(Rval(3.0, 0.35)),
        linear_damp: Some(Rval(3.5, 0.15)),
        ..Default::default()
    }
}

fn setup_combat_vfx(mut commands: Commands, mut effects: ResMut<Assets<Particle2dEffect>>) {
    let handle = effects.add(impact_particle_effect());
    commands.insert_resource(CombatVfxCache(handle));
}

fn system_spawn_attack_vfx(
    mut commands: Commands,
    q_attacks: Query<(Entity, &AttackExecution)>,
    q_vfx: Query<&CombatVfx>,
    vfx_cache: Res<CombatVfxCache>,
    game: Res<GameState>,
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
        let wy = (game.map.height as f32 - active_target.y as f32 - 0.5) * TILE_SIZE;

        let spawn_pos = Vec2::new(wx, wy);

        commands.spawn((
            CombatVfx {
                target_exec: attack_ent,
            },
            InGameEntity,
            Transform::from_xyz(spawn_pos.x, spawn_pos.y, 110.0),
            Visibility::default(),
            InheritedVisibility::default(),
            ViewVisibility::default(),
            ParticleSpawner::<ColorParticle2dMaterial>::default(),
            ParticleEffectHandle(vfx_cache.0.clone()),
        ));
    }
}

fn system_sync_attack_vfx_position(
    mut commands: Commands,
    q_attacks: Query<&AttackExecution>,
    mut q_vfx: Query<(Entity, &CombatVfx, &mut Transform)>,
    game: Res<GameState>,
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
                let wy = (game.map.height as f32 - front.y as f32 - 0.5) * TILE_SIZE;
                
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

fn system_cleanup_ingame_entities(
    mut commands: Commands,
    mut ready: ResMut<GameReady>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
    mut highest: ResMut<HighestReceivedTurn>,
    mut outgoing_map: ResMut<OutgoingAttackRowMap>,
    mut fresh_tiles: ResMut<FreshlyPaintedTiles>,
    q_ingame: Query<Entity, With<InGameEntity>>,
    q_execs: Query<Entity, With<AttackExecution>>,
) {
    ready.0 = false;
    queue.0.clear();
    pending.0 = 0;
    highest.0 = None;
    outgoing_map.0.clear();
    fresh_tiles.tiles.clear();
    
    for e in q_ingame.iter() {
        commands.entity(e).despawn();
    }
    for e in q_execs.iter() {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<GameState>();
    commands.remove_resource::<SinglePlayerState>();
    commands.remove_resource::<LocalIntentBuffer>();
}

#[derive(Component)]
pub struct GameOverPanel;

#[derive(Component)]
pub struct GoBackButton;

fn system_evaluate_game_over(
    mut commands: Commands,
    game: Option<Res<GameState>>,
    ready: Res<GameReady>,
    my_id: Res<MyPlayerId>,
    settings: Option<Res<ClientSettings>>,
    panel_q: Query<Entity, With<GameOverPanel>>,
) {
    let Some(game) = game else { return; };
    if !ready.0 { return; }
    if panel_q.iter().next().is_some() { return; }

    let alive_count = game.players.iter().filter(|p| p.alive).count();
    let am_i_alive = game.players.iter().find(|p| p.id == my_id.0).map(|p| p.alive).unwrap_or(false);

    let mut show_panel = false;
    let mut victory = false;

    if alive_count <= 1 {
        show_panel = true;
        victory = am_i_alive;
    } else if !am_i_alive && my_id.0 != 0 {
        show_panel = true;
        victory = false;
    }

    if show_panel {
        if let Some(settings) = settings {
            let font = settings.font.clone();
        let title = if victory { "VICTORY" } else { "DEFEAT" };
        let title_color = if victory { ui_theme::accent_primary() } else { ui_theme::accent_danger() };

        commands.spawn((
            GameOverPanel,
            InGameEntity,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(GAME_OVER),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
        )).with_children(|p| {
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(40.0)),
                    border_radius: BorderRadius::all(Val::Px(16.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(ui_theme::panel_bg()),
                BorderColor::all(ui_theme::panel_border()),
            )).with_children(|modal| {
                modal.spawn((
                    Text::new(title),
                    TextFont { font: font.clone(), font_size: 64.0, ..default() },
                    TextColor(title_color),
                    Node { margin: UiRect::bottom(Val::Px(30.0)), ..default() }
                ));

                modal.spawn((
                    Button,
                    GoBackButton,
                    Node {
                        padding: UiRect::axes(Val::Px(40.0), Val::Px(20.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(ui_theme::accent_home()),
                )).with_children(|btn| {
                    btn.spawn((
                        Text::new("GO BACK TO LOBBY"),
                        TextFont { font: font.clone(), font_size: 24.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                });
            });
        });
        }
    }
}

pub fn system_handle_go_back(
    mut interaction_q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<GoBackButton>)>,
    mut next_state: ResMut<NextState<AppState>>,
    mut senders: Query<&mut MessageSender<ClientLeaveMessage>>,
    mut joined_lobby: Option<ResMut<JoinedLobby>>,
) {
    for (interaction, mut bg) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                if let Some(jl) = joined_lobby.as_mut() {
                    jl.0 = None;
                }
                for mut sender in senders.iter_mut() {
                    let _ = sender.send::<dark_rift_shared::protocol::ReliableChannel>(ClientLeaveMessage);
                }
                next_state.set(AppState::LobbyBrowser);
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(ui_theme::accent_home_hover());
            }
            Interaction::None => {
                *bg = BackgroundColor(ui_theme::accent_home());
            }
        }
    }
}

/// Headless client logic for the bot.
fn system_headless_bot_engine(
    mut bot_state: ResMut<HeadlessBotState>,
    game: Res<GameState>,
    mut sender: Query<&mut MessageSender<ClientGameplayMessage>>,
    my_id: Res<MyPlayerId>,
) {
    if !bot_state.timer.tick(std::time::Duration::from_secs_f64(1.0 / 60.0)).just_finished() {
        return;
    }

    let Some(mut sender_mut) = sender.iter_mut().next() else {
        return;
    };

    let Some(_player) = game.player(my_id.0) else {
        return;
    };

    let bot_id = my_id.0;
    
    // Exact same logic ported from `intent.rs` to generate an attack via ClientGameplayMessage.
    let trigger_ratio = bot_state.profile.trigger_ratio;
    let reserve_ratio = bot_state.profile.reserve_ratio;
    let expand_ratio = bot_state.profile.expand_ratio;
    
    let player = game.player(bot_id).unwrap();
    if player.border_tiles.is_empty() {
        return;
    }

    let mut has_neutral = false;
    let mut player_targets = Vec::new();

    for &(bx, by) in &player.border_tiles {
        for (nx, ny) in game.map.neighbors(bx, by) {
            let owner = game.map.owner_id(nx, ny);
            if owner != bot_id {
                if owner == 0 {
                    has_neutral = true;
                } else {
                    player_targets.push(owner);
                }
            }
        }
    }

    let mut bot_rng = PseudoRandom::new(game.seed.wrapping_add(game.tick as u64).wrapping_add(1999));

    let (target_owner, is_neutral) = if has_neutral {
        (0, true)
    } else {
        if player_targets.is_empty() {
            return;
        }
        (player_targets[bot_rng.next_int(0, player_targets.len() as i32) as usize], false)
    };

    if !is_neutral {
        let trigger_limit = player.max_troops * trigger_ratio;
        if player.troops < trigger_limit {
            return;
        }
    }

    let active_reserve_ratio = if is_neutral { expand_ratio } else { reserve_ratio };
    let reserve = player.max_troops * active_reserve_ratio;
    let p_send = (player.troops - reserve).max(0.0);
    
    if p_send < dark_rift_shared::game_config::GameConfig::default().expand_cost as f64 {
        return;
    }

    info!("🤖 Headless sent Attack -> Player {} (troops: {})", target_owner, p_send);
    let _ = sender_mut.send::<ReliableChannel>(ClientGameplayMessage {
        intent: dark_rift_shared::protocol::GameplayIntent::Attack(dark_rift_shared::protocol::AttackIntent {
            target_owner,
            troops: Some(p_send),
        }),
    });
}


