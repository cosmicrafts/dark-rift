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
use dark_rift_shared::building::{
    compute_buildables_for_player, existing_structure_positions_with_width, find_upgrade_target_id,
    patch_buildable_entries_gold_and_counts,
    resolve_structure_spawn_tile, structure_build_cost_gold, system_building_construction,
    BuildableEntry, Building,
};
use dark_rift_shared::game::{BuildingKind, GameEvent, GameState};
use dark_rift_shared::player::Player;
use dark_rift_shared::execution::{
    system_attack_execution,
    system_player_income,
    system_game_tick,
    system_warp_fleet_execution,
    AttackExecution,
};
use dark_rift_shared::intent::{apply_stamped_intent, system_bot_think};
use dark_rift_shared::pathfinding::WaterPathfinderScratch;
use dark_rift_shared::protocol::*;
use dark_rift_shared::warp_fleet::{
    player_water_components, resolve_fleet_route, shares_border_with, WarpFleet,
};
use dark_rift_shared::water_components::WaterComponents;
use dark_rift_shared::pseudo_random::PseudoRandom;

mod border_material;
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

use bevy::sprite_render::MeshMaterial2d;

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
pub struct PinchState {
    pub last_distance: Option<f32>,
}

#[derive(Resource, Default)]
pub struct TouchGestureState {
    pub max_touches_in_gesture: u8,
    pub last_touch_pos: Option<Vec2>,
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

/// Build grid + HOME dock: pointer hits here must not reach the map (see [`player_click`]).
#[derive(Component)]
struct ActionDockPointerSink;

#[derive(Component)]
struct ExitButton;

#[derive(Component)]
struct SettingsButton;

#[derive(Component)]
struct GameTimerText;

#[derive(Component)]
struct FpsText;

#[derive(Resource)]
struct OverlayDiagnostics {
    status_line: String,
    fps_value: String,
    ping_value: String,
    /// Build mode hint (structures); shown in the FPS bar line.
    pub build_hint: String,
}

impl Default for OverlayDiagnostics {
    fn default() -> Self {
        Self {
            status_line: "Connecting to server...".to_string(),
            fps_value: "…".to_string(),
            ping_value: "…".to_string(),
            build_hint: String::new(),
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

/// Meshes for local-only transport visuals (boat disc, wake speck, landing ring).
#[derive(Resource)]
struct FleetVisualMeshes {
    boat: Handle<Mesh>,
    wake_dot: Handle<Mesh>,
    target_ring: Handle<Mesh>,
}

/// Placement ring + optional SAM / DefensePost range preview meshes (world units use [`TILE_SIZE`]).
#[derive(Resource)]
struct StructureMarkerMeshes {
    place_ring: Handle<Mesh>,
    sam_range: Handle<Mesh>,
    defense_range: Handle<Mesh>,
}

#[derive(Resource, Default)]
struct BuildableHudState {
    entries: Vec<BuildableEntry>,
}

#[derive(Component)]
struct BuildPillRow;

#[derive(Component)]
struct BuildPillButton(pub BuildingKind);

#[derive(Component, Clone, Copy)]
struct BuildPillLabel {
    /// 1 = level total, 2 = gold cost, 3 = hotkey digit (icon is separate `ImageNode`).
    slot: u8,
    kind: BuildingKind,
}

#[derive(Component)]
struct GhostRangeRing;

#[derive(Resource, Default)]
struct StructureGhostParts {
    marker: Option<Entity>,
    range: Option<Entity>,
}

#[derive(Component)]
struct BuildingVisualDone;

#[derive(Component)]
struct BuildingMapVisualRoot;

#[derive(Component)]
struct StructureGhostRoot;

/// Selected structure from HUD pill / hotkey `1`–`6`; `Esc` clears. LMB on owned tile places.
#[derive(Resource, Default)]
struct BuildStructureMode(pub Option<BuildingKind>);

/// Follows `WarpFleet.current_tile` for the local player.
#[derive(Component)]
struct FollowFleetBoat {
    fleet: Entity,
    wake: Entity,
    prev_tile: Option<u32>,
}

/// One-tile “wake” behind the boat (OpenFront `UnitLayer` trail simplified).
#[derive(Component)]
struct FollowFleetWake;

/// Fixed at landing tile until the fleet entity is gone.
#[derive(Component)]
struct FollowFleetTarget {
    fleet: Entity,
}

/// Client already spawned boat/target meshes for this fleet.
#[derive(Component)]
struct FleetClientVisuals;

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


/// "⚔ NN% (MMM trp)" readout next to the slider rail.
#[derive(Component)]
struct AttackRatioReadout;

#[derive(Component)]
struct BottomHudCap;

#[derive(Component)]
struct BottomHudTroops;

#[derive(Component)]
struct BottomHudGold;

#[derive(Component)]
struct EventFeedContainer;

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

/// Outgoing `WarpFleet` rows (OpenFront transport list); keys are `WarpFleet.id`.
#[derive(Resource, Default)]
struct OutgoingFleetRowMap(HashMap<u64, (Entity, Entity)>);

#[derive(Component)]
struct OutgoingFleetRowRoot;

#[derive(Component)]
struct OutgoingFleetLineText;

#[derive(Component)]
struct CancelFleetButton(u64);

/// Slider inner rail (fixed height in design space; scaled by `combat_hud_scale`).
#[derive(Component)]
struct AttackRatioBarShell;

#[inline]
fn clamp_ui_scale(s: f32) -> f32 {
    s.clamp(0.25, 8.0)
}

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
            .init_resource::<OutgoingAttackRowMap>()
            .init_resource::<OutgoingFleetRowMap>()
            .init_resource::<WaterPathfinderScratch>()
            .init_resource::<WaterComponents>()
            .init_resource::<CombatDockUiScale>();

        // `StartGameParams` includes `ClientSettings` (HUD scale / attack ratio); headless still runs `start_game_core`.
        app.insert_resource(ClientSettings {
            config: build_client_settings_config(),
            font: Handle::default(),
        });

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
        app.add_plugins(DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Dark Rift — Multiplayer".to_string(),
                    resolution: (1280u32, 720u32).into(),
                    fit_canvas_to_parent: true,
                    ..default()
                }),
                ..default()
            })
            // Disable AudioPlugin: no audio is used, and on WASM it crashes due to
            // browser autoplay policy blocking AudioBufferSourceNode.start() before
            // user gesture (cpal/rodio → bevy_audio fatal error).
            .disable::<bevy::audio::AudioPlugin>()
        );

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
        .init_resource::<PinchState>()
        .init_resource::<TouchGestureState>()
        .init_resource::<FreshlyPaintedTiles>()
            .init_resource::<AttackRatio>()
            .init_resource::<OutgoingAttackRowMap>()
            .init_resource::<OutgoingFleetRowMap>()
            .init_resource::<NetTiming>()
        .init_resource::<OverlayDiagnostics>()
        .init_resource::<UiShellState>()
        .init_resource::<CombatDockUiScale>()
        .init_resource::<WaterPathfinderScratch>()
        .init_resource::<WaterComponents>()
        .init_resource::<BuildStructureMode>()
        .init_resource::<StructureGhostParts>()
        .init_resource::<BuildableHudState>();

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
                setup_structure_marker_meshes,
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
               .add_plugins(bevy::sprite_render::Material2dPlugin::<border_material::BorderMaterial>::default())
               .init_resource::<CombatVfxCache>();
        }

        app.add_systems(
                Startup,
                (
                    setup_network,
                    setup_attack_ring_mesh,
                    setup_fleet_visual_meshes,
                    setup_structure_marker_meshes,
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
                ui_lobby_browser::system_nickname_keyboard_when_focused,
                ui_lobby_browser::system_nickname_cursor_blink,
                ui_lobby_browser::system_nickname_autosave_tick,
                system_lobby_main_menu_anim,
                system_update_lobby_browser,
            ).run_if(|state: Res<State<AppState>>, cli: Res<Cli>| *state.get() == AppState::LobbyBrowser && !cli.bot))
            .add_systems(
                Update,
                (
                    ui_lobby_browser::system_apply_lobby_layout_responsive,
                    ui_lobby_browser::system_apply_lobby_controls_responsive,
                )
                    .run_if(|state: Res<State<AppState>>, cli: Res<Cli>| *state.get() == AppState::LobbyBrowser && !cli.bot)
                    .before(system_lobby_main_menu_anim),
            )
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
            system_refresh_buildable_hud,
            system_apply_buildable_hud.after(system_refresh_buildable_hud),
            system_buildpill_interactions,
            system_build_structure_hotkeys,
            system_sync_structure_ghost,
            system_attack_ratio_keyboard,
            system_attack_ratio_slider_drag,
            player_click,
            system_force_boat_key,
            system_exit_button,
            system_home_button,
            update_hud,
            system_apply_responsive_hud_layout,
            system_sync_bottom_combat_hud,
            system_update_event_feed,
            system_update_scroll_position,
            system_sync_outgoing_attack_rows,
            system_handle_cancel_click,
            system_handle_recall_fleet_click,
        ).run_if(in_state(AppState::InGame)));

        app.add_systems(Update,
            (
                ui_leaderboard::system_update_leaderboard.run_if(bevy::time::common_conditions::on_timer(std::time::Duration::from_secs(1))),
                ui_leaderboard::system_handle_leaderboard_toggle,
            ).run_if(in_state(AppState::InGame))
        );

        // ── GPU shader map rendering ──
        // handle_tile_change now writes owner IDs into the ShaderStorageBuffer (2 bytes/tile),
        // eliminating the old chunk texture uploads entirely.
        app.add_systems(Update, (
            handle_tile_change,
            update_water_animation_time,
        ).run_if(in_state(AppState::InGame)));
        
        app.add_systems(
            Update,
            system_animate_attack_command_rings
                .after(player_click)
                .run_if(in_state(AppState::InGame)),
        );

        app.add_systems(
            Update,
            (
                system_spawn_fleet_visuals,
                system_sync_fleet_visuals,
                system_attach_building_visuals,
                system_sync_building_visual_lod,
                system_sync_building_map_labels,
            )
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
        system_warp_fleet_execution,
        system_attack_execution,
        system_building_construction,
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
    mut q_fleet: Query<(Entity, &mut WarpFleet)>,
    mut q_building: Query<(Entity, &mut Building)>,
    mut path_scratch: ResMut<WaterPathfinderScratch>,
    water_components: Res<WaterComponents>,
) {
    if let Some(turn) = queue.0.pop_front() {
        for (i, stamped) in turn.intents.iter().enumerate() {
            apply_stamped_intent(
                &mut commands,
                &mut *game,
                &mut q_exec,
                &mut q_fleet,
                &mut q_building,
                stamped,
                i as u32,
                &mut *path_scratch,
                &water_components,
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
    mut pinch_state: ResMut<PinchState>,
    mut touch_gesture: ResMut<TouchGestureState>,
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
    if touch_count > 0 {
        if touch_gesture.max_touches_in_gesture == 0 || touches.any_just_pressed() {
            touch_gesture.max_touches_in_gesture = touch_count as u8;
        } else {
            touch_gesture.max_touches_in_gesture =
                touch_gesture.max_touches_in_gesture.max(touch_count as u8);
        }
    }
    if let Some(touch) = touches.iter().next() {
        touch_gesture.last_touch_pos = Some(touch.position());
    } else if !touches.any_just_released() {
        touch_gesture.last_touch_pos = None;
        touch_gesture.max_touches_in_gesture = 0;
    }

    // Two-finger pinch zoom (mobile/tablet). Keep this branch isolated so
    // desktop wheel zoom and one-finger pan semantics remain unchanged.
    let mut touch_a = None;
    let mut touch_b = None;
    let mut extra_touches = false;
    for touch in touches.iter() {
        if touch_a.is_none() {
            touch_a = Some(touch.position());
        } else if touch_b.is_none() {
            touch_b = Some(touch.position());
        } else {
            extra_touches = true;
            break;
        }
    }
    let is_pinching = touch_a.is_some() && touch_b.is_some() && !extra_touches;
    let mut pinch_zoom = 0.0_f32;
    if let (Some(a), Some(b)) = (touch_a, touch_b) {
        let distance = a.distance(b);
        if is_pinching {
            if let Some(prev_distance) = pinch_state.last_distance {
                let delta = distance - prev_distance;
                pinch_zoom = -delta * 0.004;
            }
            pinch_state.last_distance = Some(distance);
            // Treat pinch as non-tap interaction.
            drag_state.accumulated_drag = drag_state.accumulated_drag.max(11.0);
        } else {
            pinch_state.last_distance = None;
        }
    } else {
        pinch_state.last_distance = None;
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
        
        zoom += pinch_zoom;
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
        if drag_state.is_dragging && !is_pinching {
            let mut raw_drag = Vec2::ZERO;
            let drag_delta = if touches.iter().count() > 0 {
                // Prefer touch on mobile (avoids DPI-scaled emulated MouseMotion spikes)
                raw_drag = touch_delta;
                let _ = motion_evr.read().count(); // Drain mouse events
                raw_drag * 0.5 // Dampen mobile sensitivity significantly
            } else {
                for ev in motion_evr.read() {
                    raw_drag += ev.delta;
                }
                raw_drag * 0.85 // Slightly dampen desktop mouse
            };

            drag_state.accumulated_drag += raw_drag.length();

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
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<border_material::BorderMaterial>>,
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
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(12.0),
                ..default()
            },
        ))
        .with_children(|right_group| {
            // Dev Diagnostics (FPS, Ping, Status) Pill
            right_group.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.8)),
            )).with_children(|pill| {
                pill.spawn((
                    FpsText,
                    Text::new("Connecting…"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 15.0,
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

    let grid = spawn_shader_map(&mut commands, &game, &mut images, &mut materials, &mut meshes, false);
    commands.insert_resource(grid);

    commands.insert_resource(WaterComponents::compute(&game.map));
    commands.insert_resource(game);
    ready.0 = true;
}

fn setup_live(
    mut commands: Commands,
    _cli: Res<Cli>,
    settings: Res<ClientSettings>,
    asset_server: Res<AssetServer>,
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
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                margin: UiRect::left(Val::Auto),
                column_gap: Val::Px(12.0),
                ..default()
            },
        ))
        .with_children(|right_group| {
            // Dev Diagnostics (FPS, Ping, Status) Pill
            right_group.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.8)),
            )).with_children(|pill| {
                pill.spawn((
                    FpsText,
                    Text::new("Connecting…"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.4, 0.9, 0.4, 1.0)),
                ));
            });

            // Game Timer Pill
            right_group.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.8)),
            )).with_children(|pill| {
                pill.spawn((
                    GameTimerText,
                    Text::new("00:00"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.9, 0.9, 0.9, 1.0)),
                ));
            });

            // SETTINGS Button
            right_group.spawn((
                Button,
                SettingsButton,
                Node {
                    width: Val::Px(48.0),
                    height: Val::Px(48.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(panel_bg()),
                BorderColor::all(crate::ui_theme::panel_border()),
            ))
            .with_children(|b| {
                b.spawn((
                    ImageNode::new(asset_server.load("icons/home.png")),
                    Node {
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        ..default()
                    },
                ));
            });

            // EXIT Button
            right_group.spawn((
                Button,
                ExitButton,
                Node {
                    width: Val::Px(48.0),
                    height: Val::Px(48.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(crate::ui_theme::accent_danger()),
                BorderColor::all(crate::ui_theme::panel_border()),
            ))
            .with_children(|b| {
                b.spawn((
                    ImageNode::new(asset_server.load("icons/exit.png")),
                    Node {
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        ..default()
                    },
                ));
            });
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
            "Offline".to_string()
        } else if net.handshake_ms.is_none() && net.connect_started.is_some() {
            "Connecting…".to_string()
        } else if net.handshake_ms.is_some() {
            let ping = net.turn_spacing_ema_ms.max(1.0);
            format!("{ping:.0} ms")
        } else {
            "…".to_string()
        };
    }

    if !(refresh_fps || refresh_net) {
        return;
    }
}

/// StarCraft-style 3×2 build palette + HOME, anchored on [`HudShellRegions::action_dock`].
fn spawn_action_dock(
    commands: &mut Commands,
    shell: &HudShellRegions,
    observer: bool,
    hud_scale: f32,
    asset_server: &AssetServer,
    font: Handle<Font>,
) {
    let s = clamp_ui_scale(hud_scale);
    let vis = if observer {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    let (cell, gap, pad) = action_dock_grid_layout_sizes(s);
    let frame = 2.0 * s;
    let outer_w = action_dock_grid_outer_width_px(s);
    let icon = ((cell - 12.0 * s).max(20.0)).min(cell - 2.0);

    commands
        .spawn((
            ChildOf(shell.action_dock),
            ActionDockPointerSink,
            InGameEntity,
            RelativeCursorPosition::default(),
            vis,
            Node {
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::FlexEnd,
                column_gap: Val::Px(SPACE_2),
                ..default()
            },
        ))
        .with_children(|dock| {
            dock.spawn((
                BuildPillRow,
                Node {
                    width: Val::Px(outer_w),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(gap),
                    padding: UiRect::all(Val::Px(pad)),
                    border: UiRect::all(Val::Px(frame)),
                    border_radius: BorderRadius::all(Val::Px(6.0 * s)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.04, 0.05, 0.08, 0.92)),
                BorderColor::all(panel_border()),
            ))
            .with_children(|grid| {
                for row_i in 0..2 {
                    grid.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::Center,
                            column_gap: Val::Px(gap),
                            ..default()
                        },
                    ))
                    .with_children(|row| {
                        for c in 0..3 {
                            let idx = row_i * 3 + c;
                            let kind = BuildingKind::ALL[idx];
                            let hk = (idx + 1).to_string();
                            row.spawn((
                                Button,
                                BuildPillButton(kind),
                                Node {
                                    width: Val::Px(cell),
                                    height: Val::Px(cell),
                                    flex_direction: FlexDirection::Column,
                                    padding: UiRect::all(Val::Px(3.0 * s)),
                                    justify_content: JustifyContent::FlexStart,
                                    row_gap: Val::Px(1.0 * s),
                                    border_radius: BorderRadius::all(Val::Px(4.0 * s)),
                                    border: UiRect::all(Val::Px(1.0 * s)),
                                    ..default()
                                },
                                BackgroundColor(panel_bg()),
                                BorderColor::all(panel_border()),
                            ))
                            .with_children(|cell_ui| {
                                cell_ui.spawn((
                                    Node {
                                        width: Val::Percent(100.0),
                                        flex_direction: FlexDirection::Row,
                                        justify_content: JustifyContent::FlexEnd,
                                        min_height: Val::Px(11.0 * s),
                                        ..default()
                                    },
                                ))
                                .with_children(|hk_row| {
                                    hk_row.spawn((
                                        Text::new(hk),
                                        BuildPillLabel { slot: 3, kind },
                                        TextFont {
                                            font: font.clone(),
                                            font_size: 10.0 * s,
                                            ..default()
                                        },
                                        TextColor(Color::srgb(0.55, 0.62, 0.72)),
                                    ));
                                });
                                cell_ui.spawn((
                                    ImageNode::new(asset_server.load(building_kind_icon_path(kind))),
                                    Node {
                                        width: Val::Px(icon),
                                        height: Val::Px(icon),
                                        align_self: AlignSelf::Center,
                                        ..default()
                                    },
                                ));
                                cell_ui.spawn((
                                    Text::new("0"),
                                    BuildPillLabel { slot: 1, kind },
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 10.0 * s,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.85, 0.9, 1.0)),
                                ));
                                cell_ui.spawn((
                                    Text::new("0"),
                                    BuildPillLabel { slot: 2, kind },
                                    TextFont {
                                        font: font.clone(),
                                        font_size: 9.5 * s,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.75, 0.78, 0.82)),
                                ));
                            });
                        }
                    });
                }
            });

            dock.spawn((
                Node {
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                Button,
                HomeButton,
                BackgroundColor(accent_home()),
                BorderColor::all(panel_border()),
            ))
            .with_children(|b| {
                b.spawn((
                    ImageNode::new(asset_server.load("icons/home.png")),
                    Node {
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        ..default()
                    },
                ));
            });
        });
}

