use bevy::prelude::*;
use bevy::app::ScheduleRunnerPlugin;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use lightyear::netcode::NetcodeServer;
use lightyear::websocket::server::{WebSocketServerIo, ServerConfig as WebSocketConfig, Identity};use std::time::Duration;
use std::net::{Ipv4Addr, SocketAddr};

use dark_rift_shared::config;
use dark_rift_shared::game::GameState;
use dark_rift_shared::player::{Player, player_colors};
use dark_rift_shared::protocol::*;

// ─── ECS Components ─────────────────────────────────────────────────────────

/// A task representing an active expansion flood from a player toward a target.
#[derive(Component, Debug, Clone)]
pub struct ExpansionExecution {
    pub player_id: u16,
    pub target_x: u32,
    pub target_y: u32,
}

// ─── Resources ──────────────────────────────────────────────────────────────

/// Maps Bevy Entity → PlayerId so we know who sent what.
#[derive(Resource, Default)]
struct ClientPlayerMap {
    entries: Vec<(Entity, u16)>,
    next_id: u16,
}

impl ClientPlayerMap {
    fn get_player_id(&self, entity: Entity) -> Option<u16> {
        self.entries.iter().find(|(e, _)| *e == entity).map(|(_, id)| *id)
    }

    fn register(&mut self, entity: Entity) -> u16 {
        self.next_id += 1;
        let id = self.next_id;
        self.entries.push((entity, id));
        id
    }
}

/// Timer for the game tick (runs at TICK_RATE).
#[derive(Resource)]
struct GameTickTimer(Timer);

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
        Duration::from_secs_f64(1.0 / 60.0), // Bevy loop at 60fps
    )));
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_plugins(SharedProtocolPlugin);
    app.add_plugins(ServerPlugins::default());

    app.insert_resource(ClientPlayerMap::default());
    app.insert_resource(GameTickTimer(Timer::from_seconds(
        1.0 / config::TICK_RATE as f32,
        TimerMode::Repeating,
    )));

    app.add_systems(Startup, setup);
    
    // Grouping correctly to respect Bevy tuple limits
    app.add_systems(Update, (
        handle_new_clients,
        handle_player_actions,
        game_tick,
    ));
    app.add_systems(Update, (
        system_player_execution,
        system_process_expansions,
        broadcast_state,
    ));

    app.run();
}

// ─── Startup ────────────────────────────────────────────────────────────────

fn setup(mut commands: Commands) {
    let game = GameState::new(42);
    info!("Server starting with Map Grid: {}x{}", game.map.width, game.map.height);
    commands.insert_resource(game);

    let server_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 5000);

    // For local MVP testing where the client is Native on OS loopback, use `with_no_encryption()`.
    // Once you compile the client to WASM and host on HTTPS, change this back to:
    // .with_identity(Identity::self_signed(vec!["localhost"...]).unwrap())
    let ws_config = WebSocketConfig::builder()
        .with_bind_address(server_addr)
        .with_no_encryption();

    let server_entity = commands.spawn((
        Name::from("Server"),
        NetcodeServer::new(NetcodeConfig::default()),
        LocalAddr(server_addr),
        WebSocketServerIo { config: ws_config },
    )).id();

    commands.trigger(Start { entity: server_entity });
}

// ─── Systems ────────────────────────────────────────────────────────────────

/// Detect new client connections. 
/// Using `MessageSender` means we wait until Lightyear sets up the entity's send/recv channels!
fn handle_new_clients(
    mut game: ResMut<GameState>,
    mut client_map: ResMut<ClientPlayerMap>,
    senders: Query<Entity, With<MessageSender<SyncMapState>>>,
) {
    for entity in senders.iter() {
        if client_map.get_player_id(entity).is_some() {
            continue; // Already registered
        }

        // Register a new player
        let player_id = client_map.register(entity);
        let colors = player_colors();
        let color_idx = (player_id as usize - 1) % colors.len();
        let color = colors[color_idx];
        let name = format!("Player {}", player_id);

        let player = Player::new_human(player_id, name.clone(), color);

        // Find a good spawn location (away from other players)
        let (sx, sy) = find_spawn_location(&game, player_id);

        game.spawn_player(player, sx, sy);
        info!("Player '{}' (id={}) spawned at ({}, {})", name, player_id, sx, sy);
    }
}

/// Find a spawn location that's reasonably far from existing players.
fn find_spawn_location(game: &GameState, _player_id: u16) -> (u32, u32) {
    let w = game.map.width;
    let h = game.map.height;
    let margin = config::SPAWN_RADIUS + 2;

    // Predefined spawn positions distributed across the map
    let spawn_points = [
        (w / 4, h / 4),
        (3 * w / 4, h / 4),
        (w / 4, 3 * h / 4),
        (3 * w / 4, 3 * h / 4),
        (w / 2, h / 4),
        (w / 2, 3 * h / 4),
        (w / 4, h / 2),
        (3 * w / 4, h / 2),
    ];

    // Find the first spawn point that doesn't overlap existing territory
    for &(sx, sy) in &spawn_points {
        let sx = sx.clamp(margin, w - margin);
        let sy = sy.clamp(margin, h - margin);
        if game.map.owner_id(sx, sy) == 0 {
            return (sx, sy);
        }
    }

    // Fallback: center
    (w / 2, h / 2)
}

