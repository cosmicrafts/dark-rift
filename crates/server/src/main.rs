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
use std::collections::HashMap;
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
const TURN_INTERVAL_MS: f32 = 40.0;

/// Auto-start timer when lobby reaches top of queue 
const LOBBY_COUNTDOWN_SECS: f32 = 15.0;

// ─── Componentes del Lobby (Phase 1) ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LobbyPhase {
    Waiting,
    CountingDown,
    Active,
    Finished,
}

#[derive(Component)]
struct Lobby {
    id: u64,
}

#[derive(Component, PartialEq)]
struct StatePhase(LobbyPhase);

#[derive(Component)]
pub struct MatchMetadata {
    pub seed: u64,
    pub config: dark_rift_shared::game_config::GameConfig,
    pub map_cache: Option<dark_rift_shared::map::GameMap>,
    pub manifest_cache: Option<dark_rift_shared::map_loader::MapManifest>,

}

#[derive(Component)]
struct CountdownTimer(Timer);

#[derive(Component)]
struct ActiveEmptyTimer(Timer);

#[derive(Component, Default)]
struct MatchRoster(Vec<PlayerInfo>);

#[derive(Component, Default)]
struct MatchTurnLog {
    turns: Vec<Turn>,
    current: u64,
}

#[derive(Component, Default)]
struct MatchIntentBuffer(Vec<StampedIntent>);

#[derive(Component)]
struct MatchTimer(Timer);

#[derive(Component)]
struct HasSentStart(bool);

// ─── Resources ──────────────────────────────────────────────────────────────

/// Maps Bevy Entity (Client) -> (MatchEntity, Option<PlayerId>)
#[derive(Resource, Default)]
struct ClientRegistry {
    clients: HashMap<Entity, (Entity, Option<u16>)>,
}

#[derive(Resource, Default)]
struct MasterLobbyIdGen(u64);

// ─── Main ───────────────────────────────────────────────────────────────────

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

    app.add_systems(Startup, setup);
    app.add_systems(Update, (
        system_master_lobby,
        system_lobby_gc,
        system_start_matches,
        handle_client_joins,
        handle_client_leave,
        system_broadcast_lobbies,
        handle_intent_messages,
        system_end_turn,
        system_dump_replay,
    ));

    app.run();
}

// ─── Startup ────────────────────────────────────────────────────────────────

fn setup(mut commands: Commands) {
    let server_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 25565);

    let ws_config = WebSocketConfig::builder()
        .with_bind_address(server_addr)
        .with_no_encryption();

    let mut netcode_config = NetcodeConfig::default();
    netcode_config.client_timeout_secs = 120; // Wait generously to allow observers long catch-ups

    let server_entity = commands.spawn((
        Name::from("DarkRiftServer"),
        NetcodeServer::new(netcode_config),
        LocalAddr(server_addr),
        WebSocketServerIo { config: ws_config },
    )).id();

    commands.trigger(Start { entity: server_entity });

    info!("🚀 Dark Rift LOBBY relay server → ws://0.0.0.0:25565");
}

// ─── Sistemas ───────────────────────────────────────────────────────────────