/// Bottom-center stack: attack summary (OpenFront `AttacksDisplay`) + ratio slider row (`ControlPanel` row 2).
fn spawn_bottom_combat_hud(
    commands: &mut Commands,
    shell: &HudShellRegions,
    observer: bool,
    hud_scale: f32,
    asset_server: &AssetServer,
    font: Handle<Font>,
) {
    let s = clamp_ui_scale(hud_scale);
    let vis = if observer {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    spawn_action_dock(commands, shell, observer, hud_scale, asset_server, font);
    commands
        .spawn((
            ChildOf(shell.bottom_sheet),
            EventFeedContainer,
            InGameEntity,
            vis,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0 * s),
                bottom: Val::Px(16.0 * s),
                width: Val::Px(300.0 * s),
                height: Val::Px(200.0 * s),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                row_gap: Val::Px(4.0 * s),
                ..default()
            },
        ));

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
                row_gap: Val::Px(12.0 * s),
                padding: UiRect::bottom(Val::Px(16.0 * s)),
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                OutgoingRowsContainer,
                Node {
                    width: Val::Percent(96.0),
                    min_width: Val::Px(280.0 * s),
                    max_width: Val::Px(800.0 * s),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    align_content: AlignContent::FlexStart,
                    row_gap: Val::Px(8.0 * s),
                    column_gap: Val::Px(8.0 * s),
                    max_height: Val::Px(280.0 * s),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                bevy::ui::ScrollPosition::default(),
            ));

            root.spawn((
                Node {
                    width: Val::Percent(96.0),
                    max_width: Val::Px(800.0 * s),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    row_gap: Val::Px(8.0 * s),
                    padding: UiRect::all(Val::Px(12.0 * s)),
                    border_radius: BorderRadius::all(Val::Px(12.0 * s)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.15, 0.85)),
            ))
            .with_children(|panel| {
                // Top row: Economy (Cap | Gold | Troops)
                panel.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: Val::Px(8.0 * s),
                        ..default()
                    },
                )).with_children(|eco_row| {
                    // Capacity Pill
                    eco_row.spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(12.0 * s), Val::Px(6.0 * s)),
                            border_radius: BorderRadius::all(Val::Px(8.0 * s)),
                            column_gap: Val::Px(6.0 * s),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)), // OpenFront bg-white/4
                    )).with_children(|pill| {
                        pill.spawn((
                            ImageNode::new(asset_server.load("icons/capacity.png")),
                            Node { width: Val::Px(16.0 * s), height: Val::Px(16.0 * s), ..default() }
                        ));
                        pill.spawn((
                            BottomHudCap,
                            Text::new("0"),
                            TextFont { font_size: 16.0 * s, ..default() },
                            TextColor(Color::WHITE),
                        ));
                        pill.spawn((
                            Text::new("Cap"),
                            TextFont { font_size: 14.0 * s, ..default() },
                            TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        ));
                    });

                    // Gold Pill
                    eco_row.spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(12.0 * s), Val::Px(6.0 * s)),
                            border_radius: BorderRadius::all(Val::Px(8.0 * s)),
                            column_gap: Val::Px(6.0 * s),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)),
                    )).with_children(|pill| {
                        pill.spawn((
                            ImageNode::new(asset_server.load("icons/gold.png")),
                            Node { width: Val::Px(16.0 * s), height: Val::Px(16.0 * s), ..default() }
                        ));
                        pill.spawn((
                            BottomHudGold,
                            Text::new("0"),
                            TextFont { font_size: 16.0 * s, ..default() },
                            TextColor(Color::WHITE),
                        ));
                        pill.spawn((
                            Text::new("Gold"),
                            TextFont { font_size: 14.0 * s, ..default() },
                            TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        ));
                    });

                    // Troops Pill
                    eco_row.spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(12.0 * s), Val::Px(6.0 * s)),
                            border_radius: BorderRadius::all(Val::Px(8.0 * s)),
                            column_gap: Val::Px(6.0 * s),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)), // OpenFront bg-white/4
                    )).with_children(|pill| {
                        pill.spawn((
                            ImageNode::new(asset_server.load("icons/troops.png")),
                            Node { width: Val::Px(16.0 * s), height: Val::Px(16.0 * s), ..default() }
                        ));
                        pill.spawn((
                            BottomHudTroops,
                            Text::new("0"),
                            TextFont { font_size: 16.0 * s, ..default() },
                            TextColor(Color::WHITE),
                        ));
                        pill.spawn((
                            Text::new("Troops"),
                            TextFont { font_size: 14.0 * s, ..default() },
                            TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        ));
                    });
                });

                // Bottom row: Attack Slider
                panel.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(16.0 * s),
                        ..default()
                    },
                )).with_children(|slider_row| {
                    slider_row.spawn((
                        AttackRatioReadout,
                        Text::new(""),
                        TextFont {
                            font_size: 18.0 * s,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Node {
                            width: Val::Px(180.0 * s),
                            ..default()
                        },
                    ));

                    slider_row.spawn((
                        Button,
                        AttackRatioTrack,
                        RelativeCursorPosition::default(),
                        Node {
                            flex_grow: 1.0,
                            height: Val::Px(32.0 * s),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Stretch,
                            ..default()
                        },
                    ))
                    .with_children(|track| {
                        track
                            .spawn((
                                AttackRatioBarShell,
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(16.0 * s),
                                    flex_direction: FlexDirection::Row,
                                    overflow: Overflow::clip(),
                                    border_radius: BorderRadius::all(Val::Px(8.0 * s)),
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

use bevy::ecs::system::SystemParam;

#[derive(SystemParam)]
pub struct StartGameParams<'w> {
    ready: ResMut<'w, GameReady>,
    my_id: ResMut<'w, MyPlayerId>,
    queue: ResMut<'w, TurnQueue>,
    pending: ResMut<'w, PendingTicks>,
    highest: ResMut<'w, HighestReceivedTurn>,
    overlay: Option<ResMut<'w, OverlayDiagnostics>>,
    shell: Option<Res<'w, HudShellRegions>>,
    settings: Res<'w, ClientSettings>,
    asset_server: Res<'w, AssetServer>,
    meshes: Option<ResMut<'w, Assets<Mesh>>>,
    materials: Option<ResMut<'w, Assets<border_material::BorderMaterial>>>,
    images: Option<ResMut<'w, Assets<Image>>>,
    next_state: ResMut<'w, NextState<AppState>>,
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
    mut cam_q: Query<&mut Transform, With<Camera2d>>,
    mut params: StartGameParams,
) {
    timeout.0.tick(time.delta());
    if timeout.0.just_finished() {
        bevy::log::error!("Map load timeout! Returning to lobby.");
        params.next_state.set(AppState::LobbyBrowser);
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
        &mut cam_q,
        &mut commands,
        &mut params,
    );

    commands.remove_resource::<PendingServerStart>();
}


fn system_mock_single_player_start(
    mock:      Option<ResMut<MockSinglePlayerStart>>,
    mut cam_q:     Query<&mut Transform, With<Camera2d>>,
    mut commands:  Commands,
    mut params:    StartGameParams,
) {
    if let Some(m) = mock {
        commands.insert_resource(SinglePlayerState {
            timer: Timer::from_seconds(0.040, TimerMode::Repeating),
            current_turn: 0,
        });
        commands.insert_resource(LocalIntentBuffer::default());
        start_game_core(
            m.0.clone(), &mut cam_q, &mut commands, &mut params
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
    cam_q:         &mut Query<&mut Transform, With<Camera2d>>,
    commands:      &mut Commands,
    params:        &mut StartGameParams,
) {
            if params.ready.0 { return; }

            let hud_s = clamp_ui_scale(params.settings.config.ui.combat_hud_scale);

            if let Some(pid) = msg.my_player_id {
                info!("🎮 Game starting! I am player {}", pid);
                params.my_id.0 = pid;
            } else {
                info!("👁  Game starting! I am an observer");
                params.my_id.0 = 0;
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
                params.queue.0.push_back(turn.clone());
                params.pending.0 += 1;
                params.highest.0 = Some(params.highest.0.unwrap_or(0).max(turn.turn_number));
            }

            commands.insert_resource(msg.config.clone());
            
            
            let map_w = msg.config.map_width as f32;
            let map_h = msg.config.map_height as f32;
            let mut cx = map_w * TILE_SIZE / 2.0;
            let mut cy = map_h * TILE_SIZE / 2.0;

            if params.my_id.0 != 0 {
                if let Some(me) = msg.players.iter().find(|p| p.id == params.my_id.0) {
                    cx = me.spawn_x as f32 * TILE_SIZE;
                    cy = (game.map.height as f32 - 1.0 - me.spawn_y as f32) * TILE_SIZE;
                }
            }

            if let Ok(mut cam) = cam_q.single_mut() {
                cam.translation = Vec3::new(cx, cy, 999.0);
            }
            if let Some(images_res) = params.images.as_mut() {
                let grid = spawn_shader_map(commands, &game, images_res, params.materials.as_mut().unwrap(), params.meshes.as_mut().unwrap(), true);
                commands.insert_resource(grid);
            }

            commands.insert_resource(WaterComponents::compute(&game.map));
            commands.insert_resource(game);
            params.ready.0 = true;
            params.next_state.set(AppState::InGame);

            commands.insert_resource(CombatDockUiScale(hud_s));
            if let Some(s) = &params.shell {
                spawn_bottom_combat_hud(
                    commands,
                    s,
                    msg.my_player_id.is_none(),
                    hud_s,
                    params.asset_server.as_ref(),
                    params.settings.font.clone(),
                );
            }

            if let Some(over) = params.overlay.as_mut() {
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

fn setup_attack_ring_mesh(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    settings: Res<ClientSettings>,
) {
    let s = clamp_ui_scale(settings.config.ui.click_feedback_scale);
    let mesh = Annulus::new(5.0 * s, 8.8 * s)
        .mesh()
        .resolution(56)
        .build();
    commands.insert_resource(AttackRingMesh(meshes.add(mesh)));
}

fn setup_fleet_visual_meshes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    settings: Res<ClientSettings>,
) {
    let sb = clamp_ui_scale(settings.config.ui.fleet_boat_scale);
    let sw = clamp_ui_scale(settings.config.ui.fleet_wake_scale);
    let st = clamp_ui_scale(settings.config.ui.fleet_target_ring_scale);
    let boat = Annulus::new(0.35 * sb, 2.9 * sb)
        .mesh()
        .resolution(28)
        .build();
    let wake_dot = Annulus::new(0.1 * sw, 1.15 * sw)
        .mesh()
        .resolution(14)
        .build();
    let target_ring = Annulus::new(4.0 * st, 7.4 * st)
        .mesh()
        .resolution(52)
        .build();
    commands.insert_resource(FleetVisualMeshes {
        boat: meshes.add(boat),
        wake_dot: meshes.add(wake_dot),
        target_ring: meshes.add(target_ring),
    });
}

fn setup_structure_marker_meshes(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let place_ring = Annulus::new(0.12, 1.35)
        .mesh()
        .resolution(20)
        .build();
    let sam_r = config::SAM_RANGE_TILES as f32 * TILE_SIZE;
    let sam_range = Annulus::new((sam_r - 0.28).max(0.12), sam_r + 0.28)
        .mesh()
        .resolution(36)
        .build();
    let def_r = config::DEFENSE_POST_RANGE as f32 * TILE_SIZE;
    let defense_range = Annulus::new((def_r - 0.22).max(0.12), def_r + 0.22)
        .mesh()
        .resolution(30)
        .build();
    commands.insert_resource(StructureMarkerMeshes {
        place_ring: meshes.add(place_ring),
        sam_range: meshes.add(sam_range),
        defense_range: meshes.add(defense_range),
    });
}

/// Text row looks up the parent [`Building`] via this entity id.
#[derive(Component)]
struct BuildingLabelTarget(pub Entity);

#[inline]
fn tile_idx_to_world(idx: u32, map_w: u32, map_h: u32) -> Vec3 {
    let x = idx % map_w;
    let y = idx / map_w;
    let wx = (x as f32 + 0.5) * TILE_SIZE;
    let wy = (map_h as f32 - y as f32 - 0.5) * TILE_SIZE;
    Vec3::new(wx, wy, 0.0)
}

/// Returns `LaunchFleet` only if [`resolve_fleet_route`] succeeds (same as simulation).
fn try_launch_fleet_intent(
    game: &GameState,
    wc: &WaterComponents,
    path_scratch: &mut WaterPathfinderScratch,
    pid: u16,
    target_owner: u16,
    target_tile: u32,
    troops: f64,
    log_ctx: &str,
) -> Option<GameplayIntent> {
    let Some(launcher) = game.player(pid) else {
        bevy::log::warn!("{log_ctx}: launcher player {pid} not found");
        return None;
    };
    let border = launcher.border_tiles.as_slice();
    let target_border = if target_owner != 0 {
        match game.player(target_owner) {
            Some(p) => Some(p.border_tiles.as_slice()),
            None => {
                bevy::log::warn!("{log_ctx}: target_owner {target_owner} not found");
                return None;
            }
        }
    } else {
        None
    };
    match resolve_fleet_route(
        &game.map,
        wc,
        path_scratch,
        pid,
        target_owner,
        target_tile,
        border,
        target_border,
        true,
    ) {
        Ok(_) => Some(GameplayIntent::LaunchFleet {
            target_owner,
            target_tile,
            troops,
        }),
        Err(e) => {
            bevy::log::warn!("{log_ctx}: {e}");
            None
        }
    }
}

fn spawn_attack_command_ring(
    commands: &mut Commands,
    ring_mesh: &AttackRingMesh,
    materials: &mut Assets<ColorMaterial>,
    settings: &ClientSettings,
    player_color: [f32; 3],
    x: u32,
    y: u32,
    map_h: u32,
    send: f64,
) {
    let wx = (x as f32 + 0.5) * TILE_SIZE;
    let wy = (map_h as f32 - y as f32 - 0.5) * TILE_SIZE;
    let [r, g, b] = player_color;
    let mat = materials.add(ColorMaterial::from_color(Color::srgba(r, g, b, 0.97)));
    let label = format!("{:.0}", send);
    let ls = clamp_ui_scale(settings.config.ui.click_troop_label_scale);
    let ox = TILE_SIZE * 1.75 * ls;
    let oy = TILE_SIZE * 0.35 * ls;
    let troop_font = 28.0 * ls;
    commands
        .spawn((
            AttackCommandRing {
                elapsed: 0.0,
                rgb: player_color,
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
                    font_size: troop_font,
                    weight: FontWeight::BOLD,
                    ..default()
                },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.98)),
                TextLayout::new_with_justify(Justify::Left),
                Transform::from_xyz(ox, oy, 2.0),
            ));
        });
}

fn send_gameplay_intent(
    intent: GameplayIntent,
    pid: u16,
    local_buf: Option<ResMut<LocalIntentBuffer>>,
    senders: &mut Query<&mut MessageSender<ClientGameplayMessage>>,
) {
    if let Some(mut buf) = local_buf {
        buf.0.push(StampedIntent {
            player_id: pid,
            intent,
        });
    } else {
        for mut sender in senders.iter_mut() {
            let _ = sender.send::<ReliableChannel>(ClientGameplayMessage { intent: intent.clone() });
        }
    }
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

fn system_build_structure_hotkeys(
    kb: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<BuildStructureMode>,
    mut overlay: ResMut<OverlayDiagnostics>,
) {
    if kb.just_pressed(KeyCode::Escape) {
        mode.0 = None;
    }
    let keys = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
    ];
    for (i, key) in keys.iter().enumerate() {
        if kb.just_pressed(*key) {
            mode.0 = Some(BuildingKind::ALL[i]);
            break;
        }
    }
    overlay.build_hint = match mode.0 {
        None => String::new(),
        Some(k) => format!(
            "{} · 1–6 · Esc · click own tile (pays gold)",
            k.as_str()
        ),
    };
}

fn system_refresh_buildable_hud(
    game_opt: Option<Res<GameState>>,
    my_id: Res<MyPlayerId>,
    q_building: Query<&Building>,
    mut removed: RemovedComponents<Building>,
    added: Query<(), Added<Building>>,
    mut state: ResMut<BuildableHudState>,
    mut last_gold: Local<Option<f64>>,
) {
    let Some(game) = game_opt.as_deref() else {
        return;
    };
    let pid = my_id.0;
    if pid == 0 {
        return;
    }
    let Some(p) = game.player(pid) else {
        return;
    };
    let buildings_changed = removed.read().count() > 0 || added.iter().next().is_some();
    let gold_changed = last_gold.map(|g| (p.gold - g).abs() > 0.5).unwrap_or(true);
    let buildings: Vec<Building> = q_building.iter().copied().collect();

    if buildings_changed || state.entries.len() != BuildingKind::ALL.len() {
        state.entries = compute_buildables_for_player(&game.map, pid, p.gold, &buildings)
            .into_iter()
            .collect();
        *last_gold = Some(p.gold);
        return;
    }

    if gold_changed && state.entries.len() == BuildingKind::ALL.len() {
        patch_buildable_entries_gold_and_counts(&mut state.entries, pid, p.gold, &buildings);
        *last_gold = Some(p.gold);
    }
}

fn system_apply_buildable_hud(
    state: Res<BuildableHudState>,
    mode: Res<BuildStructureMode>,
    mut q_btn: Query<
        (
            &BuildPillButton,
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<Button>,
    >,
    mut q_label: Query<(&BuildPillLabel, &mut Text)>,
) {
    if state.entries.len() != BuildingKind::ALL.len() {
        return;
    }
    for (label, mut text) in q_label.iter_mut() {
        let Some(entry) = state.entries.iter().find(|e| e.kind == label.kind) else {
            continue;
        };
        let hk = BuildingKind::ALL
            .iter()
            .position(|&k| k == entry.kind)
            .map(|i| (i + 1).to_string())
            .unwrap_or_else(|| "?".into());
        let s = match label.slot {
            1 => format!("{}", entry.level_total),
            2 => format!("{:.0}", entry.cost),
            3 => hk,
            _ => continue,
        };
        if **text != s {
            **text = s;
        }
    }
    for (btn, interaction, mut bg, mut border) in q_btn.iter_mut() {
        let Some(entry) = state.entries.iter().find(|e| e.kind == btn.0) else {
            continue;
        };
        let disabled = !entry.can_build && !entry.can_upgrade;
        let sel = mode.0 == Some(btn.0);
        if disabled {
            *bg = Color::srgba(0.1, 0.11, 0.13, 0.38).into();
            *border = BorderColor::all(Color::srgba(0.3, 0.32, 0.36, 0.35));
        } else if sel {
            *bg = accent_primary().with_alpha(0.42).into();
            *border = BorderColor::all(accent_primary());
        } else if *interaction == Interaction::Hovered {
            *bg = panel_bg().with_alpha(0.92).into();
            *border = BorderColor::all(panel_border());
        } else {
            *bg = panel_bg().into();
            *border = BorderColor::all(panel_border());
        }
    }
}

fn system_buildpill_interactions(
    q: Query<(&Interaction, &BuildPillButton), (Changed<Interaction>, With<Button>)>,
    mut mode: ResMut<BuildStructureMode>,
    mut overlay: ResMut<OverlayDiagnostics>,
) {
    for (ix, btn) in &q {
        if *ix == Interaction::Pressed {
            mode.0 = Some(btn.0);
            overlay.build_hint = format!("Place {} — click own tile (gold) · Esc", btn.0.as_str());
        }
    }
}

#[inline]
fn building_kind_short(kind: BuildingKind) -> &'static str {
    match kind {
        BuildingKind::City => "C",
        BuildingKind::Factory => "F",
        BuildingKind::Port => "P",
        BuildingKind::DefensePost => "D",
        BuildingKind::SamLauncher => "S",
        BuildingKind::MissileSilo => "M",
    }
}

fn building_kind_icon_path(kind: BuildingKind) -> &'static str {
    match kind {
        BuildingKind::City => "icons/building_city.png",
        BuildingKind::Factory => "icons/building_factory.png",
        BuildingKind::Port => "icons/building_port.png",
        BuildingKind::DefensePost => "icons/building_defense.png",
        BuildingKind::SamLauncher => "icons/building_sam.png",
        BuildingKind::MissileSilo => "icons/building_silo.png",
    }
}

fn system_sync_structure_ghost(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    my_id: Res<MyPlayerId>,
    mode: Res<BuildStructureMode>,
    hovered: Res<HoveredTile>,
    meshes: Option<Res<StructureMarkerMeshes>>,
    mut ghost_parts: ResMut<StructureGhostParts>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    q_building: Query<(Entity, &Building)>,
) {
    let despawn_all = |commands: &mut Commands, parts: &mut StructureGhostParts| {
        if let Some(e) = parts.marker.take() {
            commands.entity(e).despawn();
        }
        if let Some(e) = parts.range.take() {
            commands.entity(e).despawn();
        }
    };

    let Some(game) = game_opt.as_deref() else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };
    let Some(ms) = meshes.as_ref() else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };
    let Some(kind) = mode.0 else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };
    let Some((x, y)) = hovered.0 else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };

    let pid = my_id.0;
    if pid == 0 {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    }
    let Some(player) = game.player(pid) else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };

    let w = game.map.width;
    let mh = game.map.height;
    let click_idx = y * w + x;
    if game.map.owner_id(x, y) != pid {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    }

    let snapshot: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();
    let positions = existing_structure_positions_with_width(snapshot.iter(), w, None);
    let cost = structure_build_cost_gold(kind, pid, &snapshot);
    let afford = player.gold >= cost && cost.is_finite();

    enum GhostKind {
        Upgrade,
        Place,
        Invalid,
    }

    let (tile_idx, ghost_kind, ok_afford) =
        if let Some(bid) = find_upgrade_target_id(&game.map, pid, kind, click_idx, &snapshot) {
            let mut ti = None;
            for b in &snapshot {
                if b.id == bid {
                    ti = Some(b.tile_idx);
                    break;
                }
            }
            let Some(ti) = ti else {
                despawn_all(&mut commands, &mut ghost_parts);
                return;
            };
            (ti, GhostKind::Upgrade, afford)
        } else if let Some(spawn) =
            resolve_structure_spawn_tile(&game.map, pid, kind, click_idx, &positions)
        {
            (spawn, GhostKind::Place, afford)
        } else {
            (click_idx, GhostKind::Invalid, false)
        };

    let pos = tile_idx_to_world(tile_idx, w, mh);
    let color = match ghost_kind {
        GhostKind::Invalid => Color::srgba(0.95, 0.25, 0.2, 0.4),
        GhostKind::Upgrade if ok_afford => Color::srgba(0.2, 0.88, 0.95, 0.45),
        GhostKind::Upgrade => Color::srgba(0.95, 0.35, 0.2, 0.38),
        GhostKind::Place if ok_afford => Color::srgba(0.25, 0.95, 0.35, 0.42),
        GhostKind::Place => Color::srgba(0.95, 0.35, 0.2, 0.38),
    };
    let mat = materials.add(ColorMaterial::from_color(color));

    let wants_range = matches!(
        kind,
        BuildingKind::SamLauncher | BuildingKind::DefensePost
    );
    let range_mesh = if kind == BuildingKind::SamLauncher {
        Some(ms.sam_range.clone())
    } else if kind == BuildingKind::DefensePost {
        Some(ms.defense_range.clone())
    } else {
        None
    };
    let range_color = materials.add(ColorMaterial::from_color(Color::srgba(
        0.9, 0.9, 0.35, 0.22,
    )));

    if let Some(e) = ghost_parts.marker {
        if commands.get_entity(e).is_ok() {
            commands.entity(e).insert(MeshMaterial2d(mat.clone()));
            commands
                .entity(e)
                .insert(Transform::from_xyz(pos.x, pos.y, 275.0));
        } else {
            ghost_parts.marker = None;
        }
    }

    if ghost_parts.marker.is_none() {
        let e = commands
            .spawn((
                StructureGhostRoot,
                InGameEntity,
                Mesh2d(ms.place_ring.clone()),
                MeshMaterial2d(mat.clone()),
                Transform::from_xyz(pos.x, pos.y, 275.0),
                GlobalTransform::default(),
            ))
            .id();
        ghost_parts.marker = Some(e);
    }

    if wants_range {
        if let Some(rm) = range_mesh {
            if let Some(re) = ghost_parts.range {
                if commands.get_entity(re).is_ok() {
                    commands.entity(re).insert(Mesh2d(rm.clone()));
                    commands
                        .entity(re)
                        .insert(Transform::from_xyz(pos.x, pos.y, 274.0));
                    commands
                        .entity(re)
                        .insert(MeshMaterial2d(range_color.clone()));
                } else {
                    ghost_parts.range = None;
                }
            }
            if ghost_parts.range.is_none() {
                let re = commands
                    .spawn((
                        GhostRangeRing,
                        InGameEntity,
                        Mesh2d(rm),
                        MeshMaterial2d(range_color),
                        Transform::from_xyz(pos.x, pos.y, 274.0),
                        GlobalTransform::default(),
                    ))
                    .id();
                ghost_parts.range = Some(re);
            }
        }
    } else if let Some(re) = ghost_parts.range.take() {
        commands.entity(re).despawn();
    }
}

