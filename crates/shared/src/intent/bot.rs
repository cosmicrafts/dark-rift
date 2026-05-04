use crate::engine::DarkRiftEngine;
use crate::protocol::{GameplayIntent, StampedIntent, AttackIntent};
use crate::rng::{NextIntExt, WyRand, SeedableRng};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum BotDecisionKind {
    Attack = 1,
}

#[derive(Clone, Debug)]
struct BotDecision {
    bot_id: u16,
    kind: BotDecisionKind,
    intent: GameplayIntent,
}

impl DarkRiftEngine {
pub fn execute_bot_think(&mut self) {
    if self.state.phase != crate::game::GamePhase::Playing {
        return;
    }

    let tick = self.state.tick;
    // Tribes are generally passive, they just expand and occasionally attack
    let attack_interval_base = 250;
    let trigger_ratio = 0.6;
    let reserve_ratio = 0.3;
    let expand_ratio = 0.15;
    let random_attack_div = 5;

    let mut schedule: Vec<(u16, u64, u64)> = Vec::new();
    for p in self.state.players.iter() {
        if p.player_type != crate::player::PlayerType::Bot || !p.alive {
            continue;
        }
        let bot_id = p.id;
        let mut sched_rng = WyRand::seed_from_u64(
            self.state.seed.wrapping_add(bot_id as u64).wrapping_add(attack_interval_base as u64)
        );
        let interval = sched_rng
            .next_int(attack_interval_base as i32, (attack_interval_base as i32 * 2).max(1))
            .max(1) as u64;
        let offset = sched_rng.next_int(0, interval as i32) as u64;
        schedule.push((bot_id, interval, offset));
    }
    schedule.sort_unstable_by_key(|(bot_id, _, _)| *bot_id);
    if schedule.is_empty() {
        return;
    }

    let mut decisions: Vec<BotDecision> = Vec::new();

    for (bot_id, interval, offset) in schedule {
        let phase = tick % interval;
        let do_attack = phase == offset;

        if !do_attack {
            continue;
        }
        let (troops, max_troops, border_tiles) = {
            let Some(player) = self.state.player(bot_id) else {
                continue;
            };
            (player.troops, player.max_troops, player.border_coords(self.state.map.width).collect::<Vec<_>>())
        };
        if border_tiles.is_empty() {
            continue;
        }
        let mut has_neutral = false;
        let mut player_targets = Vec::new();
        let max_scan = border_tiles.len().min(256);
        let start_idx = {
             let p_mut = self.state.player_mut(bot_id).unwrap();
             p_mut.bot_rng.next_int(0, border_tiles.len() as i32) as usize
        };
        for i in 0..max_scan {
            let b_idx = (start_idx + i) % border_tiles.len();
            let (bx, by) = border_tiles[b_idx];
            self.state.map.for_each_neighbor(bx, by, |nx, ny| {
                let owner = self.state.map.owner_id(nx, ny);
                if owner != bot_id {
                    if owner == 0 {
                        has_neutral = true;
                    } else {
                        player_targets.push(owner);
                    }
                }
            });
        }
        player_targets.sort_unstable();
        player_targets.dedup();
        let (target_owner, is_neutral) = if has_neutral {
            (0, true)
        } else if player_targets.is_empty() {
            continue;
        } else {
            let p_mut = self.state.player_mut(bot_id).unwrap();
            let t = if p_mut.bot_rng.next_int(0, random_attack_div) == 0 {
                player_targets[p_mut.bot_rng.next_int(0, player_targets.len() as i32) as usize]
            } else {
                player_targets[0]
            };
            (t, false)
        };
        if !is_neutral && troops < max_troops * trigger_ratio {
            continue;
        }
        let reserve = max_troops * if is_neutral { expand_ratio } else { reserve_ratio };
        let p_send = (troops - reserve).max(0.0);
        if p_send < self.state.config.attack_cost_neutral {
            continue;
        }
        decisions.push(BotDecision {
            bot_id,
            kind: BotDecisionKind::Attack,
            intent: GameplayIntent::Attack(AttackIntent {
                target_owner,
                troops: Some(p_send),
            }),
        });
    }

    decisions.sort_by_key(|d| (d.bot_id, d.kind));
    for (intent_index, d) in decisions.into_iter().enumerate() {
        let stamped = StampedIntent {
            player_id: d.bot_id,
            intent: d.intent,
        };
        self.apply_stamped_intent(
            &stamped,
            intent_index as u32,
        );
    }
}
}