/// Implements OpenFront's "Always Active" Lobby Pattern
fn system_master_lobby(
    mut commands: Commands,
    mut id_gen: ResMut<MasterLobbyIdGen>,
    time: Res<Time>,
    mut q_lobbies: Query<(Entity, &Lobby, &mut StatePhase, &mut MatchRoster, &MatchMetadata, Option<&mut CountdownTimer>)>,
) {
    let mut lobby_phases = Vec::new();
    
    for (entity, _lobby, phase, _roster, _meta, _timer) in q_lobbies.iter() {
        if phase.0 == LobbyPhase::Waiting || phase.0 == LobbyPhase::CountingDown {
            lobby_phases.push((entity, phase.0));
        }
    }
    
    // Regla 1: Un solo lobby en cola (Waiting / CountingDown). Evita duplicar tarjetas en el cliente
    // y coincide con el join que prioriza una sala canónica.
    while lobby_phases.len() < 1 {
        let lobby_id = id_gen.0;
        id_gen.0 += 1;
        
        let mut config = dark_rift_shared::game_config::GameConfig::default();
        
        let maps = ["europe", "world"];
        use rand::Rng;
        let mut rng = rand::thread_rng();
        config.map_name = maps[rng.gen_range(0..maps.len())].to_string();

        let loaded = dark_rift_shared::map_loader::load_openfront_map(&config.map_name);
        let (map_cache, manifest_cache) = match loaded {
            Some((map, manifest)) => {
                config.map_width = manifest.map.width;
                config.map_height = manifest.map.height;
                (Some(map), Some(manifest))
            },
            None => (None, None),
        };

        let new_lobby = commands.spawn((
            Lobby { id: lobby_id },
            StatePhase(LobbyPhase::Waiting),
            MatchMetadata { seed: 42 + lobby_id, config, map_cache, manifest_cache },
            MatchRoster::default(),
            MatchTurnLog::default(),
            MatchIntentBuffer::default(),
            HasSentStart(false),
            MatchTimer(Timer::from_seconds(TURN_INTERVAL_MS / 1000.0, TimerMode::Repeating)),
        )).id();
        info!("🏟️  [Master] Generado nuevo Lobby de backup #{}", lobby_id);
        lobby_phases.push((new_lobby, LobbyPhase::Waiting));
    }
    
    // Regla 2: Un Lobby debe estar contando
    let has_counting_down = lobby_phases.iter().any(|(_, p)| *p == LobbyPhase::CountingDown);
    if !has_counting_down {
        if let Some((first_waiting_entity, _)) = lobby_phases.iter().find(|(_, p)| *p == LobbyPhase::Waiting) {
            commands.entity(*first_waiting_entity).insert(CountdownTimer(Timer::from_seconds(LOBBY_COUNTDOWN_SECS, TimerMode::Once)));
            if let Ok((_, _, mut phase, _, _, _)) = q_lobbies.get_mut(*first_waiting_entity) {
                phase.0 = LobbyPhase::CountingDown;
            }
            info!("⏳  [Master] Lobby Promovido a CountingDown (comienza la cuenta regresiva)");
        }
    }
    
    // Tick y progreso a Active
    for (entity, lobby, mut phase, mut roster, meta, timer_opt) in q_lobbies.iter_mut() {
        if phase.0 == LobbyPhase::CountingDown {
            if let Some(mut timer) = timer_opt {
                timer.0.tick(time.delta());
                
                // Si el timer termina o se llena el lobby (550 max)
                if timer.0.just_finished() || roster.0.len() >= 550 {
                    phase.0 = LobbyPhase::Active;
                    commands.entity(entity).remove::<CountdownTimer>();
                    commands.entity(entity).insert(ActiveEmptyTimer(Timer::from_seconds(30.0, TimerMode::Once)));
                    
                    // Spawn Tribal Bots locally in the server
                    let bot_count = meta.config.bot_count;
                    if bot_count > 0 {
                        info!("🤖 Spawning {} Tribal Bots into Lobby {}...", bot_count, lobby.id);
                        let prefixes = ["Akkadian", "Scythian", "Roman", "Greek", "Spartan", "Persian", "Minoan", "Etruscan", "Celtic", "Gallic", "Thracian", "Dacian", "Sumerian"];
                        let suffixes = ["Tribe", "Empire", "Kingdom", "Horde", "Clan", "Legion"];
                        
                        for _i in 0..bot_count {
                            let player_id = (roster.0.len() + 1) as u16;
                            let color = dark_rift_shared::player::bot_territory_color(meta.seed, player_id);
                            
                            let mut rng = dark_rift_shared::pseudo_random::PseudoRandom::new(meta.seed.wrapping_add(player_id as u64));
                            let p_idx = rng.next_int(0, prefixes.len() as i32) as usize;
                            let s_idx = rng.next_int(0, suffixes.len() as i32) as usize;
                            let mut bot_name = format!("👤 {} {}", prefixes[p_idx], suffixes[s_idx]);
                            
                            let (sx, sy, nation_opt) = pick_spawn(&roster, player_id, meta);
                            if let Some(nation_name) = nation_opt {
                                bot_name = nation_name;
                            }
                            
                            roster.0.push(PlayerInfo {
                                id: player_id,
                                name: bot_name,
                                player_type: dark_rift_shared::player::PlayerType::Bot,
                                color,
                                spawn_x: sx,
                                spawn_y: sy,
                            });
                        }
                    }
                    
                    info!("⚔️  [Master] Lobby {} es ahora ACTIVE con {} jugadores (incluyendo bots)!", lobby.id, roster.0.len());
                }
            }
        }
    }
}