fn system_attach_building_visuals(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    mesh: Option<Res<StructureMarkerMeshes>>,
    settings: Res<ClientSettings>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    q_new: Query<(Entity, &Building), Without<BuildingVisualDone>>,
) {
    let Some(game) = game_opt.as_deref() else {
        return;
    };
    let Some(marker) = mesh.as_ref() else {
        return;
    };
    let mw = game.map.width;
    let mh = game.map.height;
    let pending: Vec<(Entity, Building)> = q_new.iter().map(|(e, b)| (e, *b)).collect();
    for (e, b) in pending {
        let rgb = game
            .player(b.owner_id)
            .map(|p| p.color)
            .unwrap_or([0.7, 0.7, 0.7]);
        let mat = materials.add(ColorMaterial::from_color(Color::srgba(
            rgb[0], rgb[1], rgb[2], 0.88,
        )));
        let pos = tile_idx_to_world(b.tile_idx, mw, mh);
        commands.entity(e).insert((
            BuildingVisualDone,
            BuildingMapVisualRoot,
            Transform::from_xyz(pos.x, pos.y, 268.0),
            GlobalTransform::default(),
            Visibility::default(),
        ));
        let font = settings.font.clone();
        commands.entity(e).with_children(|ch| {
            ch.spawn((
                InGameEntity,
                Mesh2d(marker.place_ring.clone()),
                MeshMaterial2d(mat),
                Transform::default(),
                GlobalTransform::default(),
                Visibility::default(),
            ));
            ch.spawn((
                InGameEntity,
                Sprite {
                    image: asset_server.load(building_kind_icon_path(b.kind)),
                    custom_size: Some(Vec2::splat(TILE_SIZE * 2.4)),
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, 1.0),
                GlobalTransform::default(),
                Visibility::default(),
            ));
            ch.spawn((
                InGameEntity,
                BuildingLabelTarget(e),
                Text2d::new(format!(
                    "{}{}",
                    building_kind_short(b.kind),
                    b.level
                )),
                TextFont {
                    font,
                    font_size: 11.0,
                    weight: FontWeight::SEMIBOLD,
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::new_with_justify(Justify::Center),
                Transform::from_xyz(0.0, TILE_SIZE * 0.42, 2.0),
            ));
        });
    }
}

fn system_sync_building_visual_lod(
    q_camera: Query<&Projection, With<Camera2d>>,
    mut q_visuals: Query<&mut Transform, With<BuildingMapVisualRoot>>,
) {
    let Ok(projection) = q_camera.single() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection else {
        return;
    };

    // OpenFront keeps structure symbols readable by scaling icon stages with zoom.
    // Bevy orthographic scale grows as the camera zooms out, so compensate the
    // world-space visual root only; the deterministic simulation never sees this.
    let visual_scale = (ortho.scale / 1.4).clamp(1.0, 7.0);
    let scale = Vec3::splat(visual_scale);
    for mut transform in &mut q_visuals {
        if transform.scale != scale {
            transform.scale = scale;
        }
    }
}

fn system_sync_building_map_labels(
    game_opt: Option<Res<GameState>>,
    q_b: Query<&Building>,
    mut q_l: Query<(&BuildingLabelTarget, &mut Text2d)>,
) {
    if game_opt.is_none() {
        return;
    }
    for (t, mut text) in q_l.iter_mut() {
        let Ok(b) = q_b.get(t.0) else {
            continue;
        };
        let pending = if b.under_construction { "…" } else { "" };
        let line = format!(
            "{}{}{}",
            building_kind_short(b.kind),
            b.level,
            pending
        );
        if text.0 != line {
            text.0 = line.clone();
        }
    }
}

/// Server stamps player_id → bundles into next Turn → broadcasts → we apply next frame.
/// Key `B` force-boat path (OpenFront `DoBoatAttackEvent`): same resolver as right-click.
#[derive(bevy::ecs::system::SystemParam)]
struct ForceBoatKeyCtx<'w> {
    game_opt: Option<Res<'w, GameState>>,
    my_id: Res<'w, MyPlayerId>,
    attack_ratio: Res<'w, AttackRatio>,
    components: Option<Res<'w, WaterComponents>>,
    path_scratch: ResMut<'w, WaterPathfinderScratch>,
}

