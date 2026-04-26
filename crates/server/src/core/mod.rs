use bevy::prelude::*;
use std::collections::HashMap;
use dark_rift_shared::protocol::*;

/// Turn interval: how often the server bundles + broadcasts a turn.
/// Mirrors OpenFront's config.turnIntervalMs() (100ms = 10 turns/sec).
pub const TURN_INTERVAL_MS: f32 = 10.0;

/// Auto-start timer when lobby reaches top of queue 
pub const LOBBY_COUNTDOWN_SECS: f32 = 15.0;

// ─── Componentes del Lobby (Phase 1) ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyPhase {
    Waiting,
    CountingDown,
    Active,
    Finished,
}

#[derive(Component)]
pub struct Lobby {
    pub id: u64,
}

#[derive(Component, PartialEq)]
pub struct StatePhase(pub LobbyPhase);

#[derive(Component)]
pub struct MatchMetadata {
    pub seed: u64,
    pub config: dark_rift_shared::game_config::GameConfig,
    pub map_cache: Option<dark_rift_shared::map::GameMap>,
    pub manifest_cache: Option<dark_rift_shared::map_loader::MapManifest>,
}

#[derive(Component)]
pub struct CountdownTimer(pub Timer);

#[derive(Component)]
pub struct ActiveEmptyTimer(pub Timer);

#[derive(Component, Default)]
pub struct MatchRoster(pub Vec<PlayerInfo>);

#[derive(Component, Default)]
pub struct MatchTurnLog {
    pub turns: Vec<Turn>,
    pub current: u64,
}

#[derive(Component, Default)]
pub struct MatchIntentBuffer(pub Vec<StampedIntent>);

#[derive(Component)]
pub struct MatchTimer(pub Timer);

#[derive(Component)]
pub struct HasSentStart(pub bool);

// ─── Resources ──────────────────────────────────────────────────────────────

/// Maps Bevy Entity (Client) -> (MatchEntity, Option<PlayerId>)
#[derive(Resource, Default)]
pub struct ClientRegistry {
    pub clients: HashMap<Entity, (Entity, Option<u16>)>,
}

#[derive(Resource, Default)]
pub struct MasterLobbyIdGen(pub u64);
