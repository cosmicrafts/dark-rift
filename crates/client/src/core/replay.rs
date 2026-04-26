use bevy::prelude::*;
use dark_rift_shared::protocol::ReplayFile;
use crate::{PendingTicks, TurnQueue};

pub struct ReplayDirector {
    pub record: ReplayFile,
    pub turn_index: usize,
}

pub fn system_replay_pulse(
    mut timer: Local<f32>,
    time:      Res<Time>,
    mut director: NonSendMut<ReplayDirector>,
    mut queue: ResMut<TurnQueue>,
    mut pending: ResMut<PendingTicks>,
) {
    *timer += time.delta_secs();
    let tick_len = 0.040; // 40ms turn interval
    
    while *timer >= tick_len {
        *timer -= tick_len;
        if director.turn_index < director.record.turns.len() {
            queue.0.push_back(director.record.turns[director.turn_index].clone());
            pending.0 += 1;
            director.turn_index += 1;
        }
    }
}