fn system_force_boat_key(
    kb: Res<ButtonInput<KeyCode>>,
    hovered: Res<HoveredTile>,
    mut hud_hit: ParamSet<(
        Query<&RelativeCursorPosition, With<BottomHudRoot>>,
        Query<&RelativeCursorPosition, With<ActionDockPointerSink>>,
    )>,
    mut commands: Commands,
    ring_mesh: Res<AttackRingMesh>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    settings: Res<ClientSettings>,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
    local_buf: Option<ResMut<LocalIntentBuffer>>,
    mut ctx: ForceBoatKeyCtx,
) {
    if !kb.just_pressed(KeyCode::KeyB) {
        return;
    }
    let over_hud = hud_hit.p0().iter().any(|rcp| rcp.cursor_over)
        || hud_hit.p1().iter().any(|rcp| rcp.cursor_over);
    if over_hud {
        return;
    }
    let Some(game) = ctx.game_opt.as_deref() else {
        return;
    };
    let Some((x, y)) = hovered.0 else {
        bevy::log::warn!("force boat (B): no tile under cursor");
        return;
    };
    let pid = ctx.my_id.0;
    if pid == 0 {
        return;
    }
    let Some(player) = game.player(pid) else {
        return;
    };
    if !player.alive || player.troops < config::EXPAND_COST {
        return;
    }
    let target_owner = game.map.owner_id(x, y);
    if target_owner == pid {
        return;
    }
    let target_tile = y * game.map.width + x;
    let ratio = ctx.attack_ratio.0.clamp(0.01, 1.0) as f64;
    let send = (player.troops * ratio).floor().max(0.0).min(player.troops);
    if send < config::EXPAND_COST {
        return;
    }
    let Some(wc) = ctx.components.as_deref() else {
        bevy::log::warn!("force boat (B): WaterComponents not loaded");
        return;
    };
    let Some(intent) = try_launch_fleet_intent(
        game,
        wc,
        &mut *ctx.path_scratch,
        pid,
        target_owner,
        target_tile,
        send,
        "KeyB force boat",
    ) else {
        return;
    };
    spawn_attack_command_ring(
        &mut commands,
        &ring_mesh,
        &mut materials,
        &settings,
        player.color,
        x,
        y,
        game.map.height,
        send,
    );
    send_gameplay_intent(intent, pid, local_buf, &mut senders);
}

