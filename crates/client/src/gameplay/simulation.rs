use bevy::prelude::*;
use dark_rift_shared::engine::DarkRiftEngine;
use dark_rift_shared::game::GameState;
use dark_rift_shared::player::Player;
use dark_rift_shared::protocol::*;

use crate::*;

pub fn start_game_core(
    msg:           ServerStartMessage,
    cam_q:         &mut Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    commands:      &mut Commands,
    params:        &mut StartGameParams,
) {
            if params.ready.0 { return; }

            let hud_s = clamp_ui_scale(params.settings.config.ui.combat_hud_scale);

            if let Some(pid) = msg.my_player_id {
                debug!("Game starting (player {})", pid);
                params.my_id.0 = pid;
            } else {
                debug!("Game starting (observer)");
                params.my_id.0 = 0;
            }

            let mut game = GameState::new(msg.seed, msg.config.map_width, msg.config.map_height, msg.config.clone());
            map_fetch::apply_map_to_game(&mut game, &msg.config.map_name, msg.map_data.clone());

            for info in &msg.players {
                let player = match info.player_type {
                    dark_rift_shared::player::PlayerType::Human => {
                        Player::new_human(info.id, info.name.clone(), info.color, &msg.config)
                    }
                    dark_rift_shared::player::PlayerType::Bot => {
                        Player::new_bot(info.id, info.name.clone(), info.color, &msg.config)
                    }
                    dark_rift_shared::player::PlayerType::Nation => {
                        Player::new_nation(info.id, info.name.clone(), info.color, &msg.config)
                    }
                };
                game.spawn_player(player, info.spawn_x, info.spawn_y);
            }

            // Replay missed turns (late-join) — identical behavior to OpenFront
            for turn in &msg.missed_turns {
                params.queue.0.push_back(turn.clone());
                params.pending.0 += 1;
                params.highest.0 = Some(params.highest.0.unwrap_or(0).max(turn.turn_number));
            }

            commands.insert_resource(msg.config.clone());
            
            
            let map_w = msg.config.map_width as f32;
            let map_h = msg.config.map_height as f32;
            let mut cx = map_w * TILE_SIZE / 2.0;
            let mut cy = map_h * TILE_SIZE / 2.0;

            if params.my_id.0 != 0 {
                if let Some(me) = msg.players.iter().find(|p| p.id == params.my_id.0) {
                    cx = me.spawn_x as f32 * TILE_SIZE;
                    cy = (game.map.height as f32 - 1.0 - me.spawn_y as f32) * TILE_SIZE;
                }
            }

            if let Ok((mut transform, mut projection)) = cam_q.single_mut() {
                reset_game_camera(&mut transform, &mut projection, Vec2::new(cx, cy));
            }
            if let Some(images_res) = params.images.as_mut() {
                let grid = spawn_shader_map(
                    commands,
                    &game,
                    images_res,
                    params.materials.as_mut().unwrap(),
                    params.meshes.as_mut().unwrap(),
                    true,
                    &params.settings.config.visuals,
                );
                commands.insert_resource(grid);
            }

            let water = WaterComponents::compute(&game.map);
            commands.insert_resource(water.clone());
            let engine = DarkRiftEngine::new(game, water);
            commands.insert_resource(engine);
            params.ready.0 = true;
            params.next_state.set(AppState::InGame);

            commands.insert_resource(CombatDockUiScale(hud_s));
            if let Some(s) = &params.shell {
                spawn_bottom_combat_hud(
                    commands,
                    s,
                    msg.my_player_id.is_none(),
                    hud_s,
                    params.asset_server.as_ref(),
                    params.settings.font.clone(),
                );
            }

            if let Some(over) = params.overlay.as_mut() {
                if msg.my_player_id.is_some() {
                    over.status_line = format!("Connected — Player {:?}", msg.my_player_id);
                } else {
                    over.status_line = "Connected — Observer".to_string();
                }
            }
}

pub fn catchup_fast_forward(world: &mut World) {
    let ready = world.resource::<GameReady>().0;
    if !ready {
        return;
    }

    let queue_len = world.resource::<TurnQueue>().0.len();
    let pending = world.resource::<PendingTicks>().0;
    if pending == 0 && queue_len == 0 {
        return;
    }

    // Process up to `max(pending, queue)` ticks: fixes stuck queues when `pending` was decremented
    // without a matching `pop_turn` (e.g. schedule chain ran `decrement_pending` while `pop_turn`
    // did not run), and drains backlog when `pending` under-counted turns.
    let work = (pending as usize).max(queue_len);
    let process_count = work
        .min(CLIENT_CATCHUP_MAX_TICKS_PER_FRAME)
        .max(1);

    // Fixed iteration count avoids repeated `World::resource` lookups per tick; `pop_turn` no-ops
    // cheaply once queue and pending are drained.
    for _ in 0..process_count {
        let _ = world.try_run_schedule(SimulationSchedule);
    }
}

/// One simulation step: optionally applies the next queued turn, then consumes one pending tick slot
/// when `PendingTicks > 0`. If `pending == 0` but the queue still has turns, applies turns until the
/// queue is drained (recovers from pending/queue mismatch).
pub fn pop_turn(
    mut engine: ResMut<DarkRiftEngine>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
) {
    if pending.0 == 0 && queue.0.is_empty() {
        return;
    }

    if let Some(turn) = queue.0.pop_front() {
        engine.apply_intents(&turn.intents);
        engine.execute_income();
        engine.execute_bot_think(); // Tribes
        engine.execute_nation_think(); // Nations
        engine.execute_fleets();
        engine.execute_combat();
        engine.execute_construction();
        engine.execute_tick();
    }

    if pending.0 > 0 {
        pending.0 -= 1;
    }
}

pub fn system_single_player_tick(
    time: Res<Time>,
    sp_state: Option<ResMut<SinglePlayerState>>,
    buffer: Option<ResMut<LocalIntentBuffer>>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
    mut highest: ResMut<HighestReceivedTurn>,
) {
    if let (Some(mut st), Some(mut buf)) = (sp_state, buffer) {
        st.timer.tick(time.delta());
        if st.timer.just_finished() {
            let turn = dark_rift_shared::protocol::Turn {
                turn_number: st.current_turn,
                intents: std::mem::take(&mut buf.0),
            };
            st.current_turn += 1;
            
            queue.0.push_back(turn);
            pending.0 += 1;
            highest.0 = Some(highest.0.unwrap_or(0).max(st.current_turn));
        }
    }
}