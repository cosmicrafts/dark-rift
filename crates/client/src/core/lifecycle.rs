use bevy::prelude::*;
use dark_rift_shared::game::GameState;
use dark_rift_shared::protocol::*;
use lightyear::prelude::*;
use instant::Instant;
use crate::*;

pub fn handle_server_start(
    mut receivers: Query<&mut MessageReceiver<ServerStartMessage>>,
    mut commands:  Commands,
    mut net:       ResMut<NetTiming>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            commands.insert_resource(PendingServerStart(msg.clone()));
            next_state.set(AppState::LoadingMap);
            
            if net.handshake_ms.is_none() {
                if let Some(t0) = net.connect_started.take() {
                    net.handshake_ms = Some(t0.elapsed().as_secs_f32() * 1000.0);
                }
            }
        }
    }
}

pub fn handle_server_turns(
    mut receivers: Query<&mut MessageReceiver<ServerTurnMessage>>,
    ready:         Res<GameReady>,
    mut net:       ResMut<NetTiming>,
    mut queue:     ResMut<TurnQueue>,
    mut pending:   ResMut<PendingTicks>,
    mut highest:   ResMut<HighestReceivedTurn>,
) {
    if !ready.0 { return; }

    for mut receiver in receivers.iter_mut() {
        for msg in receiver.receive() {
            if let Some(h) = highest.0 {
                if msg.turn.turn_number <= h {
                    continue; // Drop overlapping buffered turns already processed via missed_turns!
                }
            }
            highest.0 = Some(msg.turn.turn_number);

            let now = Instant::now();
            if let Some(prev) = net.last_turn_at {
                let dt_ms = now.duration_since(prev).as_secs_f32() * 1000.0;
                if net.turn_spacing_ema_ms <= 0.0 {
                    net.turn_spacing_ema_ms = dt_ms;
                } else {
                    net.turn_spacing_ema_ms = net.turn_spacing_ema_ms * 0.92 + dt_ms * 0.08;
                }
            }
            net.last_turn_at = Some(now);

            queue.0.push_back(msg.turn);
            pending.0 += 1;
        }
    }
}