/// Bundled per-click game-state params for [`player_click`]. Collapsed into a
/// single [`SystemParam`] to stay under Bevy 0.18's 16-arg limit after adding
/// the water-component resource and pathfinder scratch for neutral-land BFS.
#[derive(bevy::ecs::system::SystemParam)]
struct ClickCtx<'w> {
    game_opt: Option<Res<'w, GameState>>,
    my_id: Res<'w, MyPlayerId>,
    attack_ratio: Res<'w, AttackRatio>,
    components: Option<Res<'w, WaterComponents>>,
    path_scratch: ResMut<'w, WaterPathfinderScratch>,
}

fn player_click(
    mut commands: Commands,
    ring_mesh: Res<AttackRingMesh>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mouse:       Res<ButtonInput<MouseButton>>,
    touches:     Res<Touches>,
    q_camera:    Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    drag_state:  Res<CameraDragState>,
    touch_gesture: Res<TouchGestureState>,
    hovered:     Res<HoveredTile>,
    settings:    Res<ClientSettings>,
    mut hud_hit: ParamSet<(
        Query<&RelativeCursorPosition, With<BottomHudRoot>>,
        Query<&RelativeCursorPosition, With<ActionDockPointerSink>>,
    )>,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
    local_buf: Option<ResMut<LocalIntentBuffer>>,
    build_mode: Res<BuildStructureMode>,
    q_building: Query<(Entity, &Building)>,
    mut ctx: ClickCtx,
) {
    // We trigger ONLY on release to support dragging.
    // Additionally, if the user accumulated too much drag, we abort the intent payload.
    let left_mouse_release = mouse.just_released(MouseButton::Left);
    let right_mouse_release = mouse.just_released(MouseButton::Right);
    let touch_release = touches.any_just_released();
    if !left_mouse_release && !right_mouse_release && !touch_release {
        return;
    }
    let wants_force_boat = right_mouse_release && !touch_release;
    if touch_release && touch_gesture.max_touches_in_gesture > 1 {
        return; // Ignore pinch/two-finger releases.
    }
    if drag_state.accumulated_drag > 10.0 { return; } // It was a drag, not a click

    // Clicks/releases over the combat HUD must not issue map attacks (OpenFront: UI eats pointer).
    let over_hud = hud_hit.p0().iter().any(|rcp| rcp.cursor_over)
        || hud_hit.p1().iter().any(|rcp| rcp.cursor_over);
    if over_hud {
        return;
    }

    let Some(game) = ctx.game_opt.as_deref() else { return };
    let target = hovered.0.or_else(|| {
        let viewport = touch_gesture.last_touch_pos?;
        let (camera, cam_t) = q_camera.single().ok()?;
        let world = camera.viewport_to_world_2d(cam_t, viewport).ok()?;
        let gx = (world.x / TILE_SIZE).round() as i32;
        let gy = game.map.height as i32 - 1 - (world.y / TILE_SIZE).round() as i32;
        if game.map.is_valid_coord(gx, gy) {
            Some((gx as u32, gy as u32))
        } else {
            None
        }
    });
    let Some((x, y)) = target else { return };

    let pid = ctx.my_id.0;
    let Some(player) = game.player(pid) else { return };
    if !player.alive {
        return;
    }

    let target_owner = game.map.owner_id(x, y);
    let map_w = game.map.width;
    let target_tile = y * map_w + x;

    if target_owner == pid {
        if wants_force_boat {
            return;
        }
        if let Some(kind) = build_mode.0 {
            let place_release = left_mouse_release || touch_release;
            if place_release {
                let snapshot: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();
                let positions =
                    existing_structure_positions_with_width(snapshot.iter(), map_w, None);
                if let Some(bid) =
                    find_upgrade_target_id(&game.map, pid, kind, target_tile, &snapshot)
                {
                    let cost = structure_build_cost_gold(kind, pid, &snapshot);
                    if player.gold >= cost && cost.is_finite() {
                        send_gameplay_intent(
                            GameplayIntent::UpgradeStructure { building_id: bid },
                            pid,
                            local_buf,
                            &mut senders,
                        );
                    }
                    return;
                }
                if resolve_structure_spawn_tile(&game.map, pid, kind, target_tile, &positions)
                    .is_some()
                {
                    let cost = structure_build_cost_gold(kind, pid, &snapshot);
                    if player.gold >= cost && cost.is_finite() {
                        send_gameplay_intent(
                            GameplayIntent::BuildStructure {
                                kind,
                                target_tile,
                            },
                            pid,
                            local_buf,
                            &mut senders,
                        );
                    }
                    return;
                }
            }
            return;
        }
        return;
    }

    if player.troops < config::EXPAND_COST {
        return;
    }

    // OpenFront `canAttack` / `canBoatAttack` decision tree:
    //   • enemy-owned tile    → ground attack if I share a border with that owner
    //   • neutral-owned tile  → ground attack if BFS through neutral land (≤200) reaches my territory
    //   • otherwise          → boat if I share a water component with a reachable shoreline
    let border_tiles: &[u32] = &player.border_tiles;

    let can_ground_attack = if target_owner != 0 {
        shares_border_with(&game.map, border_tiles, target_owner)
    } else {
        let scratch = &mut *ctx.path_scratch;
        dark_rift_shared::warp_fleet::can_ground_attack_neutral(
            &game.map,
            pid,
            target_tile,
            200,
            &mut scratch.bfs_queue,
            &mut scratch.bfs_visited,
            &mut scratch.bfs_stamp,
        )
    };

    let has_water_access = match ctx.components.as_deref() {
        Some(wc) => !player_water_components(&game.map, wc, pid, border_tiles).is_empty(),
        None => false,
    };

    let ratio = ctx.attack_ratio.0.clamp(0.01, 1.0) as f64;
    let send_floor = (player.troops * ratio).floor();
    let send = send_floor.max(0.0).min(player.troops);
    if send < config::EXPAND_COST {
        return;
    }


    // Decide intent first (OpenFront: LMB canAttack wins; auto-boat only when route resolves).
    // Right-click / force-boat skips ground branch and uses the same resolver as the simulation.
    let intent = if wants_force_boat {
        let Some(wc) = ctx.components.as_deref() else {
            bevy::log::warn!("player_click: right-click boat — WaterComponents not loaded");
            return;
        };
        try_launch_fleet_intent(
            game,
            wc,
            &mut *ctx.path_scratch,
            pid,
            target_owner,
            target_tile,
            send,
            "player_click: right-click boat",
        )
    } else if can_ground_attack {
        Some(GameplayIntent::Attack(AttackIntent {
            target_owner,
            troops: Some(send),
        }))
    } else if has_water_access {
        let Some(wc) = ctx.components.as_deref() else {
            return;
        };
        try_launch_fleet_intent(
            game,
            wc,
            &mut *ctx.path_scratch,
            pid,
            target_owner,
            target_tile,
            send,
            "player_click: auto boat",
        )
    } else {
        return;
    };

    let Some(intent) = intent else {
        return;
    };

    spawn_attack_command_ring(
        &mut commands,
        &ring_mesh,
        &mut materials,
        &settings,
        player.color,
        x,
        y,
        game.map.height,
        send,
    );
    send_gameplay_intent(intent, pid, local_buf, &mut senders);
}

