use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use lightyear::netcode::NetcodeServer;
use lightyear::websocket::server::{WebSocketServerIo, ServerConfig as WebSocketConfig};
use std::net::{Ipv4Addr, SocketAddr};

use dark_rift_shared::protocol::*;
use dark_rift_shared::player::player_colors;
use crate::core::*;
use crate::lobby::pick_spawn;

pub fn setup(mut commands: Commands) {
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

pub fn handle_client_joins(
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

pub fn handle_client_leave(
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

pub fn handle_intent_messages(
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