pub fn system_evaluate_game_over(
    mut commands: Commands,
    game: Option<Res<GameState>>,
    ready: Res<GameReady>,
    my_id: Res<MyPlayerId>,
    settings: Option<Res<ClientSettings>>,
    panel_q: Query<Entity, With<GameOverPanel>>,
) {
    let Some(game) = game else { return; };
    if !ready.0 { return; }
    if panel_q.iter().next().is_some() { return; }

    let alive_count = game.players.iter().filter(|p| p.alive).count();
    let am_i_alive = game.player(my_id.0).map(|p| p.alive).unwrap_or(false);

    let mut show_panel = false;
    let mut victory = false;

    if alive_count <= 1 {
        show_panel = true;
        victory = am_i_alive;
    } else if !am_i_alive && my_id.0 != 0 {
        show_panel = true;
        victory = false;
    }

    if show_panel {
        if let Some(settings) = settings {
            let font = settings.font.clone();
        let title = if victory { "VICTORY" } else { "DEFEAT" };
        let title_color = if victory { theme::accent_primary() } else { theme::accent_danger() };

        commands.spawn((
            GameOverPanel,
            InGameEntity,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(GAME_OVER),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
        )).with_children(|p| {
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(40.0)),
                    border_radius: BorderRadius::all(Val::Px(16.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(theme::panel_bg()),
                BorderColor::all(theme::panel_border()),
            )).with_children(|modal| {
                modal.spawn((
                    Text::new(title),
                    TextFont { font: font.clone(), font_size: 64.0, ..default() },
                    TextColor(title_color),
                    Node { margin: UiRect::bottom(Val::Px(30.0)), ..default() }
                ));

                modal.spawn((
                    Button,
                    GoBackButton,
                    Node {
                        padding: UiRect::axes(Val::Px(40.0), Val::Px(20.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(theme::accent_home()),
                )).with_children(|btn| {
                    btn.spawn((
                        Text::new("GO BACK TO LOBBY"),
                        TextFont { font: font.clone(), font_size: 24.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                });
            });
        });
        }
    }
}

pub fn system_handle_go_back(
    mut interaction_q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<GoBackButton>)>,
    mut next_state: ResMut<NextState<AppState>>,
    mut senders: Query<&mut MessageSender<ClientLeaveMessage>>,
    mut joined_lobby: Option<ResMut<JoinedLobby>>,
) {
    for (interaction, mut bg) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                if let Some(jl) = joined_lobby.as_mut() {
                    jl.0 = None;
                }
                for mut sender in senders.iter_mut() {
                    let _ = sender.send::<dark_rift_shared::protocol::ReliableChannel>(ClientLeaveMessage);
                }
                next_state.set(AppState::LobbyBrowser);
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(theme::accent_home_hover());
            }
            Interaction::None => {
                *bg = BackgroundColor(theme::accent_home());
            }
        }
    }
}

pub fn system_cleanup_ingame_entities(
    mut commands: Commands,
    mut ready: ResMut<GameReady>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
    mut highest: ResMut<HighestReceivedTurn>,
    mut outgoing_map: ResMut<OutgoingAttackRowMap>,
    mut outgoing_fleet_map: ResMut<OutgoingFleetRowMap>,
    mut fresh_tiles: ResMut<FreshlyPaintedTiles>,
    mut build_mode: ResMut<BuildStructureMode>,
    mut ghost_parts: ResMut<StructureGhostParts>,
    mut build_hud: ResMut<BuildableHudState>,
    mut build_hud_dirty: ResMut<crate::ui::hud_build::BuildableHudDirty>,
    q_ingame: Query<Entity, With<InGameEntity>>,
    q_execs: Query<Entity, With<AttackExecution>>,
    q_buildings: Query<Entity, With<Building>>,
    q_fleets: Query<Entity, With<dark_rift_shared::warp_fleet::WarpFleet>>,
) {
    ready.0 = false;
    queue.0.clear();
    pending.0 = 0;
    highest.0 = None;
    outgoing_map.0.clear();
    outgoing_fleet_map.0.clear();
    fresh_tiles.tiles.clear();
    build_mode.0 = None;
    *ghost_parts = StructureGhostParts::default();
    *build_hud = BuildableHudState::default();
    *build_hud_dirty = crate::ui::hud_build::BuildableHudDirty::default();

    for e in q_ingame.iter() {
        commands.entity(e).despawn();
    }
    for e in q_execs.iter() {
        commands.entity(e).despawn();
    }
    for e in q_buildings.iter() {
        commands.entity(e).despawn();
    }
    for e in q_fleets.iter() {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<GameState>();
    commands.remove_resource::<SinglePlayerState>();
    commands.remove_resource::<LocalIntentBuffer>();
}

/// Cleanup rail rendering resources — runs alongside entity cleanup on OnExit(InGame).
/// Prevents cross-game memory accumulation of mesh handles, pathfinding buffers,
/// tile caches, and train entity pools.
pub fn system_cleanup_rail_resources(
    mut rail_cache: ResMut<crate::rendering::rail::RailRenderCache>,
    mut rail_dirty: ResMut<crate::rendering::rail::RailColorDirty>,
    mut rail_pool: ResMut<crate::rendering::rail::RailTrainPool>,
    mut rail_scratch: ResMut<dark_rift_shared::execution::rail::RailPathScratchResource>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // Remove the stale mesh asset so GPU memory is freed.
    if let Some(h) = rail_cache.mesh_handle.take() {
        meshes.remove(&h);
    }
    rail_cache.mesh_entity = None;
    rail_cache.topo_hash = (0, 0);
    rail_cache.tile_ranges.clear();
    rail_cache.tile_ranges.shrink_to_fit();
    rail_cache.tiles.clear();
    rail_cache.tiles.shrink_to_fit();
    rail_cache.tile_owner_cache.clear();
    rail_cache.tile_owner_cache.shrink_to_fit();
    rail_cache.next_check = 0;

    rail_dirty.tiles.clear();
    rail_dirty.tiles.shrink_to_fit();

    // Pool entity references are stale (entities are despawned by system_cleanup_ingame_entities).
    rail_pool.by_train_id.clear();
    rail_pool.by_train_id.shrink_to_fit();

    // Reset pathfinding scratch — free the ~95MB backing buffers.
    *rail_scratch = dark_rift_shared::execution::rail::RailPathScratchResource::default();
}