use bevy::prelude::*;
use dark_rift_shared::game::GameState;
use dark_rift_shared::protocol::*;

use crate::*;
use std::collections::VecDeque;

#[derive(Resource)]
pub struct DesyncTracker {
    pub log: VecDeque<TurnLogEntry>,
}

impl Default for DesyncTracker {
    fn default() -> Self {
        Self {
            log: VecDeque::with_capacity(512),
        }
    }
}

pub struct TurnLogEntry {
    pub tick: u64,
    pub checksum: u64,
    pub intents: Vec<dark_rift_shared::protocol::StampedIntent>,
}

pub fn start_game_core(
    msg:           ServerStartMessage,
    cam_q:         &mut Query<&mut Transform, With<Camera2d>>,
    commands:      &mut Commands,
    params:        &mut StartGameParams,
) {
            if params.ready.0 { return; }

            let hud_s = clamp_ui_scale(params.settings.config.ui.combat_hud_scale);

            if let Some(pid) = msg.my_player_id {
                info!("🎮 Game starting! I am player {}", pid);
                params.my_id.0 = pid;
            } else {
                info!("👁  Game starting! I am an observer");
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

            if let Ok(mut cam) = cam_q.single_mut() {
                cam.translation = Vec3::new(cx, cy, 999.0);
            }
            if let Some(images_res) = params.images.as_mut() {
                let grid = spawn_shader_map(
                    commands,
                    &game,
                    images_res,
                    params.materials.as_mut().unwrap(),
                    params.meshes.as_mut().unwrap(),
                    true,
                    &params.settings.config.visuals.border,
                );
                commands.insert_resource(grid);
            }

            commands.insert_resource(WaterComponents::compute(&game.map));
            commands.insert_resource(game);
            params.ready.0 = true;
            params.next_state.set(AppState::InGame);
            commands.insert_resource(DesyncTracker::default());

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
    if !ready || !world.contains_resource::<GameState>() { return; }
    
    let pending = world.resource::<PendingTicks>().0;
    if pending == 0 { return; }
    
    let process_count = pending
        .min(CLIENT_CATCHUP_MAX_TICKS_PER_FRAME as u32)
        .max(1) as usize;
    
    for _ in 0..process_count {
        if world.resource::<PendingTicks>().0 == 0 { break; }
        let _ = world.try_run_schedule(SimulationSchedule);
    }
}

pub fn pop_turn(
    mut commands: Commands,
    mut game: ResMut<GameState>,
    mut queue: ResMut<TurnQueue>,
    mut q_exec: Query<(Entity, &mut AttackExecution)>,
    mut q_fleet: Query<(Entity, &mut WarpFleet)>,
    mut path_scratch: ResMut<WaterPathfinderScratch>,
    water_components: Res<WaterComponents>,
    mut desync_tracker: ResMut<DesyncTracker>,
) {
    if let Some(turn) = queue.0.pop_front() {
        for (i, stamped) in turn.intents.iter().enumerate() {
            apply_stamped_intent(
                &mut commands,
                &mut *game,
                &mut q_exec,
                &mut q_fleet,
                stamped,
                i as u32,
                &mut *path_scratch,
                &water_components,
            );
        }

        let checksum = dark_rift_shared::checksum::compute_state_hash(
            &game,
            game.buildings.iter(),
            q_fleet.iter().map(|(_, f)| f as &WarpFleet),
            q_exec.iter().map(|(_, e)| e as &AttackExecution),
        );
        
        desync_tracker.log.push_back(TurnLogEntry {
            tick: turn.turn_number,
            checksum,
            intents: turn.intents,
        });

        if desync_tracker.log.len() > 500 {
            desync_tracker.log.pop_front();
        }
    }
}

pub fn decrement_pending(mut pending: ResMut<PendingTicks>) {
    pending.0 = pending.0.saturating_sub(1);
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