/// Spawn disc + landing ring for new `WarpFleet`s owned by the local player (OpenFront-style feedback).
fn system_spawn_fleet_visuals(
    mut commands: Commands,
    meshes: Option<Res<FleetVisualMeshes>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    my_id: Res<MyPlayerId>,
    game_opt: Option<Res<GameState>>,
    mut q: Query<(Entity, &WarpFleet), (Added<WarpFleet>, Without<FleetClientVisuals>)>,
) {
    let Some(m) = meshes.as_ref() else {
        return;
    };
    let Some(game) = game_opt.as_deref() else {
        return;
    };
    let pid = my_id.0;
    if pid == 0 {
        return;
    }
    let mw = game.map.width;
    let mh = game.map.height;
    for (fleet_e, fleet) in q.iter_mut() {
        if fleet.owner_id != pid {
            continue;
        }
        commands.entity(fleet_e).insert(FleetClientVisuals);
        let boat_mat =
            materials.add(ColorMaterial::from_color(Color::srgba(0.12, 0.82, 1.0, 0.9)));
        let wake_mat =
            materials.add(ColorMaterial::from_color(Color::srgba(0.35, 0.95, 1.0, 0.4)));
        let ring_mat =
            materials.add(ColorMaterial::from_color(Color::srgba(1.0, 0.1, 0.06, 0.42)));
        let p0 = tile_idx_to_world(fleet.current_tile, mw, mh);
        let wake_e = commands
            .spawn((
                FollowFleetWake,
                InGameEntity,
                Mesh2d(m.wake_dot.clone()),
                MeshMaterial2d(wake_mat),
                Transform::from_xyz(p0.x, p0.y, 269.0).with_scale(Vec3::ZERO),
                GlobalTransform::default(),
                Visibility::Hidden,
            ))
            .id();
        commands.spawn((
            FollowFleetBoat {
                fleet: fleet_e,
                wake: wake_e,
                prev_tile: None,
            },
            InGameEntity,
            Mesh2d(m.boat.clone()),
            MeshMaterial2d(boat_mat),
            Transform::from_xyz(p0.x, p0.y, 272.0),
            GlobalTransform::default(),
            Visibility::default(),
        ));
        let pl = tile_idx_to_world(fleet.dst_tile, mw, mh);
        commands.spawn((
            FollowFleetTarget { fleet: fleet_e },
            InGameEntity,
            Mesh2d(m.target_ring.clone()),
            MeshMaterial2d(ring_mat),
            Transform::from_xyz(pl.x, pl.y, 264.0).with_scale(Vec3::splat(1.12)),
            GlobalTransform::default(),
            Visibility::default(),
        ));
    }
}

