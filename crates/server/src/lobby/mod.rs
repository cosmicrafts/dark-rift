use bevy::prelude::*;
use lightyear::prelude::*;
use dark_rift_shared::config;
use dark_rift_shared::protocol::*;
use dark_rift_shared::rng::SeedableRng;
use crate::core::*;

/// Implements OpenFront's "Always Active" Lobby Pattern
pub fn system_master_lobby(
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
        
        #[derive(serde::Deserialize)]
        struct ServerConfigWrapper {
            #[serde(default)]
            gameplay: dark_rift_shared::game_config::GameConfig,
        }
        
        let mut config = dark_rift_shared::game_config::GameConfig::default();
        if let Ok(s) = std::fs::read_to_string("assets/settings.ron") {
            if let Ok(wrapper) = ron::from_str::<ServerConfigWrapper>(&s) {
                config = wrapper.gameplay;
                info!("⚙️  Loaded gameplay modifiers from settings.ron");
            } else {
                warn!("⚠️ Failed to parse settings.ron gameplay section, using defaults");
            }
        } else {
            warn!("⚠️ Could not read assets/settings.ron, using defaults");
        }
        
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
            MatchMetadata { seed: 42 + lobby_id, config: config.clone(), map_cache, manifest_cache },
            MatchRoster::default(),
            MatchTurnLog::default(),
            MatchIntentBuffer::default(),
            HasSentStart(false),
            MatchTimer(Timer::from_seconds(config.tick_rate_ms / 1000.0, TimerMode::Repeating)),
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
                            
                            let mut rng = dark_rift_shared::rng::WyRand::seed_from_u64(meta.seed.wrapping_add(player_id as u64));
                            use dark_rift_shared::rng::NextIntExt;
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

pub fn system_lobby_gc(
    mut commands: Commands,
    time: Res<Time>,
    mut q_lobbies: Query<(Entity, &Lobby, &mut StatePhase, &MatchRoster, Option<&mut ActiveEmptyTimer>)>,
) {
    for (entity, lobby, mut phase, roster, timer_opt) in q_lobbies.iter_mut() {
        if phase.0 == LobbyPhase::Active {
            let human_count = roster.0.iter().filter(|p| p.player_type == dark_rift_shared::player::PlayerType::Human).count();
            if human_count == 0 {
                if let Some(mut timer) = timer_opt {
                    timer.0.tick(time.delta());
                    if timer.0.just_finished() {
                        info!("🗑️  [GC] Lobby {} estuvo vacío de humanos por 30s. Terminando...", lobby.id);
                        phase.0 = LobbyPhase::Finished;
                        commands.entity(entity).despawn();
                    }
                } else {
                    info!("🗑️  [GC] Todos los humanos abandonaron el Lobby {}. Terminando...", lobby.id);
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

pub fn system_broadcast_lobbies(
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

pub fn system_start_matches(
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

pub fn system_end_turn(
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

pub fn system_dump_replay(
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

pub fn pick_spawn(roster: &MatchRoster, player_id: u16, metadata: &MatchMetadata) -> (u32, u32, Option<String>) {
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
        let mut rng = dark_rift_shared::rng::WyRand::seed_from_u64(metadata.seed + player_id as u64);
        use dark_rift_shared::rng::NextIntExt;
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
