//! Lightyear networking protocol definitions.
//! Contains the messages, channels, and plugin registration for shared protocol.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::game::{GameEvent, GameState};

// ─── Messages ───────────────────────────────────────────────────────────────

/// Sent by Client: Request to join the game with a chosen name.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlayerJoinRequest {
    pub name: String,
}

/// Sent by Client: Attempt to expand or attack a tile.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlayerActionMessage {
    pub target_owner: Option<u16>, // None = TerraNullius (neutral), Some(id) = enemy player
    pub troops: Option<f64>,       // None = use default logic
}

/// Sent by Server: The full authoritative state of the game (map, players).
/// For MVP, we broadcast the entire GameState because the map is small.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SyncMapState {
    pub state: Option<GameState>,
}

/// Sent by Server: Important events that happened (e.g. system captured).
/// Sent by Server: Important events that happened (e.g. system captured).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GameEventBroadcast {
    pub event: GameEvent,
}

// ─── Channels ───────────────────────────────────────────────────────────────

pub struct ReliableChannel;

// ─── Protocol Plugin ────────────────────────────────────────────────────────

pub struct SharedProtocolPlugin;

impl Plugin for SharedProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Register channels
        app.add_channel::<ReliableChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        });
        
        // Register messages that are sent over the network
        app.register_message::<PlayerJoinRequest>();
        app.register_message::<PlayerActionMessage>();
        app.register_message::<SyncMapState>();
        app.register_message::<GameEventBroadcast>();
    }
}