fn system_lobby_gc(
    mut commands: Commands,
    time: Res<Time>,
    mut q_lobbies: Query<(Entity, &Lobby, &mut StatePhase, &MatchRoster, Option<&mut ActiveEmptyTimer>)>,
) {
    for (entity, lobby, mut phase, roster, timer_opt) in q_lobbies.iter_mut() {
        if phase.0 == LobbyPhase::Active {
            if roster.0.is_empty() {
                if let Some(mut timer) = timer_opt {
                    timer.0.tick(time.delta());
                    if timer.0.just_finished() {
                        info!("🗑️  [GC] Lobby {} estuvo vacío por 30s. Terminando...", lobby.id);
                        phase.0 = LobbyPhase::Finished;
                        commands.entity(entity).despawn();
                    }
                } else {
                    info!("🗑️  [GC] Todos los jugadores abandonaron el Lobby {}. Terminando...", lobby.id);
                    phase.0 = LobbyPhase::Finished;
                    commands.entity(entity).despawn();
                }
            } else {
                if timer_opt.is_some() {
                    commands.entity(entity).remove::<ActiveEmptyTimer>();
                }
            }
        }
    }
}

fn system_broadcast_lobbies(
    time: Res<Time>,
    mut next_broadcast: Local<f32>,
    q_lobbies: Query<(&Lobby, &StatePhase, &MatchRoster, &MatchMetadata, Option<&CountdownTimer>)>,
    mut senders: Query<&mut MessageSender<ServerLobbiesBroadcastMessage>>,
) {
    *next_broadcast += time.delta_secs();
    if *next_broadcast < 0.5 { return; }
    *next_broadcast -= 0.5;
    
    let mut infos = Vec::new();
    for (lobby, phase, roster, meta, timer) in q_lobbies.iter() {
        if phase.0 == LobbyPhase::Active || phase.0 == LobbyPhase::Finished { continue; }
        
        let timer_secs = if let Some(t) = timer {
            t.0.duration().as_secs_f32() - t.0.elapsed_secs()
        } else {
            0.0
        };
        
        let player_names = roster.0.iter().map(|p| p.name.clone()).collect();
        
        infos.push(LobbyInfo {
            id: lobby.id,
            num_players: roster.0.len() as u32,
            max_players: meta.config.max_players,
            is_counting_down: phase.0 == LobbyPhase::CountingDown,
            timer_secs,
            map_name: meta.config.map_name.clone(),
            player_names,
        });
    }

    infos.sort_by_key(|l| l.id);
    
    let msg = ServerLobbiesBroadcastMessage { lobbies: infos };
    for mut sender in senders.iter_mut() {
        let _ = sender.send::<ReliableChannel>(msg.clone());
    }
}

fn handle_client_joins(
    mut registry: ResMut<ClientRegistry>,
    mut q_lobbies: Query<(Entity, &Lobby, &StatePhase, &mut MatchRoster, &MatchMetadata, &MatchTurnLog)>,
    mut senders: Query<(Entity, &mut MessageSender<ServerStartMessage>)>,
    mut receivers: Query<(Entity, &mut MessageReceiver<ClientJoinMessage>)>,
) {
    for (client_entity, mut receiver) in receivers.iter_mut() {
        for msg in receiver.receive() {
            // Buscamos el Lobby solicitado por el cliente, si no hay buscamos CountingDown en su defecto
            let mut target_lobby = None;
            
            if let Some(target_id) = msg.target_lobby_id {
                for (entity, lobby, phase, _roster, _, _) in q_lobbies.iter() {
                    if lobby.id == target_id {
                        if phase.0 == LobbyPhase::Finished {
                            warn!("Client tried to join finished Lobby {}, redirecting...", target_id);
                        } else if phase.0 == LobbyPhase::Active && !msg.is_observer {
                            warn!("Client tried to join active Lobby {} as player, redirecting...", target_id);
                        } else {
                            target_lobby = Some(entity);
                        }
                        break;
                    }
                }
            }
            
            if target_lobby.is_none() {
                for (entity, _lobby, phase, _roster, _, _) in q_lobbies.iter() {
                    if phase.0 == LobbyPhase::CountingDown {
                        target_lobby = Some(entity);
                        break;
                    }
                }
            }
            if target_lobby.is_none() {
                for (entity, _lobby, phase, _roster, _, _) in q_lobbies.iter() {
                    if phase.0 == LobbyPhase::Waiting {
                        target_lobby = Some(entity);
                        break;
                    }
                }
            }
            if target_lobby.is_none() {
                for (entity, _lobby, phase, _roster, _, _) in q_lobbies.iter() {
                    if phase.0 == LobbyPhase::Active {
                        target_lobby = Some(entity);
                        break;
                    }
                }
            }
            
            let Some(match_entity) = target_lobby else {
                warn!("No lobby available to join!");
                continue;
            };
            
            let Ok((_, lobby, phase, mut roster, metadata, log)) = q_lobbies.get_mut(match_entity) else { continue; };
            
            let mut assigned_id = None;
            
            if msg.is_observer {
                info!("👁  Observer joined (name={})", msg.name);
            } else {
                let player_id = (roster.0.len() + 1) as u16;
                assigned_id = Some(player_id);
                
                let colors = player_colors();
                let color = colors[(player_id as usize - 1) % colors.len()];
                let (sx, sy, _nation_opt) = pick_spawn(&roster, player_id, metadata);

                let final_name = msg.name.clone();

                roster.0.push(PlayerInfo { id: player_id, name: final_name.clone(), player_type: dark_rift_shared::player::PlayerType::Human, color, spawn_x: sx, spawn_y: sy });
                info!("✅ {} (id={}) se unió al Match ID {:?} → spawn({},{})", final_name, player_id, lobby.id, sx, sy);
            }
            registry.clients.insert(client_entity, (match_entity, assigned_id));
            
            if phase.0 == LobbyPhase::Active {
                if let Ok((_, mut sender)) = senders.get_mut(client_entity) {
                    let start_msg = ServerStartMessage {
                        config: metadata.config.clone(),
                        my_player_id: assigned_id,
                        seed: metadata.seed,
                        players: roster.0.clone(),
                        missed_turns: log.turns.clone(),
                        map_data: None,
                    };
                    info!("📨  Enviando ServerStartMessage (late-join) a {:?}", assigned_id);
                    let _ = sender.send::<ReliableChannel>(start_msg);
                }
            }
        }
    }
}

