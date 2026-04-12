//! Dark Rift Server — Pure Turn-Relay (mirrors OpenFront's GameServer.ts)
//!
//! The server does NOT simulate the game. It:
//!   1. Accepts WebSocket connections via Lightyear
//!   2. Assigns player IDs and spawn points on connect
//!   3. Collects incoming ClientIntentMessage into an IntentBuffer
//!   4. At fixed TURN_INTERVAL, bundles intents into a Turn → broadcasts ServerTurnMessage
//!   5. Sends ServerStartMessage with seed + player roster + missed turns on join
//!
//! Each client runs the identical shared ECS systems to simulate game state independently.

use bevy::prelude::*;
use bevy::app::ScheduleRunnerPlugin;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use lightyear::netcode::NetcodeServer;
use lightyear::websocket::server::{WebSocketServerIo, ServerConfig as WebSocketConfig};
use std::time::Duration;
use std::net::{Ipv4Addr, SocketAddr};

use dark_rift_shared::config;
use dark_rift_shared::player::player_colors;
use dark_rift_shared::protocol::*;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Turn interval: how often the server bundles + broadcasts a turn.
/// Mirrors OpenFront's config.turnIntervalMs() (200ms = 5 turns/sec).
const TURN_INTERVAL_MS: f32 = 50.0;

/// Auto-start when this many players connect.
const MIN_PLAYERS: usize = 2;

// ─── Resources ──────────────────────────────────────────────────────────────

/// Maps Bevy Entity (ClientOf) → our game PlayerId.
#[derive(Resource, Default)]
struct ClientPlayerMap {
    entries: Vec<(Entity, u16)>,
    next_id: u16,
}

impl ClientPlayerMap {
    fn player_id(&self, entity: Entity) -> Option<u16> {
        self.entries.iter().find(|(e, _)| *e == entity).map(|(_, id)| *id)
    }

    fn register(&mut self, entity: Entity) -> u16 {
        self.next_id += 1;
        self.entries.push((entity, self.next_id));
        self.next_id
    }

    fn count(&self) -> usize { self.entries.len() }
    
    fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entries.iter().map(|(e, _)| *e)
    }
}

/// Accumulates StampedIntents between turns.
/// Mirrors OpenFront's `private intents: StampedIntent[]`.
#[derive(Resource, Default)]
struct IntentBuffer(Vec<StampedIntent>);

/// All turns broadcast so far — needed for late-join catchup.
/// Mirrors OpenFront's `private turns: Turn[]`.
#[derive(Resource, Default)]
struct TurnLog {
    turns: Vec<Turn>,
    current: u64,
}

/// Full player roster (for ServerStartMessage).
#[derive(Resource, Default)]
struct PlayerRoster(Vec<PlayerInfo>);

/// Game seed + started flag.
#[derive(Resource)]
struct GameMetadata {
    seed: u64,
    started: bool,
}

impl Default for GameMetadata {
    fn default() -> Self { Self { seed: 42, started: false } }
}

/// Turn dispatch timer.
#[derive(Resource)]
struct TurnTimer(Timer);

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
        Duration::from_millis(16),
    )));
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_plugins(SharedProtocolPlugin);
    app.add_plugins(ServerPlugins::default());

    app.init_resource::<ClientPlayerMap>()
       .init_resource::<IntentBuffer>()
       .init_resource::<TurnLog>()
       .init_resource::<PlayerRoster>()
       .init_resource::<GameMetadata>();

    app.insert_resource(TurnTimer(Timer::from_seconds(
        TURN_INTERVAL_MS / 1000.0,
        TimerMode::Repeating,
    )));

    app.add_systems(Startup, setup);
    app.add_systems(Update, (
        handle_new_clients,
        handle_intent_messages,
        system_end_turn,
    ));

    app.run();
}

// ─── Startup ────────────────────────────────────────────────────────────────

fn setup(mut commands: Commands) {
    let server_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 5005);

    let ws_config = WebSocketConfig::builder()
        .with_bind_address(server_addr)
        .with_no_encryption();

    let server_entity = commands.spawn((
        Name::from("DarkRiftServer"),
        NetcodeServer::new(NetcodeConfig::default()),
        LocalAddr(server_addr),
        WebSocketServerIo { config: ws_config },
    )).id();

    commands.trigger(Start { entity: server_entity });

    info!("🚀 Dark Rift relay server → ws://0.0.0.0:5005");
    info!("   Waiting for {} players...", MIN_PLAYERS);
}

// ─── Connection Handling ────────────────────────────────────────────────────

