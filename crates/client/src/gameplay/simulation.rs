use bevy::prelude::*;
use dark_rift_shared::engine::DarkRiftEngine;
use dark_rift_shared::game::GameState;
use dark_rift_shared::player::Player;
use dark_rift_shared::protocol::*;
use instant::Instant;

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

#[inline]
fn dur_us(d: std::time::Duration) -> u64 {
    d.as_micros().min(u128::from(u64::MAX)) as u64
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

    let t0 = Instant::now();
    for _ in 0..process_count {
        let _ = world.try_run_schedule(SimulationSchedule);
    }
    let wall_us = dur_us(t0.elapsed());

    let pending_after = world.resource::<PendingTicks>().0;
    let queue_len_after = world.resource::<TurnQueue>().0.len();
    if let Some(mut diag) = world.get_resource_mut::<SimPerfDiagnostics>() {
        diag.catchup_schedule_iters = process_count as u32;
        diag.catchup_wall_us = wall_us;
        diag.pending_after_catchup = pending_after;
        diag.queue_len_after_catchup = queue_len_after;
    }
}

/// One simulation step: optionally applies the next queued turn, then consumes one pending tick slot
/// when `PendingTicks > 0`. If `pending == 0` but the queue still has turns, applies turns until the
/// queue is drained (recovers from pending/queue mismatch).
pub fn pop_turn(
    mut engine: ResMut<DarkRiftEngine>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
    mut diag: ResMut<SimPerfDiagnostics>,
) {
    diag.pop_turn_calls = diag.pop_turn_calls.saturating_add(1);
    if pending.0 == 0 && queue.0.is_empty() {
        diag.pop_turn_early_returns = diag.pop_turn_early_returns.saturating_add(1);
        return;
    }

    if let Some(turn) = queue.0.pop_front() {
        diag.turns_applied = diag.turns_applied.saturating_add(1);

        let t = Instant::now();
        engine.apply_intents(&turn.intents);
        diag.us_apply_intents = diag.us_apply_intents.saturating_add(dur_us(t.elapsed()));

        let t = Instant::now();
        engine.execute_income();
        diag.us_income = diag.us_income.saturating_add(dur_us(t.elapsed()));

        let t = Instant::now();
        engine.execute_bot_think(); // Tribes
        diag.us_bot = diag.us_bot.saturating_add(dur_us(t.elapsed()));

        let t = Instant::now();
        engine.execute_nation_think(); // Nations
        diag.us_nation = diag.us_nation.saturating_add(dur_us(t.elapsed()));

        let t = Instant::now();
        engine.execute_fleets();
        diag.us_fleets = diag.us_fleets.saturating_add(dur_us(t.elapsed()));

        let t = Instant::now();
        engine.execute_combat();
        diag.us_combat = diag.us_combat.saturating_add(dur_us(t.elapsed()));

        let t = Instant::now();
        engine.execute_construction();
        diag.us_construction = diag.us_construction.saturating_add(dur_us(t.elapsed()));

        let t = Instant::now();
        engine.execute_tick();
        diag.us_tick = diag.us_tick.saturating_add(dur_us(t.elapsed()));
    }

    if pending.0 > 0 {
        pending.0 -= 1;
    }
}

/// Clears per-frame sim perf counters before [`handle_tile_change`] and other [`Update`] systems run.
pub fn system_sim_perf_reset(mut diag: ResMut<SimPerfDiagnostics>) {
    diag.reset_frame();
}

/// Writes compact diagnostics into [`OverlayDiagnostics::sim_perf_line`] for HUD / correlation with FPS.
pub fn system_sync_sim_perf_overlay(
    diag: Res<SimPerfDiagnostics>,
    engine: Option<Res<DarkRiftEngine>>,
    mut overlay: ResMut<OverlayDiagnostics>,
) {
    let (atk, fl, bd) = engine
        .as_ref()
        .map(|e| (e.attacks.len(), e.fleets.len(), e.buildings.len()))
        .unwrap_or((0, 0, 0));
    let sim_phase_us = diag
        .us_apply_intents
        .saturating_add(diag.us_income)
        .saturating_add(diag.us_bot)
        .saturating_add(diag.us_nation)
        .saturating_add(diag.us_fleets)
        .saturating_add(diag.us_combat)
        .saturating_add(diag.us_construction)
        .saturating_add(diag.us_tick);
    let line = format!(
        "sim:{}µs cu:{}@{}µs ta:{} idle:{} pend:{} q:{} tc:{} ev:{} atk:{} fl:{} bd:{}",
        sim_phase_us,
        diag.catchup_schedule_iters,
        diag.catchup_wall_us,
        diag.turns_applied,
        diag.pop_turn_early_returns,
        diag.pending_after_catchup,
        diag.queue_len_after_catchup,
        diag.tile_captured_writes,
        diag.tile_handler_events_scanned,
        atk,
        fl,
        bd,
    );
    if overlay.sim_perf_line != line {
        overlay.sim_perf_line = line;
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
