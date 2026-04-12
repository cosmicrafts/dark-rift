//! Lightyear networking protocol — Turn-Relay model (mirrors OpenFront architecture).
//!
//! Server flow:  Client sends intent → Server stamps clientID → Server bundles intents
//!               into Turn at fixed interval → Server broadcasts Turn to all clients
//!               → Each client simulates the Turn locally using shared ECS systems.
//!
//! The server NEVER runs game simulation. It is a pure relay.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

// ─── Intents (Client → Server) ─────────────────────────────────────────────

/// A player wants to attack/expand toward a tile owner.
/// Mirrors OpenFront's AttackIntent { type: "attack", targetID, troops }.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AttackIntent {
    pub target_owner: u16, // 0 = TerraNullius/neutral
    pub troops: Option<f64>, // None = commit all troops
}

/// An intent stamped with the server-assigned player ID.
/// Server attaches player_id before bundling into a Turn.
/// Mirrors OpenFront's StampedIntent = Intent & { clientID }.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StampedIntent {
    pub player_id: u16,
    pub intent: AttackIntent,
}

/// A bundle of intents for a single game tick.
/// Mirrors OpenFront's Turn = { turnNumber, intents: StampedIntent[] }.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Turn {
    pub turn_number: u64,
    pub intents: Vec<StampedIntent>,
}

// ─── Client Messages (Client → Server) ─────────────────────────────────────

/// Client announces themselves when they connect.
/// Mirrors OpenFront's ClientJoinMessage.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ClientJoinMessage {
    pub name: String,
}

/// Client sends a gameplay intent.
/// Mirrors OpenFront's ClientIntentMessage { type: "intent", intent }.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ClientIntentMessage {
    pub intent: AttackIntent,
}

// ─── Server Messages (Server → Client) ─────────────────────────────────────

/// Server sends this when the game starts.
/// Contains the RNG seed, player roster, and any turns a late-joiner missed.
/// Mirrors OpenFront's ServerStartGameMessage { gameStartInfo, turns, myClientID }.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerStartMessage {
    pub my_player_id: u16,
    pub seed: u64,
    pub players: Vec<PlayerInfo>,
    pub missed_turns: Vec<Turn>, // For late-joining clients
}

/// Player descriptor embedded in ServerStartMessage.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlayerInfo {
    pub id: u16,
    pub name: String,
    pub color: [f32; 3],
    pub spawn_x: u32,
    pub spawn_y: u32,
}

/// Server broadcasts this every turn (at fixed TURN_INTERVAL_MS).
/// Mirrors OpenFront's ServerTurnMessage { type: "turn", turn }.
/// Each client receives this and runs the intents through local ECS systems.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerTurnMessage {
    pub turn: Turn,
}

// ─── Channels ───────────────────────────────────────────────────────────────

/// Ordered reliable channel — used for all messages.
pub struct ReliableChannel;

// ─── Protocol Plugin ────────────────────────────────────────────────────────

pub struct SharedProtocolPlugin;

impl Plugin for SharedProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Register channels
        // add_direction(Bidirectional) registers observers that call
        //   transport.add_sender_from_registry / add_receiver_from_registry
        // on every ClientOf entity that gets a Transport component.
        // WITHOUT this, Transport.senders is empty → ChannelNotFound on every send.
        app.add_channel::<ReliableChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        }).add_direction(NetworkDirection::Bidirectional);

        // Client → Server
        // add_direction(ClientToServer) calls register_required_components:
        //   Client   → MessageSender<M>    (client can send it)
        //   ClientOf → MessageReceiver<M>  (server entity that wraps the connection receives it)
        app.register_message::<ClientJoinMessage>()
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<ClientIntentMessage>()
            .add_direction(NetworkDirection::ClientToServer);

        // Server → Client
        // add_direction(ServerToClient) calls register_required_components:
        //   ClientOf → MessageSender<M>    (server entity that wraps the connection sends it)
        //   Client   → MessageReceiver<M>  (client receives it)
        app.register_message::<ServerStartMessage>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<ServerTurnMessage>()
            .add_direction(NetworkDirection::ServerToClient);
    }
}
