use crate::engine::DarkRiftEngine;
use crate::game::BuildingKind;
use crate::protocol::{GameplayIntent, StampedIntent, AttackIntent};
use crate::building::{Building, existing_structure_positions_with_width, structure_kind_enabled, structure_build_cost_gold, resolve_structure_spawn_tile};
use crate::rng::{NextIntExt, WyRand, SeedableRng};
fn bot_structure_target_count(
    kind: BuildingKind,
    city_equivalent: u32,
    difficulty: crate::game_config::BotDifficulty,
) -> u32 {
    let sam_ratio = match difficulty {
        crate::game_config::BotDifficulty::BrainDead => 0.15,
        crate::game_config::BotDifficulty::Vanilla => 0.20,
        crate::game_config::BotDifficulty::Terminator => 0.30,
    };
    match kind {
        BuildingKind::DefensePost => ((city_equivalent as f64) * 0.25).floor() as u32,
        BuildingKind::Port => ((city_equivalent as f64) * 0.75).floor() as u32,
        BuildingKind::Factory => ((city_equivalent as f64) * 0.75).floor() as u32,
        BuildingKind::SamLauncher => ((city_equivalent as f64) * sam_ratio).floor() as u32,
        BuildingKind::MissileSilo => ((city_equivalent as f64) * 0.2).floor() as u32,
        BuildingKind::City => city_equivalent.saturating_add(1),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum BotDecisionKind {
    Build = 0,
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

    let config = self.state.config.clone();
    let tick = self.state.tick;
    let (attack_interval_base, trigger_ratio, reserve_ratio, expand_ratio, random_attack_div) =
        match config.bot_difficulty {
            crate::game_config::BotDifficulty::BrainDead => (500, 0.9, 0.8, 0.12, 8),
            crate::game_config::BotDifficulty::Vanilla => (250, 0.6, 0.3, 0.15, 5),
            crate::game_config::BotDifficulty::Terminator => (125, 0.4, 0.1, 0.18, 3),
        };

    let mut schedule: Vec<(u16, u64, u64)> = Vec::new();
    let mut any_structures = false;
    for p in self.state.players.iter() {
        if p.is_human() || !p.alive {
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
        
        let phase = tick % interval;
        let one_third = (offset + interval / 3) % interval;
        let two_thirds = (offset + (interval * 2) / 3) % interval;
        let do_attack = phase == offset;
        let do_structures = do_attack || phase == one_third || phase == two_thirds;
        if do_structures {
            any_structures = true;
        }
    }
    schedule.sort_unstable_by_key(|(bot_id, _, _)| *bot_id);
    if schedule.is_empty() {
        return;
    }

    let mut structure_positions = Vec::new();
    if any_structures {
        structure_positions =
            existing_structure_positions_with_width(self.buildings.iter(), self.state.map.width, None);
    }
    let mut decisions: Vec<BotDecision> = Vec::new();

    for (bot_id, interval, offset) in schedule {
        let phase = tick % interval;
        let one_third = (offset + interval / 3) % interval;
        let two_thirds = (offset + (interval * 2) / 3) % interval;
        let do_attack = phase == offset;
        let do_structures = do_attack || phase == one_third || phase == two_thirds;

        if do_structures {
            let Some(player) = self.state.player(bot_id) else {
                continue;
            };
            let city_count = self.buildings
                .iter()
                .filter(|b| b.owner_id == bot_id && b.kind == BuildingKind::City && !b.under_construction)
                .count() as u32;
            let city_equivalent = city_count.max((player.tile_count / 2000).max(1));
            let build_order = [
                BuildingKind::DefensePost,
                BuildingKind::Port,
                BuildingKind::Factory,
                BuildingKind::SamLauncher,
                BuildingKind::MissileSilo,
                BuildingKind::City,
            ];
            for kind in build_order {
                if !structure_kind_enabled(kind) {
                    continue;
                }
                let owned = self.buildings
                    .iter()
                    .filter(|b| b.owner_id == bot_id && b.kind == kind && !b.under_construction)
                    .count() as u32;
                let target_count =
                    bot_structure_target_count(kind, city_equivalent, config.bot_difficulty);
                if kind == BuildingKind::MissileSilo && owned >= 3 {
                    continue;
                }
                if owned >= target_count {
                    continue;
                }
                let cost = structure_build_cost_gold(kind, bot_id, &self.buildings);
                let mut tile_choice: Option<u32> = None;
                let map_w = self.state.map.width;
                let Some(bot_now) = self.state.player_mut(bot_id) else { continue; };
                if bot_now.gold < cost {
                    continue;
                }

                // PERF: resolve_structure_spawn_tile allocates vec![false; map_area]
                // internally (BFS visited array). On a 2000x1200 map that's 2.4MB per call.
                // With 400 bots × 6 kinds × 256 attempts = OOM / freeze.
                // Fix: try exactly ONE random border tile. If it fails, wait until next tick.
                let border_len = bot_now.border_tiles.count_ones();
                if border_len > 0 {
                    let pick = bot_now.bot_rng.next_int(0, border_len as i32) as usize;
                    let raw_idx = bot_now.border_tiles.ones().nth(pick).unwrap();
                    let (bx, by) = (raw_idx % map_w, raw_idx / map_w);
                    let idx = by * map_w + bx;
                    if resolve_structure_spawn_tile(&self.state.map, bot_id, kind, idx, &structure_positions, &mut self.placement_scratch).is_some() {
                        tile_choice = Some(idx);
                    }
                }
                let Some(target_tile) = tile_choice else {
                    continue;
                };
                decisions.push(BotDecision {
                    bot_id,
                    kind: BotDecisionKind::Build,
                    intent: GameplayIntent::BuildStructure { kind, target_tile },
                });
                break;
            }
        }

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

#[cfg(test)]
mod bot_ratio_tests {
    use super::bot_structure_target_count;
    use crate::game::BuildingKind;
    use crate::game_config::BotDifficulty;

    #[test]
    fn bot_structure_ratio_targets_follow_openfront_like_values() {
        let city_equivalent = 10;
        let sam_vanilla = bot_structure_target_count(
            BuildingKind::SamLauncher,
            city_equivalent,
            BotDifficulty::Vanilla,
        );
        let sam_terminator = bot_structure_target_count(
            BuildingKind::SamLauncher,
            city_equivalent,
            BotDifficulty::Terminator,
        );
        let ports = bot_structure_target_count(
            BuildingKind::Port,
            city_equivalent,
            BotDifficulty::Terminator,
        );
        let silos = bot_structure_target_count(
            BuildingKind::MissileSilo,
            city_equivalent,
            BotDifficulty::Terminator,
        );
        assert_eq!(sam_vanilla, 2);
        assert_eq!(sam_terminator, 3);
        assert_eq!(ports, 7);
        assert_eq!(silos, 2);
    }
}
