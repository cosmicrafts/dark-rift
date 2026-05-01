use std::time::Duration;
use crate::ClientSettings;
use bevy::prelude::*;
use std::collections::HashMap;
use instant::Instant;
use dark_rift_shared::protocol::*;
use dark_rift_shared::game::BuildingKind;
use dark_rift_shared::protocol::StampedIntent;

#[derive(Resource, Default)] pub struct GameReady(pub bool);
#[derive(Resource, Default)] pub struct MyPlayerId(pub u16);
#[derive(Resource)] pub struct PendingServerStart(pub ServerStartMessage);
#[derive(Resource)] pub struct LoadingMapTimeout(pub Timer);
#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)] pub enum AppState { #[default] LobbyBrowser, LoadingMap, InGame }
#[derive(Component)] pub struct InGameEntity;
#[derive(Resource, Default)] pub struct PendingTicks(pub u32);
#[derive(Resource, Default)] pub struct TurnQueue(pub std::collections::VecDeque<Turn>);
#[derive(Component)] pub struct ActionDockPointerSink;
#[derive(Resource)] pub struct OverlayDiagnostics { pub status_line: String, pub fps_value: String, pub ping_value: String, pub build_hint: String }
impl Default for OverlayDiagnostics { fn default() -> Self { Self { status_line: "Connecting to server...".to_string(), fps_value: "…".to_string(), ping_value: "…".to_string(), build_hint: String::new() } } }
#[derive(Default)] pub struct PerfOverlayUiState { pub last_fps_refresh: Option<Instant>, pub last_net_refresh: Option<Instant> }
#[derive(Resource, Default)] pub struct NetTiming { pub connect_started: Option<Instant>, pub handshake_ms: Option<f32>, pub last_turn_at: Option<Instant>, pub turn_spacing_ema_ms: f32 }
#[derive(Resource)] pub struct StructureMarkerMeshes { pub place_ring: Handle<Mesh>, pub sam_range: Handle<Mesh>, pub defense_range: Handle<Mesh> }
#[derive(Resource, Default)] pub struct StructureGhostParts { pub marker: Option<Entity>, pub range: Option<Entity> }
#[derive(Component)] pub struct BuildingVisual(pub u64);
#[derive(Component)] pub struct AttackVisual(pub u64);
#[derive(Component)] pub struct FleetVisual(pub u64);
#[derive(Component)] pub struct BuildingVisualDone;
#[derive(Component)] pub struct BuildingMapVisualRoot;
#[derive(Component)] pub struct StructureGhostRoot;
#[derive(Resource, Default)] pub struct BuildStructureMode(pub Option<BuildingKind>);
#[derive(Resource, Default)] pub struct FreshlyPaintedTiles { pub tiles: HashMap<(u32, u32), f32> }
#[derive(Resource)] pub struct MockSinglePlayerStart(pub ServerStartMessage);
#[derive(Resource, Default)] pub struct SinglePlayerState { pub timer: Timer, pub current_turn: u64 }
#[derive(Resource, Default)] pub struct LocalIntentBuffer(pub Vec<StampedIntent>);
#[derive(Resource, Default)] pub struct HighestReceivedTurn(pub Option<u64>);
#[derive(bevy::ecs::schedule::ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)] pub struct SimulationSchedule;
#[derive(bevy::ecs::system::SystemParam)] pub struct StartGameParams<'w> { pub ready: ResMut<'w, GameReady>, pub my_id: ResMut<'w, MyPlayerId>, pub settings: Res<'w, ClientSettings>, pub queue: ResMut<'w, TurnQueue>, pub pending: ResMut<'w, PendingTicks>, pub highest: ResMut<'w, HighestReceivedTurn>, pub overlay: Option<ResMut<'w, OverlayDiagnostics>>, pub shell: Option<Res<'w, crate::ui::shell::HudShellRegions>>, pub asset_server: Res<'w, AssetServer>, pub meshes: Option<ResMut<'w, Assets<Mesh>>>, pub materials: Option<ResMut<'w, Assets<crate::rendering::BorderMaterial>>>, pub images: Option<ResMut<'w, Assets<Image>>>, pub next_state: ResMut<'w, NextState<AppState>> }
#[derive(Component)] pub struct GameOverPanel;
#[derive(Component)] pub struct GoBackButton;
#[derive(Resource)] pub struct ShaderMapData { pub board_data: Handle<Image>, pub player_colors: Handle<Image>, pub colors_dirty: bool }
#[derive(Component)] pub struct BuildingLabelTarget(pub Entity);
#[derive(Component)] pub struct EventFeedLine { pub timer: Timer }
#[inline] pub fn tile_idx_to_world(idx: u32, map_width: u32, map_height: u32) -> Vec2 { let x = (idx % map_width) as f32; let y = (idx / map_width) as f32; Vec2::new((x + 0.5) * crate::TILE_SIZE, (map_height as f32 - y - 0.5) * crate::TILE_SIZE) }
#[inline] pub fn clamp_ui_scale(s: f32) -> f32 { s.clamp(0.25, 8.0) }

pub use bevy::ui::RelativeCursorPosition;

#[derive(Component)]
pub struct GhostRangeRing;


#[derive(Component)]
pub struct FpsText;

#[derive(Component)]
pub struct GameTimerText;

pub const PERF_OVERLAY_FPS_INTERVAL: Duration = Duration::from_millis(500);
pub const PERF_OVERLAY_NET_INTERVAL: Duration = Duration::from_millis(1000);
