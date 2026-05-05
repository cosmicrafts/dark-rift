//! Dark Rift Client — Networked (Lightyear WebSocket + Netcode)

use bevy::prelude::*;
use bevy::ecs::schedule::common_conditions::resource_changed;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::log::{Level, LogPlugin, DEFAULT_FILTER};
use lightyear::prelude::client::*;
use lightyear::netcode::Key;
use std::net::SocketAddr;
use std::time::Duration;
use clap::Parser;

use dark_rift_shared::config;
use dark_rift_shared::building::*;
use dark_rift_shared::game::GameEvent;

use dark_rift_shared::pathfinding::WaterPathfinderScratch;
use dark_rift_shared::protocol::*;
use dark_rift_shared::warp_fleet::*;
use dark_rift_shared::water_components::WaterComponents;

pub mod map_fetch;
pub mod settings;

pub use map_fetch::{MapCache, MapFetchChannel};
pub use settings::*;

pub mod core;
pub mod gameplay;
pub mod network;
pub mod rendering;
pub mod ui;

pub use crate::core::*;
pub use gameplay::*;
pub use network::*;
pub use rendering::*;
pub use ui::*;

/// World units per map tile edge. Kept at 12 for current HUD/shader map layout (older snapshots used 8).
pub const TILE_SIZE: f32 = 12.0;