fn handle_client_leave(
    mut registry: ResMut<ClientRegistry>,
    mut q_matches: Query<&mut MatchRoster>,
    mut receivers: Query<(Entity, &mut MessageReceiver<ClientLeaveMessage>)>,
    q_clients: Query<(), (With<MessageSender<ServerStartMessage>>, With<MessageReceiver<ClientJoinMessage>>)>,
) {
    let mut to_remove = Vec::new();
    
    // 1. Explicit Leave
    for (client_entity, mut receiver) in receivers.iter_mut() {
        for _ in receiver.receive() {
            to_remove.push(client_entity);
        }
    }
    
    // 2. Disconnects (ghosts)
    for (client_entity, _) in registry.clients.iter() {
        if !q_clients.contains(*client_entity) && !to_remove.contains(client_entity) {
            to_remove.push(*client_entity);
        }
    }
    
    // Process
    for client_entity in to_remove {
        if let Some((match_entity, player_id)) = registry.clients.remove(&client_entity) {
            if let Ok(mut roster) = q_matches.get_mut(match_entity) {
                if let Some(pid) = player_id {
                    roster.0.retain(|p| p.id != pid);
                    info!("👋 Jugador {} abandonó el lobby (Roster actualizado)", pid);
                } else {
                    info!("👁  Observer abandonó el lobby");
                }
            }
        }
    }
}

fn system_start_matches(
    registry: Res<ClientRegistry>,
    mut q_lobbies: Query<(Entity, &Lobby, &StatePhase, &MatchRoster, &MatchMetadata, &MatchTurnLog, &mut HasSentStart)>,
    mut senders: Query<(Entity, &mut MessageSender<ServerStartMessage>)>,
) {
    for (match_entity, lobby, phase, roster, meta, log, mut has_sent) in q_lobbies.iter_mut() {
        // Enviar a todos el "Start" cuando el Lobby cambia a Active
        if phase.0 == LobbyPhase::Active && !has_sent.0 {
            has_sent.0 = true;
            info!("📨  Haciendo Broadcast de ServerStartMessage a todos los clientes del Lobby {}", lobby.id);
            for (client_entity, mut sender) in senders.iter_mut() {
                if let Some(&(stored_match, player_id)) = registry.clients.get(&client_entity) {
                    if stored_match == match_entity {
                        let start_msg = ServerStartMessage {
                            config: meta.config.clone(),
                            my_player_id: player_id, 
                            seed: meta.seed,
                            players: roster.0.clone(),
                            missed_turns: log.turns.clone(),
                            map_data: None,
                        };
                        let _ = sender.send::<ReliableChannel>(start_msg);
                    }
                }
            }
        }
    }
}

fn handle_intent_messages(
    registry: Res<ClientRegistry>,
    mut q_matches: Query<(&StatePhase, &mut MatchIntentBuffer)>,
    mut receivers: Query<(Entity, &mut MessageReceiver<ClientGameplayMessage>)>,
) {
    for (client_entity, mut receiver) in receivers.iter_mut() {
        if let Some(&(match_entity, Some(player_id))) = registry.clients.get(&client_entity) {
            if let Ok((phase, mut buffer)) = q_matches.get_mut(match_entity) {
                if phase.0 == LobbyPhase::Active {
                    for msg in receiver.receive() {
                        buffer.0.push(StampedIntent {
                            player_id,
                            intent: msg.intent,
                        });
                    }
                } else {
                    for _ in receiver.receive() {} // clear buffer
                }
            }
        } else {
            for _ in receiver.receive() {}
        }
    }
}

