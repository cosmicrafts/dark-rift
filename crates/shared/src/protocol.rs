//! Lightyear networking protocol — Turn-Relay model (mirrors OpenFront architecture).
//!
//! Server flow:  Client sends intent → Server stamps clientID → bundles into Turn → broadcast
//!               → Each client simulates the Turn locally using shared ECS systems.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

// ─── Intents (Client → Server) ─────────────────────────────────────────────

/// A player wants to attack/expand toward a tile owner.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AttackIntent {
    pub target_owner: u16,
    pub troops: Option<f64>,
}

/// Attack or cancel — both are stamped and replayed in `Turn` for determinism.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum GameplayIntent {
    Attack(AttackIntent),
    CancelAttack { attack_id: u64 },
}

/// Stamped intent bundled into a turn (attack or cancel).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StampedIntent {
    pub player_id: u16,
    pub intent: GameplayIntent,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Turn {
    pub turn_number: u64,
    pub intents: Vec<StampedIntent>,
}

// ─── Client Messages (Client → Server) ─────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ClientJoinMessage {
    pub name: String,
    pub is_observer: bool,
    pub target_lobby_id: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LobbyInfo {
    pub id: u64,
    pub num_players: u8,
    pub max_players: u8,
    pub is_counting_down: bool,
    pub timer_secs: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerLobbiesBroadcastMessage {
    pub lobbies: Vec<LobbyInfo>,
}

/// Single FIFO gameplay channel: attacks and cancels share ordering.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ClientGameplayMessage {
    pub intent: GameplayIntent,
}

// ─── Server Messages (Server → Client) ─────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerStartMessage {
    pub my_player_id: Option<u16>,
    pub seed: u64,
    pub players: Vec<PlayerInfo>,
    pub missed_turns: Vec<Turn>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, bevy::prelude::Resource)]
pub struct TurnLog {
    pub turns: Vec<Turn>,
    pub current: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ReplayFile {
    pub seed: u64,
    pub players: Vec<PlayerInfo>,
    pub turns: Vec<Turn>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlayerInfo {
    pub id: u16,
    pub name: String,
    pub color: [f32; 3],
    pub spawn_x: u32,
    pub spawn_y: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerTurnMessage {
    pub turn: Turn,
}

pub struct ReliableChannel;

pub struct SharedProtocolPlugin;

impl Plugin for SharedProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.add_channel::<ReliableChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        app.register_message::<ClientJoinMessage>()
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<ClientGameplayMessage>()
            .add_direction(NetworkDirection::ClientToServer);

        app.register_message::<ServerStartMessage>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<ServerTurnMessage>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<ServerLobbiesBroadcastMessage>()
            .add_direction(NetworkDirection::ServerToClient);
    }
}