/// Handle player click actions: queue expansion toward the target.
fn handle_player_actions(
    mut commands: Commands,
    mut game: ResMut<GameState>,
    client_map: Res<ClientPlayerMap>,
    mut receivers: Query<(&mut MessageReceiver<PlayerActionMessage>, Entity)>,
) {
    for (mut receiver, entity) in receivers.iter_mut() {
        let player_id = match client_map.get_player_id(entity) {
            Some(id) => id,
            None => continue,
        };

        for msg in receiver.receive() {
            if !game.map.is_valid_coord(msg.x as i32, msg.y as i32) {
                continue;
            }

            // Queue the expansion by spawning an Execution Entity
            commands.spawn(ExpansionExecution {
                player_id,
                target_x: msg.x,
                target_y: msg.y,
            });
        }
    }
}

/// Run the authoritative game tick at TICK_RATE.
fn game_tick(
    mut game: ResMut<GameState>,
    mut timer: ResMut<GameTickTimer>,
    time: Res<Time>,
) {
    timer.0.tick(time.delta());
    if timer.0.just_finished() {
        game.tick();
    }
}

/// Passive troop generation system (replaces TypeScript PlayerExecution).
/// Runs exactly in sync with the game tick clock.
fn system_player_execution(
    mut game: ResMut<GameState>,
    timer: Res<GameTickTimer>,
) {
    if !timer.0.just_finished() || game.phase != dark_rift_shared::game::GamePhase::Playing {
        return;
    }

    // Destructure to appease the borrow checker
    let GameState { map, players, .. } = &mut *game;

    for player in players.iter_mut() {
        if !player.alive {
            continue;
        }

        let tiles_owned = map.tiles_owned_by(player.id);
        if tiles_owned == 0 {
            player.alive = false;
            continue;
        }

        // Update max troops based on territory size
        player.max_troops = config::MAX_TROOPS_BASE
            + (tiles_owned as f64).powf(0.6) * config::MAX_TROOPS_SCALE;

        // Generate income: base + territory bonus, scaled by how full we are
        let raw_income = config::TROOP_BASE_INCOME
            + (player.troops.powf(0.73) / 4.0);
        let ratio = 1.0 - (player.troops / player.max_troops).min(1.0);
        let income = raw_income * ratio;

        player.troops = (player.troops + income).min(player.max_troops);
    }
}

/// System for processing all active ExpansionExecution tasks on the server map.
/// Replaces Openfront's AttackExecution looping logic.
fn system_process_expansions(
    mut commands: Commands,
    mut game: ResMut<GameState>,
    timer: Res<GameTickTimer>,
    query: Query<(Entity, &ExpansionExecution)>,
) {
    if !timer.0.just_finished() || game.phase != dark_rift_shared::game::GamePhase::Playing {
        return;
    }

    let GameState { map, players, .. } = &mut *game;

    for (entity, execution) in query.iter() {
        let pid = execution.player_id;
        
        // Use an inner block to drop the mutable borrow if we despawn
        let player = match players.iter_mut().find(|p| p.id == pid) {
            Some(p) if p.alive => p,
            _ => {
                commands.entity(entity).despawn();
                continue;
            }
        };

        // Already reached the target?
        if map.owner_id(execution.target_x, execution.target_y) == pid {
            commands.entity(entity).despawn();
            continue;
        }

        // Not enough troops to keep expanding? Let it pause until troops rebuild.
        if player.troops < config::EXPAND_COST {
            continue;
        }

        // Find border tiles nearest to the target and expand from them
        let mut candidates: Vec<(u32, u32, f64)> = Vec::new();
        for y in 0..map.height {
            for x in 0..map.width {
                let owner = map.owner_id(x, y);
                if owner != 0 {
                    continue; // Not neutral
                }
                if !map.is_adjacent_to_player(x, y, pid) {
                    continue; // Not adjacent to our territory
                }
                let dx = x as f64 - execution.target_x as f64;
                let dy = y as f64 - execution.target_y as f64;
                let dist = (dx * dx + dy * dy).sqrt();
                candidates.push((x, y, dist));
            }
        }

        // Sort by distance to target (closest first)
        candidates.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());

        // Expand up to N tiles this tick
        let mut expanded_this_tick = 0u32;
        for (cx, cy, _dist) in &candidates {
            if expanded_this_tick >= config::EXPAND_TILES_PER_TICK {
                break;
            }
            if player.troops < config::EXPAND_COST {
                break;
            }

            map.set_owner_id(*cx, *cy, pid);
            player.troops -= config::EXPAND_COST;
            expanded_this_tick += 1;
        }
    }
}

/// Broadcast the full game state to all clients whenever it changes.
fn broadcast_state(
    game: Res<GameState>,
    mut clients: Query<&mut MessageSender<SyncMapState>>,
) {
    if !game.is_changed() { return; }

    let msg = SyncMapState { state: Some(game.clone()) };
    for mut sender in clients.iter_mut() {
        let _ = sender.send::<dark_rift_shared::protocol::ReliableChannel>(msg.clone());
    }
}
