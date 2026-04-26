//! Dark Rift Server — Pure Turn-Relay (mirrors OpenFront's GameServer.ts)
//!
//! The server does NOT simulate the game. It:
//!   1. Accepts WebSocket connections via Lightyear
//!   2. Assigns player IDs and spawn points on connect
//!   3. Collects incoming ClientGameplayMessage into an IntentBuffer
//!   4. At fixed TURN_INTERVAL, bundles intents into a Turn → broadcasts ServerTurnMessage
//!   5. Sends ServerStartMessage with seed + player roster + missed turns on join
//!
//! Each client runs the identical shared ECS systems to simulate game state independently.

use bevy::prelude::*;
use bevy::app::ScheduleRunnerPlugin;
use lightyear::prelude::server::*;
use std::time::Duration;
use dark_rift_shared::protocol::*;

pub mod core;
pub mod network;
pub mod lobby;

use crate::core::*;

fn main() {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
        Duration::from_millis(16),
    )));
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_plugins(SharedProtocolPlugin);
    app.add_plugins(ServerPlugins::default());

    app.init_resource::<ClientRegistry>()
       .init_resource::<MasterLobbyIdGen>();

    app.add_systems(Startup, network::setup);
    app.add_systems(Update, (
        lobby::system_master_lobby,
        lobby::system_lobby_gc,
        lobby::system_start_matches,
        network::handle_client_joins,
        network::handle_client_leave,
        lobby::system_broadcast_lobbies,
        network::handle_intent_messages,
        lobby::system_end_turn,
        lobby::system_dump_replay,
    ));

    app.run();
}