fn system_end_turn(
    time: Res<Time>,
    registry: Res<ClientRegistry>,
    mut q_matches: Query<(Entity, &StatePhase, &mut MatchTimer, &mut MatchIntentBuffer, &mut MatchTurnLog)>,
    mut senders: Query<(Entity, &mut MessageSender<ServerTurnMessage>)>,
) {
    for (match_entity, phase, mut timer, mut buffer, mut log) in q_matches.iter_mut() {
        if phase.0 != LobbyPhase::Active { continue; }

        timer.0.tick(time.delta());
        if !timer.0.just_finished() { continue; }

        let turn = Turn {
            turn_number: log.current,
            intents: std::mem::take(&mut buffer.0),
        };
        log.current += 1;
        log.turns.push(turn.clone());

        let msg = ServerTurnMessage { turn };
        for (client_entity, mut sender) in senders.iter_mut() {
            if let Some(&(stored_match, _)) = registry.clients.get(&client_entity) {
                if stored_match == match_entity {
                    let _ = sender.send::<ReliableChannel>(msg.clone());
                }
            }
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn pick_spawn(roster: &MatchRoster, player_id: u16, metadata: &MatchMetadata) -> (u32, u32, Option<String>) {
    let w = metadata.config.map_width;
    let h = metadata.config.map_height;
    let margin = config::SPAWN_RADIUS + 4;

    let used: std::collections::HashSet<(u32, u32)> =
        roster.0.iter().map(|p| (p.spawn_x, p.spawn_y)).collect();

    // 1. Try to use predefined nations from the manifest
    if let Some(manifest) = &metadata.manifest_cache {
        // Simple deterministic allocation based on player_id (1-indexed)
        if !manifest.nations.is_empty() {
            let n_idx = ((player_id.saturating_sub(1)) as usize) % manifest.nations.len();
            let coords = manifest.nations[n_idx].coordinates;
            let nation_name = manifest.nations[n_idx].name.clone();
            let sx = coords[0].clamp(margin, w.saturating_sub(margin));
            let sy = coords[1].clamp(margin, h.saturating_sub(margin));
            if !used.contains(&(sx, sy)) {
                return (sx, sy, Some(nation_name));
            }
        }
    }

    // 2. Fallback: Search the map for a valid land tile using a deterministic spiral/hash
    if let Some(map) = &metadata.map_cache {
        let mut rng = dark_rift_shared::pseudo_random::PseudoRandom::new(metadata.seed + player_id as u64);
        for _ in 0..1000 { // Attempt 1000 random probes
            let sx = rng.next_int(margin as i32, w.saturating_sub(margin) as i32) as u32;
            let sy = rng.next_int(margin as i32, h.saturating_sub(margin) as i32) as u32;
            let r = map.ref_id(sx, sy);
            if map.terrain[r].is_land() {
                if !used.contains(&(sx, sy)) {
                    return (sx, sy, None);
                }
            }
        }
    }

    // 3. Ultimate Fallback (No map/manifest loaded or random probe failed)
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

    for &(sx, sy) in &candidates {
        let sx = sx.clamp(margin, w.saturating_sub(margin));
        let sy = sy.clamp(margin, h.saturating_sub(margin));
        if !used.contains(&(sx, sy)) {
            return (sx, sy, None);
        }
    }

    (w / 2, h / 2, None)
}

fn system_dump_replay(
    time: Res<Time>,
    mut next_dump: Local<f32>,
    q_matches: Query<(&StatePhase, &MatchMetadata, &MatchRoster, &MatchTurnLog)>,
) {
    *next_dump += time.delta_secs();
    if *next_dump >= 10.0 {
        *next_dump -= 10.0;
        
        for (phase, meta, roster, log) in q_matches.iter() {
            if phase.0 == LobbyPhase::Active && !roster.0.is_empty() {
                let seed = meta.seed;
                let players = roster.0.clone();
                let log_clone = log.turns.clone();
                let config_clone = meta.config.clone();

                std::thread::spawn(move || {
                    let record = ReplayFile { config: config_clone, seed, players, turns: log_clone };
                    if let Ok(json) = serde_json::to_string(&record) {
                        let file_name = "latest_replay.json".to_string();
                        std::fs::write(file_name, json).ok();
                    }
                });
            }
        }
    }
}
