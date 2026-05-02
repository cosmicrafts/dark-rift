use bevy::prelude::*;
use dark_rift_shared::game_config::{BotProfile, GameConfig};
use dark_rift_shared::engine::DarkRiftEngine;
use dark_rift_shared::protocol::{ClientGameplayMessage, GameplayIntent, AttackIntent};
use lightyear::prelude::MessageSender;
use dark_rift_shared::protocol::ReliableChannel;
use crate::MyPlayerId;
use dark_rift_shared::rng::{WyRand, NextIntExt, SeedableRng};

#[derive(Resource)]
pub struct HeadlessBotState {
    pub profile: BotProfile,
    pub timer: Timer,
    pub has_joined_lobby: bool,
}

/// Headless client logic for the bot.
pub fn system_headless_bot_engine(
    mut bot_state: ResMut<HeadlessBotState>,
    engine: Res<DarkRiftEngine>,
    mut sender: Query<&mut MessageSender<ClientGameplayMessage>>,
    my_id: Res<MyPlayerId>,
) {
    if !bot_state.timer.tick(std::time::Duration::from_secs_f64(1.0 / 60.0)).just_finished() {
        return;
    }

    let Some(mut sender_mut) = sender.iter_mut().next() else {
        return;
    };

    let Some(_player) = engine.state.player(my_id.0) else {
        return;
    };

    let bot_id = my_id.0;
    
    // Exact same logic ported from `intent.rs` to generate an attack via ClientGameplayMessage.
    let trigger_ratio = bot_state.profile.trigger_ratio;
    let reserve_ratio = bot_state.profile.reserve_ratio;
    let expand_ratio = bot_state.profile.expand_ratio;
    
    let player = engine.state.player(bot_id).unwrap();
    let border_tiles: Vec<_> = player.border_coords(engine.state.map.width).collect();
    if border_tiles.is_empty() { return; }

    let mut has_neutral = false;
    let mut player_targets = Vec::new();
    let max_scan = border_tiles.len().min(256);
    let mut bot_rng = WyRand::seed_from_u64(engine.state.seed.wrapping_add(engine.state.tick as u64).wrapping_add(1999));
    let start_idx = bot_rng.next_int(0, border_tiles.len() as i32) as usize;

    for i in 0..max_scan {
        let b_idx = (start_idx + i) % border_tiles.len();
        let (bx, by) = border_tiles[b_idx];
        engine.state.map.for_each_neighbor(bx, by, |nx, ny| {
            let owner = engine.state.map.owner_id(nx, ny);
            if owner != bot_id {
                if owner == 0 {
                    has_neutral = true;
                } else {
                    player_targets.push(owner);
                }
            }
        });
    }

    let (target_owner, is_neutral) = if has_neutral {
        (0, true)
    } else {
        if player_targets.is_empty() {
            return;
        }
        (player_targets[bot_rng.next_int(0, player_targets.len() as i32) as usize], false)
    };

    if !is_neutral {
        let trigger_limit = player.max_troops * trigger_ratio;
        if player.troops < trigger_limit {
            return;
        }
    }

    let active_reserve_ratio = if is_neutral { expand_ratio } else { reserve_ratio };
    let reserve = player.max_troops * active_reserve_ratio;
    let p_send = (player.troops - reserve).max(0.0);
    
    if p_send < GameConfig::default().attack_cost_neutral as f64 {
        return;
    }

    debug!("Headless bot attack -> player {} (troops: {})", target_owner, p_send);
    let _ = sender_mut.send::<ReliableChannel>(ClientGameplayMessage {
        intent: GameplayIntent::Attack(AttackIntent {
            target_owner,
            troops: Some(p_send),
        }),
    });
}