pub const PERF_OVERLAY_FPS_INTERVAL: Duration = Duration::from_millis(250);
/// Cap simulation turns processed per frame while draining `PendingTicks` (large values help WASM after tab backgrounding).
pub const CLIENT_CATCHUP_MAX_TICKS_PER_FRAME: usize = 50;
/// Keyboard T/Y adjustment step for attack fraction slider (`AttackRatio`).
pub const ATTACK_RATIO_KEYSTEP: f32 = 0.25;

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
        debug!("Headless bot profile='{}' interval_ticks={}", profile_path, profile.attack_interval_ticks);

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
            .init_resource::<CombatDockUiScale>()
            .init_resource::<SimPerfDiagnostics>();

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
                    prevent_default_event_handling: false,
                    ..default()
                }),
                ..default()
            })
            // WARN base + `DEFAULT_FILTER` targets (wgpu, etc.); `RUST_LOG` overrides when set.
            .set(LogPlugin {
                level: Level::WARN,
                filter: DEFAULT_FILTER.to_string(),
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
        .init_resource::<DesktopCameraPanSmooth>()
        .init_resource::<FreshlyPaintedTiles>()
            .init_resource::<AttackRatio>()
            .init_resource::<OutgoingAttackRowMap>()
            .init_resource::<OutgoingFleetRowMap>()
            .init_resource::<NetTiming>()
            .init_resource::<OverlayDiagnostics>()
            .init_resource::<SimPerfDiagnostics>()
        .init_resource::<UiShellState>()
        .init_resource::<CombatDockUiScale>()
        .init_resource::<WaterPathfinderScratch>()
        .init_resource::<WaterComponents>()
        .init_resource::<BuildStructureMode>()
        .init_resource::<StructureGhostParts>()
        .init_resource::<BuildableHudState>()
        .init_resource::<BuildableHudDirty>();

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
            app.add_plugins(bevy::sprite_render::Material2dPlugin::<rendering::BorderMaterial>::default());
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
                )
                    .chain()
                    .run_if(|cli: Res<Cli>| !cli.bot),
            )
            .add_systems(
                OnEnter(AppState::LobbyBrowser),
                (
                    setup_lobby_browser,
                    lobby_browser::system_apply_lobby_layout_responsive,
                    lobby_browser::system_apply_lobby_controls_responsive,
                )
                    .chain()
                    .run_if(|cli: Res<Cli>| !cli.bot),
            )
            .add_systems(OnExit(AppState::LobbyBrowser), system_cleanup_lobby_browser.run_if(|cli: Res<Cli>| !cli.bot))
            .add_systems(OnEnter(AppState::InGame), leaderboard::setup_leaderboard.run_if(|cli: Res<Cli>| !cli.bot))
            .add_systems(
                OnExit(AppState::InGame),
                (
                    system_clear_sim_perf_line,
                    system_cleanup_ingame_entities,
                )
                    .chain()
                    .run_if(|cli: Res<Cli>| !cli.bot),
            )
            .add_systems(Update, (
                system_handle_join_clicks,
                system_handle_cancel_join,
                lobby_browser::system_handle_coming_soon_click,
                lobby_browser::system_clear_lobby_nickname_focus,
                lobby_browser::system_handle_single_player_click,
                lobby_browser::system_nickname_field_focus_from_click,
                lobby_browser::system_nickname_keyboard_when_focused,
                lobby_browser::system_nickname_cursor_blink,
                lobby_browser::system_nickname_autosave_tick,
                system_update_lobby_browser,
            ).run_if(|state: Res<State<AppState>>, cli: Res<Cli>| *state.get() == AppState::LobbyBrowser && !cli.bot))
            .add_systems(
                Update,
                system_lobby_main_menu_anim
                    .run_if(|state: Res<State<AppState>>, cli: Res<Cli>| {
                        *state.get() == AppState::LobbyBrowser && !cli.bot
                    })
                    .run_if(|anim: Option<Res<LobbyMainMenuAnim>>| {
                        if let Some(a) = anim {
                            !a.is_idle() || a.play_main_menu_intro_after_cancel
                        } else {
                            false
                        }
                    }),
            )
            .add_systems(
                Update,
                (
                    lobby_browser::system_apply_lobby_layout_responsive,
                    lobby_browser::system_apply_lobby_controls_responsive,
                )
                    .run_if(|state: Res<State<AppState>>, cli: Res<Cli>| {
                        *state.get() == AppState::LobbyBrowser && !cli.bot
                    })
                    .run_if(
                        resource_changed::<UiShellState>
                            .or(resource_changed::<ClientSettings>),
                    )
                    .before(system_lobby_main_menu_anim),
            )
            .add_systems(Update, (
                handle_server_start,
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
        app.add_systems(Update, (
            system_update_ui_shell_state,
            system_apply_ui_shell_layout
                .run_if(resource_changed::<UiShellState>.or(resource_changed::<CombatDockUiScale>)),
            system_update_perf_overlay,
        ).run_if(|cli: Res<Cli>| !cli.bot));

        app.add_systems(Update, (
            hover_system,
            (system_refresh_buildable_hud, system_apply_buildable_hud).chain(),
            system_buildpill_interactions,
            system_build_structure_hotkeys,
            system_sync_structure_ghost.run_if(bevy::time::common_conditions::on_timer(std::time::Duration::from_millis(33))),
            system_attack_ratio_keyboard,
            system_attack_ratio_slider_drag,
            (player_click, system_animate_attack_command_rings).chain(),
            system_force_boat_key,
            system_exit_button,
        ).run_if(in_state(AppState::InGame)));

        app.add_systems(Update, (
            system_home_button,
            update_hud
                .run_if(
                    resource_changed::<OverlayDiagnostics>
                        .or(resource_changed::<HighestReceivedTurn>)
                        .or(resource_changed::<BuildStructureMode>),
                ),
            system_apply_responsive_hud_layout
                .run_if(resource_changed::<UiShellState>.or(resource_changed::<CombatDockUiScale>).or(resource_changed::<ClientSettings>)),
            system_apply_responsive_hud_icons_and_hotkeys
                .run_if(resource_changed::<UiShellState>.or(resource_changed::<CombatDockUiScale>).or(resource_changed::<ClientSettings>)),
            system_apply_action_dock_layout
                .run_if(resource_changed::<UiShellState>.or(resource_changed::<CombatDockUiScale>).or(resource_changed::<ClientSettings>)),
            system_sync_bottom_combat_hud,
            system_sync_bottom_troops_bar,
            system_settings_toggle_devstats,
            system_update_event_feed,
            system_update_scroll_position,
            (system_sync_outgoing_attack_rows, system_animate_outgoing_rows).chain(),
            system_handle_cancel_click,
            system_handle_recall_fleet_click,
        ).run_if(in_state(AppState::InGame)));

        app.add_systems(Update,
            (
                leaderboard::system_update_leaderboard.run_if(bevy::time::common_conditions::on_timer(std::time::Duration::from_secs(1))),
                leaderboard::system_handle_leaderboard_toggle,
            ).run_if(in_state(AppState::InGame))
        );

        // ── GPU shader map rendering ──
        // handle_tile_change now writes owner IDs into the ShaderStorageBuffer (2 bytes/tile),
        // eliminating the old chunk texture uploads entirely.
        app.add_systems(Update, system_sim_perf_reset.run_if(in_state(AppState::InGame)));
        app.add_systems(Update, (
            handle_tile_change
                .after(system_sim_perf_reset)
                .after(system_update_event_feed),
            update_water_animation_time,
        ).run_if(in_state(AppState::InGame)));

        app.add_systems(
            Update,
            (
                system_sync_fleet_visuals,
                system_sync_building_visuals,
                system_sync_building_visual_lod,
                system_sync_building_map_labels,
            )
                .run_if(in_state(AppState::InGame))
                .run_if(bevy::time::common_conditions::on_timer(std::time::Duration::from_millis(33))),
        );
        
        app.add_systems(Update, (
            camera_movement_and_zoom,
            system_sync_player_labels
                .run_if(|settings: Res<ClientSettings>| settings.config.visuals.enable_floating_labels)
                .run_if(bevy::time::common_conditions::on_timer(std::time::Duration::from_millis(33))),
        ).run_if(in_state(AppState::InGame)));

        app.add_systems(Update, clear_inputs_on_focus_loss);
    }

    let mut sim_schedule = Schedule::new(SimulationSchedule);
    sim_schedule.add_systems((
        pop_turn,
        system_headless_bot_engine.run_if(|cli: Res<Cli>| cli.bot),
    ).chain());
    app.add_schedule(sim_schedule);

    if !cli.bot {
        app.add_systems(Update, (
            catchup_fast_forward,
            system_sync_sim_perf_overlay.after(catchup_fast_forward),
        ).run_if(in_state(AppState::InGame)));
    } else {
        app.add_systems(Update, catchup_fast_forward);
    }

    app.run();
}