/// Detect new connected ClientOf entities, register them, auto-start after MIN_PLAYERS.
/// Mirrors OpenFront's GameServer.ts onConnect() + startGameIfReady() → sendToAll().
fn handle_new_clients(
    mut client_map: ResMut<ClientPlayerMap>,
    mut roster:     ResMut<PlayerRoster>,
    mut metadata:   ResMut<GameMetadata>,
    turn_log:       Res<TurnLog>,
    mut senders:    Query<&mut MessageSender<ServerStartMessage>>,
    new_clients:    Query<Entity, Added<ClientOf>>,
) {
    for entity in new_clients.iter() {
        // Verify the MessageSender component is present (it should be, via add_direction)
        if senders.get_mut(entity).is_err() {
            error!("Client {:?} missing MessageSender<ServerStartMessage>! Skipping.", entity);
            continue;
        }

        let player_id = client_map.register(entity);
        let colors = player_colors();
        let color = colors[(player_id as usize - 1) % colors.len()];
        let name = format!("Player {}", player_id);
        let (sx, sy) = pick_spawn(&roster, player_id);

        roster.0.push(PlayerInfo { id: player_id, name: name.clone(), color, spawn_x: sx, spawn_y: sy });
        info!("✅ {} (id={}) joined → spawn({},{})", name, player_id, sx, sy);

        let just_started = !metadata.started && client_map.count() >= MIN_PLAYERS;
        if just_started {
            metadata.started = true;
            info!("⚔️  {} players — GAME START!", client_map.count());
        }

        if !metadata.started {
            continue; // Still waiting for more players, don't send start yet
        }

        if just_started {
            // ── Broadcast to ALL players simultaneously ──────────────────────
            // Mirrors OpenFront's startGameIfReady() → sendToAll(startGameMsg).
            // Snapshot entries to avoid borrow-checker issues with the query.
            let all: Vec<(Entity, u16)> = client_map.entries.clone();
            for (target, their_id) in all {
                let msg = ServerStartMessage {
                    my_player_id: their_id,
                    seed: metadata.seed,
                    players: roster.0.clone(),
                    missed_turns: turn_log.turns.clone(),
                };
                if let Ok(mut s) = senders.get_mut(target) {
                    info!("📨 → ServerStartMessage to Player {} ({:?})", their_id, target);
                    let _ = s.send::<ReliableChannel>(msg);
                }
            }
        } else {
            // ── Late joiner: catch up with turn history ───────────────────────
            let msg = ServerStartMessage {
                my_player_id: player_id,
                seed: metadata.seed,
                players: roster.0.clone(),
                missed_turns: turn_log.turns.clone(),
            };
            if let Ok(mut s) = senders.get_mut(entity) {
                info!("📨 → ServerStartMessage (late-join) to Player {} ({:?})", player_id, entity);
                let _ = s.send::<ReliableChannel>(msg);
            }
        }
    }
}


/// Collect incoming intents, stamp with player_id.
/// Mirrors OpenFront's: intents.push({ ...clientMsg.intent, clientID }).
fn handle_intent_messages(
    client_map:   Res<ClientPlayerMap>,
    metadata:     Res<GameMetadata>,
    mut buffer:   ResMut<IntentBuffer>,
    mut receivers: Query<(Entity, &mut MessageReceiver<ClientIntentMessage>)>,
) {
    if !metadata.started { return; }

    for (entity, mut receiver) in receivers.iter_mut() {
        if let Some(player_id) = client_map.player_id(entity) {
            for msg in receiver.receive() {
                buffer.0.push(StampedIntent {
                    player_id,
                    intent: msg.intent,
                });
            }
        }
    }
}

/// Timer-driven: bundle intents → broadcast Turn to all clients.
/// Mirrors OpenFront's endTurn() called by setInterval(endTurn, turnIntervalMs).
fn system_end_turn(
    time:       Res<Time>,
    mut timer:  ResMut<TurnTimer>,
    mut buffer: ResMut<IntentBuffer>,
    mut log:    ResMut<TurnLog>,
    metadata:   Res<GameMetadata>,
    mut senders: Query<&mut MessageSender<ServerTurnMessage>>,
) {
    if !metadata.started { return; }

    timer.0.tick(time.delta());
    if !timer.0.just_finished() { return; }

    let turn = Turn {
        turn_number: log.current,
        intents: std::mem::take(&mut buffer.0),
    };
    log.current += 1;
    log.turns.push(turn.clone());

    let msg = ServerTurnMessage { turn };
    for mut sender in senders.iter_mut() {
        let _ = sender.send::<ReliableChannel>(msg.clone());
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn pick_spawn(roster: &PlayerRoster, _player_id: u16) -> (u32, u32) {
    let w = config::MAP_WIDTH;
    let h = config::MAP_HEIGHT;
    let margin = config::SPAWN_RADIUS + 4;

    let candidates: [(u32, u32); 8] = [
        (w / 4,     h / 4),
        (3 * w / 4, h / 4),
        (w / 4,     3 * h / 4),
        (3 * w / 4, 3 * h / 4),
        (w / 2,     h / 4),
        (w / 2,     3 * h / 4),
        (w / 4,     h / 2),
        (3 * w / 4, h / 2),
    ];

    let used: std::collections::HashSet<(u32, u32)> =
        roster.0.iter().map(|p| (p.spawn_x, p.spawn_y)).collect();

    for &(sx, sy) in &candidates {
        let sx = sx.clamp(margin, w.saturating_sub(margin));
        let sy = sy.clamp(margin, h.saturating_sub(margin));
        if !used.contains(&(sx, sy)) {
            return (sx, sy);
        }
    }

    (w / 2, h / 2)
}
