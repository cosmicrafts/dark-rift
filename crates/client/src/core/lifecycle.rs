use bevy::prelude::*;
use dark_rift_shared::engine::DarkRiftEngine;
use dark_rift_shared::protocol::*;
use crate::ClientSettings;
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
            
            if net.handshake_ms.is_none()
                && let Some(t0) = net.connect_started.take() {
                    net.handshake_ms = Some(t0.elapsed().as_secs_f32() * 1000.0);
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
            if let Some(h) = highest.0
                && msg.turn.turn_number <= h {
                    continue; // Drop overlapping buffered turns already processed via missed_turns!
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
    engine_opt: Option<Res<DarkRiftEngine>>,
    ready: Res<GameReady>,
    my_id: Res<MyPlayerId>,
    settings: Option<Res<ClientSettings>>,
    panel_q: Query<Entity, With<GameOverPanel>>,
) {
    let Some(engine) = engine_opt else { return; };
    if !ready.0 { return; }
    if panel_q.iter().next().is_some() { return; }

    let alive_count = engine.state.players.iter().filter(|p| p.alive).count();
    let am_i_alive = engine.state.player(my_id.0).map(|p| p.alive).unwrap_or(false);

    let mut show_panel = false;
    let mut victory = false;

    if alive_count <= 1 {
        show_panel = true;
        victory = am_i_alive;
    } else if !am_i_alive && my_id.0 != 0 {
        show_panel = true;
        victory = false;
    }

    if show_panel
        && let Some(settings) = settings {
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
                    sender.send::<dark_rift_shared::protocol::ReliableChannel>(ClientLeaveMessage);
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

/// Clears FPS spike diagnostics line when leaving a match (overlay resource persists in lobby).
pub fn system_clear_sim_perf_line(mut overlay: ResMut<OverlayDiagnostics>) {
    overlay.sim_perf_line.clear();
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
    mut drag_state: ResMut<CameraDragState>,
    mut pinch_state: ResMut<PinchState>,
    mut touch_gesture: ResMut<TouchGestureState>,
    mut label_pool: ResMut<crate::EntityPool>,
    mut fleet_pool: ResMut<crate::FleetVisualPool>,
    mut building_pool: ResMut<crate::BuildingVisualPool>,
    mut attack_ring_pool: ResMut<crate::AttackRingPool>,
    mut cam_q: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    q_ingame: Query<Entity, With<InGameEntity>>,
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

    *drag_state = CameraDragState::default();
    *pinch_state = PinchState::default();
    *touch_gesture = TouchGestureState::default();

    for (mut transform, mut projection) in cam_q.iter_mut() {
        reset_lobby_camera(&mut transform, &mut projection);
    }

    // `try_despawn`: hierarchy may already have removed children when a parent was hit first — avoids Bevy 0.18 warnings.
    for e in q_ingame.iter() {
        commands.entity(e).try_despawn();
    }

    // Visual entity pools hold `Entity` ids; `Local` pools survived match exit and caused
    // invalid reuse (WASM panic) on the next `InGame` session.
    *label_pool = crate::EntityPool::default();
    *fleet_pool = crate::FleetVisualPool::default();
    *building_pool = crate::BuildingVisualPool::default();
    *attack_ring_pool = crate::AttackRingPool::default();

    commands.remove_resource::<DarkRiftEngine>();
    commands.remove_resource::<SinglePlayerState>();
    commands.remove_resource::<LocalIntentBuffer>();
    commands.remove_resource::<crate::ShaderMapData>();
    commands.remove_resource::<dark_rift_shared::water_components::WaterComponents>();
    // Keep `CombatDockUiScale` present: shell/layout systems use `resource_changed::<CombatDockUiScale>` and must not panic when resource is missing after exit.
    commands.insert_resource(CombatDockUiScale::default());
    commands.remove_resource::<dark_rift_shared::game_config::GameConfig>();
}