fn system_sync_fleet_visuals(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    mut boat_q: Query<(Entity, &mut FollowFleetBoat, &mut Transform), (Without<FollowFleetWake>, Without<FollowFleetTarget>)>,
    mut wake_q: Query<(&FollowFleetWake, &mut Transform, &mut Visibility), (Without<FollowFleetBoat>, Without<FollowFleetTarget>)>,
    mut target_q: Query<(Entity, &FollowFleetTarget, &mut Transform), (Without<FollowFleetBoat>, Without<FollowFleetWake>)>,
    q_fleet: Query<&WarpFleet>,
) {
    let Some(game) = game_opt.as_deref() else {
        return;
    };
    let mw = game.map.width;
    let mh = game.map.height;
    for (e, mut fb, mut tf) in boat_q.iter_mut() {
        let Ok(f) = q_fleet.get(fb.fleet) else {
            commands.entity(fb.wake).despawn();
            commands.entity(e).despawn();
            continue;
        };
        let cur = f.current_tile;
        let p = tile_idx_to_world(cur, mw, mh);
        tf.translation = Vec3::new(p.x, p.y, 272.0);
        if let Ok((_, mut wtf, mut vis)) = wake_q.get_mut(fb.wake) {
            if let Some(prev) = fb.prev_tile {
                if prev != cur {
                    let wp = tile_idx_to_world(prev, mw, mh);
                    wtf.translation = Vec3::new(wp.x, wp.y, 269.0);
                    wtf.scale = Vec3::ONE;
                    *vis = Visibility::Visible;
                }
            }
            fb.prev_tile = Some(cur);
        }
    }
    for (e, ft, _tf) in target_q.iter_mut() {
        if q_fleet.get(ft.fleet).is_err() {
            commands.entity(e).despawn();
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


/// Write changed owner IDs into the GPU ShaderStorageBuffer.
fn handle_tile_change(
    game_opt: Option<ResMut<GameState>>,
    shader_map_opt: Option<ResMut<ShaderMapData>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut game) = game_opt else { return };
    if !game.is_changed() { return; }
    let Some(mut shader_map) = shader_map_opt else { return };

    if shader_map.colors_dirty {
        if let Some(colors_img) = images.get_mut(&shader_map.player_colors) {
            sync_player_colors(&game, colors_img);
            shader_map.colors_dirty = false;
        }
    }

    if let Some(board_img) = images.get_mut(&shader_map.board_data) {
        if let Some(ref mut data) = board_img.data {
            for ev in &game.events {
                if let GameEvent::TileCaptured { x, y, new_owner: owner, .. } = ev {
                    // Update single owner ID at (x, y)
                    let linear_index = (*y as usize * game.map.width as usize) + *x as usize;
                    let byte_offset = linear_index * 2;
                    if byte_offset + 1 < data.len() {
                        data[byte_offset..byte_offset+2].copy_from_slice(&owner.to_ne_bytes());
                    }
                }
            }
        }
    }

    game.events.clear();
}



fn update_hud(
    overlay: Res<OverlayDiagnostics>,
    build_mode: Res<BuildStructureMode>,
    highest: Res<HighestReceivedTurn>,
    mut fps_q: Query<&mut Text, (With<FpsText>, Without<GameTimerText>)>,
    mut timer_q: Query<&mut Text, With<GameTimerText>>,
) {
    if let Some(mut fps_text) = fps_q.iter_mut().next() {
        let show_build_line =
            build_mode.0.is_some() && !overlay.build_hint.is_empty();
        let fps_str = if show_build_line {
            format!(
                "{}  |  Ping: {}  |  {} FPS  |  {}",
                overlay.status_line,
                overlay.ping_value,
                overlay.fps_value,
                overlay.build_hint
            )
        } else {
            format!(
                "{}  |  Ping: {}  |  {} FPS",
                overlay.status_line, overlay.ping_value, overlay.fps_value
            )
        };
        if **fps_text != fps_str {
            **fps_text = fps_str;
        }
    }

    if let Some(mut timer_text) = timer_q.iter_mut().next() {
        let seconds = (highest.0.unwrap_or(0) as f64 * 0.040).floor() as u64;
        let mins = seconds / 60;
        let secs = seconds % 60;
        let time_str = format!("{:02}:{:02}", mins, secs);
        if **timer_text != time_str {
            **timer_text = time_str;
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
    settings: Res<ClientSettings>,
    mut q_readout: Query<
        (&mut Text, &mut TextFont, &mut Node),
        (
            With<AttackRatioReadout>,
            Without<BottomHudCap>,
            Without<BottomHudTroops>,
            Without<BottomHudGold>,
            Without<AttackRatioTrack>,
            Without<AttackRatioBarShell>,
            Without<AttackRatioFill>,
        ),
    >,
    mut q_cap: Query<
        &mut Text,
        (
            With<BottomHudCap>,
            Without<AttackRatioReadout>,
            Without<BottomHudTroops>,
            Without<BottomHudGold>,
        ),
    >,
    mut q_gold: Query<
        &mut Text,
        (
            With<BottomHudGold>,
            Without<AttackRatioReadout>,
            Without<BottomHudCap>,
            Without<BottomHudTroops>,
        ),
    >,
    mut q_troops: Query<
        &mut Text,
        (
            With<BottomHudTroops>,
            Without<AttackRatioReadout>,
            Without<BottomHudCap>,
            Without<BottomHudGold>,
        ),
    >,
    mut q_track: Query<
        &mut Node,
        (
            With<AttackRatioTrack>,
            Without<AttackRatioReadout>,
            Without<AttackRatioBarShell>,
            Without<AttackRatioFill>,
        ),
    >,
    mut q_bar_shell: Query<
        &mut Node,
        (
            With<AttackRatioBarShell>,
            Without<AttackRatioReadout>,
            Without<AttackRatioTrack>,
            Without<AttackRatioFill>,
        ),
    >,
    mut q_fill: Query<
        &mut Node,
        (
            With<AttackRatioFill>,
            Without<AttackRatioReadout>,
            Without<AttackRatioTrack>,
            Without<AttackRatioBarShell>,
        ),
    >,
) {
    let Some(game) = game_opt.as_deref() else { return };

    let Ok((mut readout, mut readout_font, mut readout_node)) = q_readout.single_mut() else { return };
    
    if my_id.0 == 0 {
        **readout = String::new();
        if let Ok(mut g) = q_cap.single_mut() { **g = "0".to_string(); }
        if let Ok(mut gl) = q_gold.single_mut() { **gl = "0".to_string(); }
        if let Ok(mut t) = q_troops.single_mut() { **t = "0".to_string(); }
        return;
    }

    let s = clamp_ui_scale(settings.config.ui.combat_hud_scale);
    readout_font.font_size = 18.0 * s;
    readout_node.width = Val::Px(180.0 * s);
    if let Ok(mut n) = q_track.single_mut() {
        n.height = Val::Px(32.0 * s);
    }
    if let Ok(mut n) = q_bar_shell.single_mut() {
        n.height = Val::Px(16.0 * s);
        n.border_radius = BorderRadius::all(Val::Px(8.0 * s));
    }

    let r = attack_ratio.0.clamp(0.01, 1.0);
    if let Ok(mut fill) = q_fill.single_mut() {
        fill.width = Val::Percent(r * 100.0);
    }

    // Economy Sync
    if let Some(player) = game.player(my_id.0) {
        let fmt = |v: f64| -> String {
            if v >= 1_000_000.0 {
                format!("{:.1}M", v / 1_000_000.0)
            } else if v >= 1_000.0 {
                format!("{:.1}K", v / 1_000.0)
            } else {
                format!("{:.0}", v)
            }
        };

        if let Ok(mut g) = q_cap.single_mut() {
            let s = fmt(player.max_troops);
            if **g != s { **g = s; }
        }
        if let Ok(mut gl) = q_gold.single_mut() {
            let s = fmt(player.gold);
            if **gl != s { **gl = s; }
        }
        if let Ok(mut t) = q_troops.single_mut() {
            let s = fmt(player.troops);
            if **t != s { **t = s; }
        }
    }

    let preview_troops = game
        .player(my_id.0)
        .map(|p| (p.troops * r as f64).floor().max(0.0))
        .unwrap_or(0.0);
    let readout_str = format!("{}% ({:.0})", (r * 100.0).round(), preview_troops);
    if **readout != readout_str {
        **readout = readout_str;
    }

}

#[derive(Component)]
pub struct EventFeedLine {
    pub timer: Timer,
}

fn system_update_event_feed(
    mut commands: Commands,
    time: Res<Time>,
    game_opt: Option<Res<GameState>>,
    mut last_event_idx: Local<usize>,
    settings: Res<ClientSettings>,
    q_container: Query<Entity, With<EventFeedContainer>>,
    mut q_lines: Query<(Entity, &mut EventFeedLine, &mut TextColor)>,
) {
    let Some(container_ent) = q_container.iter().next() else { return };

    for (entity, mut line, mut color) in q_lines.iter_mut() {
        line.timer.tick(time.delta());
        let alpha = line.timer.fraction_remaining();
        color.0.set_alpha(alpha);
        if alpha <= 0.0 {
            commands.entity(entity).despawn();
        }
    }

    let Some(game) = game_opt else { return };

    // New events
    let current_len = game.events.len();
    if *last_event_idx < current_len {
        let s = clamp_ui_scale(settings.config.ui.combat_hud_scale);
        
        for i in *last_event_idx..current_len {
            let ev = &game.events[i];
            let msg = match ev {
                GameEvent::PlayerEliminated { player_id } => {
                    let name = game.player(*player_id).map(|p| p.name.as_str()).unwrap_or("Unknown");
                    format!("Eliminated: {}", name)
                }
                GameEvent::StructureSpawned { owner_id, kind, .. } => {
                    let name = game.player(*owner_id).map(|p| p.name.as_str()).unwrap_or("Unknown");
                    format!("Built: {} {}", name, kind.as_str())
                }
                GameEvent::StructureUpgraded { kind, level, .. } => {
                    format!("Upgraded: {} to Lvl {}", kind.as_str(), level)
                }
                GameEvent::TileCaptured { .. } => {
                    // Ignore tile captures, too noisy
                    continue;
                }
                _ => continue,
            };

            commands.entity(container_ent).with_children(|p| {
                p.spawn((
                    EventFeedLine { timer: Timer::from_seconds(5.0, TimerMode::Once) },
                    Text::new(msg),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 14.0 * s,
                        ..default()
                    },
                    TextColor(Color::srgba(0.9, 0.9, 0.9, 1.0)),
                    Node {
                        margin: UiRect::bottom(Val::Px(4.0 * s)),
                        ..default()
                    },
                ));
            });
        }
        *last_event_idx = current_len;
    }
}

fn system_sync_outgoing_attack_rows(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    slot_q: Query<Entity, With<OutgoingRowsContainer>>,
    q_exec: Query<&AttackExecution>,
    q_fleet: Query<&WarpFleet>,
    my_id: Res<MyPlayerId>,
    settings: Res<ClientSettings>,
    ui_shell: Res<UiShellState>,
    mut row_map: ResMut<OutgoingAttackRowMap>,
    mut fleet_row_map: ResMut<OutgoingFleetRowMap>,
    mut q_attack_line: Query<(&mut Text, &mut TextFont), (With<OutgoingAttackLineText>, Without<OutgoingFleetLineText>)>,
    mut q_fleet_line: Query<(&mut Text, &mut TextFont), (With<OutgoingFleetLineText>, Without<OutgoingAttackLineText>)>,
) {
    let Ok(slot) = slot_q.single() else {
        return;
    };

    if my_id.0 == 0 {
        for (_, (row_e, _)) in row_map.0.drain() {
            commands.entity(row_e).despawn();
        }
        for (_, (row_e, _)) in fleet_row_map.0.drain() {
            commands.entity(row_e).despawn();
        }
        return;
    }
    let Some(game) = game_opt else {
        return;
    };

    let s = clamp_ui_scale(settings.config.ui.combat_hud_scale);
    let row_font = 11.5 * s;
    let cancel_icon_font = 14.0 * s;
    let (tw, th) = touch_target_for_breakpoint(ui_shell.breakpoint);
    let cancel_w = tw * s;
    let cancel_h = th * s;
    let row_radius = panel_radius_for_breakpoint(ui_shell.breakpoint) * s;

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
        let who = if exec.target_owner == 0 {
            "Neutral".to_string()
        } else {
            game
                .player(exec.target_owner)
                .map(|p| hud_trunc_name(&p.name, 14))
                .unwrap_or_else(|| format!("{}", exec.target_owner))
        };
        let fmt_num = |v: f64| -> String {
            if v >= 10_000.0 {
                format!("{:.1}k", v / 1000.0)
            } else {
                format!("{:.0}", v)
            }
        };
        
        let line = format!(
            "To: {}\n{} / {}",
            who,
            fmt_num(exec.troops.max(0.0)),
            fmt_num(exec.initial_troops.max(0.0)),
        );

        if let Some((_, text_e)) = row_map.0.get(&exec.id).copied() {
            if let Ok((mut t, mut tf)) = q_attack_line.get_mut(text_e) {
                if **t != line {
                    **t = line;
                }
                tf.font_size = row_font;
            }
        } else {
            let font = settings.font.clone();
            let exec_id = exec.id;
            let row_e = commands
                .spawn((
                    ChildOf(slot),
                    OutgoingAttackRowRoot,
                    Node {
                        width: Val::Px(120.0 * s),
                        height: Val::Px(80.0 * s),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(4.0 * s), Val::Px(6.0 * s)),
                        border: UiRect::all(Val::Px(1.0 * s)),
                        border_radius: BorderRadius::all(Val::Px(row_radius)),
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
                        font_size: row_font,
                        ..default()
                    },
                    TextColor(text_primary()),
                    TextLayout::new_with_justify(Justify::Center),
                    Node {
                        flex_grow: 1.0,
                        align_self: AlignSelf::Center,
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
                        border_radius: BorderRadius::all(Val::Px(8.0 * s)),
                        ..default()
                    },
                    BackgroundColor(accent_danger()),
                    BorderColor::all(accent_danger_border()),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("X"),
                        TextFont {
                            font,
                            font_size: cancel_icon_font,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });

            row_map.0.insert(exec_id, (row_e, text_e));
        }
    }

    // Outgoing boats (`WarpFleet`) — OpenFront `AttacksDisplay` transport rows + recall.
    if !settings.config.ui.combat_hud_fleet_rows {
        for (_, (row_e, _)) in fleet_row_map.0.drain() {
            commands.entity(row_e).despawn();
        }
        return;
    }

    let mut fleets: Vec<&WarpFleet> = q_fleet
        .iter()
        .filter(|f| f.owner_id == my_id.0 && !f.retreating)
        .collect();
    fleets.sort_by_key(|f| f.id);

    let desired_f: HashSet<u64> = fleets.iter().map(|f| f.id).collect();
    let stale_f: Vec<u64> = fleet_row_map
        .0
        .keys()
        .copied()
        .filter(|id| !desired_f.contains(id))
        .collect();
    for id in stale_f {
        if let Some((row_e, _)) = fleet_row_map.0.remove(&id) {
            commands.entity(row_e).despawn();
        }
    }

    for fleet in fleets {
        let who = if fleet.target_owner == 0 {
            "Neutral".to_string()
        } else {
            game
                .player(fleet.target_owner)
                .map(|p| hud_trunc_name(&p.name, 14))
                .unwrap_or_else(|| format!("{}", fleet.target_owner))
        };
        let fmt_num = |v: f64| -> String {
            if v >= 10_000.0 {
                format!("{:.1}k", v / 1000.0)
            } else {
                format!("{:.0}", v)
            }
        };

        let line = format!(
            "⛵ To: {}\n{}",
            who,
            fmt_num(fleet.troops.max(0.0)),
        );

        if let Some((_, text_e)) = fleet_row_map.0.get(&fleet.id).copied() {
            if let Ok((mut t, mut tf)) = q_fleet_line.get_mut(text_e) {
                if **t != line {
                    **t = line;
                }
                tf.font_size = row_font;
            }
        } else {
            let font = settings.font.clone();
            let fleet_id = fleet.id;
            let row_e = commands
                .spawn((
                    ChildOf(slot),
                    OutgoingFleetRowRoot,
                    Node {
                        width: Val::Px(120.0 * s),
                        height: Val::Px(80.0 * s),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(4.0 * s), Val::Px(6.0 * s)),
                        border: UiRect::all(Val::Px(1.0 * s)),
                        border_radius: BorderRadius::all(Val::Px(row_radius)),
                        ..default()
                    },
                    BackgroundColor(panel_bg()),
                    BorderColor::all(panel_border()),
                ))
                .id();

            let text_e = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingFleetLineText,
                    Text::new(line),
                    TextFont {
                        font: font.clone(),
                        font_size: row_font,
                        ..default()
                    },
                    TextColor(text_primary()),
                    TextLayout::new_with_justify(Justify::Center),
                    Node {
                        flex_grow: 1.0,
                        align_self: AlignSelf::Center,
                        ..default()
                    },
                ))
                .id();

            commands
                .spawn((
                    ChildOf(row_e),
                    Button,
                    CancelFleetButton(fleet_id),
                    Node {
                        width: Val::Px(cancel_w),
                        height: Val::Px(cancel_h),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(Val::Px(8.0 * s)),
                        ..default()
                    },
                    BackgroundColor(accent_danger()),
                    BorderColor::all(accent_danger_border()),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("X"),
                        TextFont {
                            font,
                            font_size: cancel_icon_font,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });

            fleet_row_map.0.insert(fleet_id, (row_e, text_e));
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

fn system_handle_recall_fleet_click(
    interaction_q: Query<
        (&Interaction, &CancelFleetButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
) {
    for (interaction, btn) in &interaction_q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let fleet_id = btn.0;
        for mut sender in senders.iter_mut() {
            let _ = sender.send::<ReliableChannel>(ClientGameplayMessage {
                intent: GameplayIntent::RecallFleet { fleet_id },
            });
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────



#[derive(Resource)]
pub struct ShaderMapData {
    pub board_data: Handle<Image>,
    pub player_colors: Handle<Image>,
    pub colors_dirty: bool,
}

fn sync_player_colors(game: &GameState, image: &mut Image) {
    if let Some(ref mut data) = image.data {
        for p in &game.players {
            let idx = (p.id & 0x0FFF) as usize;
            let byte_offset = idx * 4;
            if byte_offset + 3 < data.len() {
                let rgba = bevy::color::Color::srgb(p.color[0], p.color[1], p.color[2]).to_srgba();
                data[byte_offset] = (rgba.red * 255.0) as u8;
                data[byte_offset+1] = (rgba.green * 255.0) as u8;
                data[byte_offset+2] = (rgba.blue * 255.0) as u8;
                data[byte_offset+3] = (rgba.alpha * 255.0) as u8;
            }
        }
    }
}


fn spawn_shader_map(
    commands: &mut Commands,
    game: &GameState,
    images: &mut Assets<Image>,
    materials: &mut Assets<border_material::BorderMaterial>,
    meshes: &mut Assets<Mesh>,
    in_game: bool,
) -> ShaderMapData {
    let width = game.map.width;
    let height = game.map.height;

    // 1. Board Data (Owner IDs): R16Uint
    let mut board_data = vec![0u8; (width * height * 2) as usize];
    for y in 0..height {
        for x in 0..width {
            let owner = game.map.owner_id(x, y) as u16;
            let idx = ((y * width) + x) as usize * 2;
            board_data[idx..idx+2].copy_from_slice(&owner.to_ne_bytes());
        }
    }
    let mut board_img = Image::new(
        bevy::render::render_resource::Extent3d { width, height, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        board_data,
        bevy::render::render_resource::TextureFormat::R16Uint,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    board_img.sampler = bevy::image::ImageSampler::nearest();
    let board_handle = images.add(board_img);

    // 2. Player Colors: Rgba8Unorm
    let colors_data = vec![0u8; 4096 * 4];
    let mut colors_img = Image::new(
        bevy::render::render_resource::Extent3d { width: 4096, height: 1, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        colors_data,
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    colors_img.sampler = bevy::image::ImageSampler::nearest();
    sync_player_colors(game, &mut colors_img);
    let colors_handle = images.add(colors_img);

    // 3. Map Terrain: R16Uint
    let mut terrain_data = vec![0u8; (width * height * 2) as usize];
    for y in 0..height {
        for x in 0..width {
            let tile_idx = (y * width) + x;
            let raw_byte = game.map.terrain[tile_idx as usize].as_byte();
            let tile_u16 = raw_byte as u16;
            let byte_idx = tile_idx as usize * 2;
            terrain_data[byte_idx..byte_idx+2].copy_from_slice(&tile_u16.to_ne_bytes());
        }
    }
    let mut terrain_img = Image::new(
        bevy::render::render_resource::Extent3d { width, height, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        terrain_data,
        bevy::render::render_resource::TextureFormat::R16Uint,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    terrain_img.sampler = bevy::image::ImageSampler::nearest();
    let terrain_handle = images.add(terrain_img);

    // 4. Material
    let material = border_material::BorderMaterial {
        board_data: board_handle.clone(),
        uniforms: border_material::BorderUniforms {
            border_color: LinearRgba::new(0.6, 0.6, 0.6, 1.0),
            border_thickness: Vec4::new(1.0, 0.0, 0.0, 0.0),
            texture_size: Vec4::new(width as f32, height as f32, 0.0, 0.0),
            time: Vec4::new(0.0, 0.0, 0.0, 0.0),
            enable_water_animation: UVec4::new(1, 0, 0, 0),
            enable_players: UVec4::new(1, 0, 0, 0),
        },
        player_colors: colors_handle.clone(),
        map_terrain: terrain_handle,
    };
    let material_handle = materials.add(material);

    let screen_w = width as f32 * TILE_SIZE;
    let screen_h = height as f32 * TILE_SIZE;
    let mesh = meshes.add(Rectangle::new(screen_w, screen_h));

    // Center the map at (screen_w / 2, screen_h / 2) because coordinates are from (0,0) bottom-left.
    let offset_x = screen_w / 2.0 - TILE_SIZE / 2.0;
    let offset_y = screen_h / 2.0 - TILE_SIZE / 2.0;

    let mut ecmd = commands.spawn((
        Mesh2d(mesh.into()),
        bevy::sprite_render::MeshMaterial2d(material_handle),
        Transform::from_xyz(offset_x, offset_y, 0.0),
    ));
    if in_game {
        ecmd.insert(InGameEntity);
    }

    ShaderMapData {
        board_data: board_handle,
        player_colors: colors_handle,
        colors_dirty: false,
    }
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
                TextColor(Color::BLACK),
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
        *text_color = TextColor(Color::BLACK);
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
    mut outgoing_fleet_map: ResMut<OutgoingFleetRowMap>,
    mut fresh_tiles: ResMut<FreshlyPaintedTiles>,
    mut build_mode: ResMut<BuildStructureMode>,
    mut ghost_parts: ResMut<StructureGhostParts>,
    mut build_hud: ResMut<BuildableHudState>,
    q_ingame: Query<Entity, With<InGameEntity>>,
    q_execs: Query<Entity, With<AttackExecution>>,
    q_buildings: Query<Entity, With<Building>>,
) {
    ready.0 = false;
    queue.0.clear();
    pending.0 = 0;
    highest.0 = None;
    outgoing_map.0.clear();
    outgoing_fleet_map.0.clear();
    fresh_tiles.tiles.clear();
    build_mode.0 = None;
    *ghost_parts = StructureGhostParts::default();
    *build_hud = BuildableHudState::default();

    for e in q_ingame.iter() {
        commands.entity(e).despawn();
    }
    for e in q_execs.iter() {
        commands.entity(e).despawn();
    }
    for e in q_buildings.iter() {
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
    let am_i_alive = game.player(my_id.0).map(|p| p.alive).unwrap_or(false);

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
    let border_tiles = player.border_coords(game.map.width);
    if border_tiles.is_empty() { return; }

    let mut has_neutral = false;
    let mut player_targets = Vec::new();

    for &(bx, by) in &border_tiles {
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



fn update_water_animation_time(
    time: Res<Time>,
    mut materials: ResMut<Assets<border_material::BorderMaterial>>,
) {
    for (_, material) in materials.iter_mut() {
        material.uniforms.time.x = time.elapsed_secs();
    }
}

/// Global scroll logic ported from Adventures-Bevy.
/// Scrolls the scrollable ancestor of whatever the cursor is over.
fn system_update_scroll_position(
    mut mouse_wheel_reader: MessageReader<bevy::input::mouse::MouseWheel>,
    hover_map: Res<bevy::picking::hover::HoverMap>,
    parent_query: Query<&ChildOf>,
    mut scrolled: Query<(&mut bevy::ui::ScrollPosition, &bevy::ui::ComputedNode)>,
    keyboard: Res<ButtonInput<KeyCode>>,
) {
    for event in mouse_wheel_reader.read() {
        let (mut dx, mut dy) = match event.unit {
            bevy::input::mouse::MouseScrollUnit::Line  => (event.x * 45., event.y * 45.),
            bevy::input::mouse::MouseScrollUnit::Pixel => (event.x * 2., event.y * 2.),
        };
        if keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            std::mem::swap(&mut dx, &mut dy);
        }

        let mut scroll_targets = std::collections::HashSet::<Entity>::new();
        use bevy::ecs::relationship::Relationship;
        for pointer_map in hover_map.values() {
            for entity in pointer_map.keys().copied() {
                let mut current = entity;
                loop {
                    if scrolled.get(current).is_ok() {
                        scroll_targets.insert(current);
                        break;
                    }
                    let Some(parent) = parent_query.get(current).ok().map(|c| c.get()) else {
                        break;
                    };
                    current = parent;
                }
            }
        }

        for entity in scroll_targets {
            if let Ok((mut pos, node)) = scrolled.get_mut(entity) {
                let range = (node.content_size() - node.size()).max(Vec2::ZERO)
                    * node.inverse_scale_factor();
                pos.x = (pos.x - dx).clamp(0., range.x);
                pos.y = (pos.y - dy).clamp(0., range.y);
            }
        }
    }
}
