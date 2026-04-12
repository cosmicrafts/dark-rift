use bevy::prelude::*;
use bevy::app::ScheduleRunnerPlugin;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use lightyear::netcode::NetcodeServer;
use lightyear::websocket::server::{WebSocketServerIo, ServerConfig as WebSocketConfig};
use std::time::Duration;
use std::net::{Ipv4Addr, SocketAddr};

use dark_rift_shared::config;
use dark_rift_shared::game::GameState;
use dark_rift_shared::player::{Player, player_colors};
use dark_rift_shared::protocol::*;
use dark_rift_shared::execution::{AttackExecution, PrioritizedTile, system_attack_execution, system_player_income};
use dark_rift_shared::pseudo_random::PseudoRandom;

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
    
    app.add_systems(Update, (
        handle_new_clients,
        handle_player_actions,
        game_tick,
    ));
    // Shared executions run on the server to apply GameState changes deterministically
    app.add_systems(Update, (
        system_player_income,
        system_attack_execution,
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

fn handle_new_clients(
    mut game: ResMut<GameState>,
    mut client_map: ResMut<ClientPlayerMap>,
    senders: Query<Entity, With<MessageSender<SyncMapState>>>,
) {
    for entity in senders.iter() {
        if client_map.get_player_id(entity).is_some() {
            continue; // Already registered
        }

        let player_id = client_map.register(entity);
        let colors = player_colors();
        let color_idx = (player_id as usize - 1) % colors.len();
        let color = colors[color_idx];
        let name = format!("Player {}", player_id);

        let player = Player::new_human(player_id, name.clone(), color);

        let (sx, sy) = find_spawn_location(&game, player_id);

        game.spawn_player(player, sx, sy);
        info!("Player '{}' (id={}) spawned at ({}, {})", name, player_id, sx, sy);
    }
}

fn find_spawn_location(game: &GameState, _player_id: u16) -> (u32, u32) {
    let w = game.map.width;
    let h = game.map.height;
    let margin = config::SPAWN_RADIUS + 2;

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

    for &(sx, sy) in &spawn_points {
        let sx = sx.clamp(margin, w - margin);
        let sy = sy.clamp(margin, h - margin);
        if game.map.owner_id(sx, sy) == 0 {
            return (sx, sy);
        }
    }

    (w / 2, h / 2)
}

/// Receive PlayerActionMessage and spawn AttackExecution logic queue (BFS).
fn handle_player_actions(
    mut commands: Commands,
    game: Res<GameState>,
    client_map: Res<ClientPlayerMap>,
    mut receivers: Query<(&mut MessageReceiver<PlayerActionMessage>, Entity)>,
) {
    for (mut receiver, entity) in receivers.iter_mut() {
        let player_id = match client_map.get_player_id(entity) {
            Some(id) => id,
            None => continue,
        };

        for msg in receiver.receive() {
            let player = match game.player(player_id) {
                Some(p) => p,
                None => continue,
            };

            let target_owner = msg.target_owner.unwrap_or(0); // 0 = TerraNullius/neutral
            
            // By default use all available troops, or specified.
            let troops = msg.troops.unwrap_or_else(|| player.troops);

            let mut rng = PseudoRandom::new(game.seed + game.tick);
            let mut to_conquer = std::collections::BinaryHeap::new();

            // Seed the BFS with tiles corresponding to target frontiers
            for (bx, by) in game.map.border_tiles_of(player_id) {
                // Must border the target owner to begin expansion there
                let mut valid_border = false;
                for (nx, ny) in game.map.neighbors(bx, by) {
                    if game.map.owner_id(nx, ny) == target_owner {
                        valid_border = true;
                        
                        // Push neighbor into conquest queue matching AttackExecution logic
                        let mut num_owned_by_me = 0;
                        for (nnx, nny) in game.map.neighbors(nx, ny) {
                            if game.map.owner_id(nnx, nny) == player_id {
                                num_owned_by_me += 1;
                            }
                        }
                        
                        let terrain = game.map.terrain_type(nx, ny);
                        let r = rng.next_int(0, 7) as f32;
                        let mag = match terrain {
                            dark_rift_shared::map::TerrainType::Plains => 1.0,
                            dark_rift_shared::map::TerrainType::Highland => 1.5,
                            dark_rift_shared::map::TerrainType::Mountain => 2.0,
                            _ => 1.0,
                        };
                        let prio = (r + 10.0) * (1.0 - (num_owned_by_me as f32) * 0.5 + mag / 2.0) + (game.tick as f32);

                        to_conquer.push(PrioritizedTile { priority: prio, x: nx, y: ny });
                    }
                }
            }

            if !to_conquer.is_empty() {
                commands.spawn(AttackExecution {
                    owner_id: player_id,
                    target_owner,
                    troops,
                    to_conquer,
                    rng,
                });
            }
        }
    }
}

fn game_tick(
    mut game: ResMut<GameState>,
    mut timer: ResMut<GameTickTimer>,
    time: Res<Time>,
) {
    timer.0.tick(time.delta());
    if timer.0.just_finished() {
        game.tick(); // Increment tick counter
    }
}

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
