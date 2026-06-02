use crate::engine::DarkRiftEngine;
use crate::game::BuildingKind;
use crate::protocol::{GameplayIntent, StampedIntent, AttackIntent};
use crate::building::{aggregate_buildings_per_player, structure_kind_enabled, structure_build_cost_gold, resolve_structure_spawn_tile};
use crate::rng::{NextIntExt, WyRand, SeedableRng};

/// Maximum number of AI entities that may produce decisions in a single tick.
/// This prevents thundering-herd spikes when many bots/nations have aligned
/// intervals. Excess entities are deferred to the next tick via round-robin.
const MAX_AI_DECISIONS_PER_TICK: usize = 1;

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

/// Cheapest possible gold cost for a building kind (count=0, no scaling).
/// Used as a fast pre-check to skip expensive placement logic when the bot
/// clearly cannot afford any building of this type.
#[inline]
fn cheapest_gold_cost(kind: BuildingKind) -> f64 {
    let s = crate::config::OPENFRONT_GOLD_SCALE.max(1.0);
    match kind {
        BuildingKind::City | BuildingKind::Factory | BuildingKind::Port => 125_000.0 / s,
        BuildingKind::DefensePost => 50_000.0 / s,
        BuildingKind::SamLauncher => 1_500_000.0 / s,
        BuildingKind::MissileSilo => 1_000_000.0 / s,
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

/// Describes which behaviours an AI entity should run this tick.
struct AiSlot {
    bot_id: u16,
    is_nation: bool,
    do_attack: bool,
    do_structures: bool,
}

impl DarkRiftEngine {
/// Unified AI pipeline for both Tribes (`Bot`) and Nations.
///
/// - Builds one combined schedule of all AI entities.
/// - Applies a per-tick budget (`MAX_AI_DECISIONS_PER_TICK`) with a round-robin
///   cursor so no single spike processes more than N bots.
/// - Uses `placement_scratch.border_scratch` for zero-allocation border scanning.
pub fn execute_ai_think(&mut self) {
    if self.state.phase != crate::game::GamePhase::Playing {
        return;
    }

    let config = self.state.config.clone();
    let tick = self.state.tick;

    // --- Difficulty-tuned parameters (Nations use these; Bots use fixed values) ---
    let (nation_attack_interval_base, nation_trigger_ratio, nation_reserve_ratio, nation_expand_ratio, nation_random_attack_div) =
        match config.bot_difficulty {
            crate::game_config::BotDifficulty::BrainDead => (500, 0.9, 0.8, 0.12, 8),
            crate::game_config::BotDifficulty::Vanilla => (250, 0.6, 0.3, 0.15, 5),
            crate::game_config::BotDifficulty::Terminator => (125, 0.4, 0.1, 0.18, 3),
        };

    // Bot (Tribe) fixed parameters
    let bot_attack_interval_base: u64 = 250;
    let bot_trigger_ratio = 0.6;
    let bot_reserve_ratio = 0.3;
    let bot_expand_ratio = 0.15;
    let bot_random_attack_div: i32 = 5;

    // ── Build unified schedule ──────────────────────────────────────────
    let mut schedule: Vec<AiSlot> = Vec::new();
    let mut any_structures = false;

    for p in self.state.players.iter() {
        let is_nation = p.player_type == crate::player::PlayerType::Nation;
        let is_bot = p.player_type == crate::player::PlayerType::Bot;
        if (!is_nation && !is_bot) || !p.alive {
            continue;
        }
        let bot_id = p.id;
        let interval_base = if is_nation { nation_attack_interval_base as u64 } else { bot_attack_interval_base };

        let mut sched_rng = WyRand::seed_from_u64(
            self.state.seed.wrapping_add(bot_id as u64).wrapping_add(interval_base)
        );
        let interval = sched_rng
            .next_int(interval_base as i32, (interval_base as i32 * 2).max(1))
            .max(1) as u64;
        let offset = sched_rng.next_int(0, interval as i32) as u64;

        let phase = tick % interval;
        let do_attack = phase == offset;

        // Nations get structure phases at 1/3 and 2/3 intervals; Bots never build
        let do_structures = if is_nation {
            let one_third = (offset + interval / 3) % interval;
            let two_thirds = (offset + (interval * 2) / 3) % interval;
            do_attack || phase == one_third || phase == two_thirds
        } else {
            false
        };

        if !do_attack && !do_structures {
            continue; // Nothing to do this tick for this entity
        }

        if do_structures && p.gold >= cheapest_gold_cost(BuildingKind::DefensePost) {
            any_structures = true;
        }

        schedule.push(AiSlot {
            bot_id,
            is_nation,
            do_attack,
            do_structures,
        });
    }

    schedule.sort_unstable_by_key(|s| s.bot_id);
    if schedule.is_empty() {
        return;
    }

    // ── Pre-compute aggregates if any nation wants to build ─────────────
    if any_structures {
        self.refresh_building_grid();
        if self.building_aggregates_dirty {
            let max_pid = self.state.players.iter().map(|p| p.id as usize).max().unwrap_or(0);
            self.building_aggregates = aggregate_buildings_per_player(self.buildings.iter().copied(), max_pid);
            self.building_aggregates_dirty = false;
        }
    }

    // ── Apply round-robin budget ────────────────────────────────────────
    let total = schedule.len();
    if self.ai_round_robin >= total {
        self.ai_round_robin = 0;
    }
    let start = self.ai_round_robin;
    let budget = total.min(MAX_AI_DECISIONS_PER_TICK);

    let mut decisions: Vec<BotDecision> = Vec::new();
    let mut processed = 0usize;

    for raw_i in 0..total {
        if processed >= budget {
            break;
        }
        let i = (start + raw_i) % total;
        let slot = &schedule[i];
        let bot_id = slot.bot_id;

        // ── Structure building (Nations only) ───────────────────────
        if slot.do_structures && slot.is_nation {
            let Some(player) = self.state.player(bot_id) else {
                processed += 1;
                continue;
            };
            let player_gold = player.gold;
            if player_gold >= cheapest_gold_cost(BuildingKind::DefensePost) {
                let agg = self.building_aggregates.get(bot_id as usize).copied().unwrap_or_default();
                let city_equivalent =
                    agg.ready_city_count.max((player.tile_count / 2000).max(1));
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
                    let owned = agg.total_structures_of_kind(kind);
                    let target_count =
                        bot_structure_target_count(kind, city_equivalent, config.bot_difficulty);
                    if kind == BuildingKind::MissileSilo && owned >= 3 {
                        continue;
                    }
                    if owned >= target_count {
                        continue;
                    }
                    if player_gold < cheapest_gold_cost(kind) {
                        continue;
                    }
                    let cost = structure_build_cost_gold(kind, bot_id, &self.buildings);
                    let map_w = self.state.map.width;
                    let Some(bot_now) = self.state.player_mut(bot_id) else { continue; };
                    if bot_now.gold < cost {
                        continue;
                    }

                    let border_len = bot_now.border_tiles.count_ones();
                    let mut tile_choice: Option<u32> = None;
                    if border_len > 0 {
                        let pick = bot_now.bot_rng.next_int(0, border_len as i32) as usize;
                        let chosen_idx_opt = bot_now.border_tiles.ones().nth(pick);
                        if let Some(chosen_idx) = chosen_idx_opt {
                            let (bx, by) = (chosen_idx % map_w, chosen_idx / map_w);
                            let idx = by * map_w + bx;
                            if resolve_structure_spawn_tile(&self.state.map, bot_id, kind, idx, &self.building_grid, &mut self.placement_scratch).is_some() {
                                tile_choice = Some(idx);
                            }
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
        }

        // ── Attack logic (both Bots and Nations) ────────────────────
        if slot.do_attack {
            let (trigger_ratio, reserve_ratio, expand_ratio, random_attack_div) = if slot.is_nation {
                (nation_trigger_ratio, nation_reserve_ratio, nation_expand_ratio, nation_random_attack_div)
            } else {
                (bot_trigger_ratio, bot_reserve_ratio, bot_expand_ratio, bot_random_attack_div)
            };

            let (troops, max_troops) = {
                let Some(player) = self.state.player(bot_id) else {
                    processed += 1;
                    continue;
                };
                (player.troops, player.max_troops)
            };

            // Zero-allocation border scanning via placement_scratch
            self.placement_scratch.border_scratch.clear();
            if let Some(player) = self.state.player(bot_id) {
                self.placement_scratch.border_scratch.extend(player.border_coords(self.state.map.width));
            }

            if !self.placement_scratch.border_scratch.is_empty() {
                let border_len = self.placement_scratch.border_scratch.len();
                let mut has_neutral = false;
                let mut player_targets = Vec::new();
                let max_scan = border_len.min(256);
                let start_idx = {
                    let p_mut = self.state.player_mut(bot_id).unwrap();
                    p_mut.bot_rng.next_int(0, border_len as i32) as usize
                };
                for si in 0..max_scan {
                    let b_idx = (start_idx + si) % border_len;
                    let (bx, by) = self.placement_scratch.border_scratch[b_idx];
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
                    processed += 1;
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
                if is_neutral || troops >= max_troops * trigger_ratio {
                    let reserve = max_troops * if is_neutral { expand_ratio } else { reserve_ratio };
                    let p_send = (troops - reserve).max(0.0);
                    if p_send >= self.state.config.attack_cost_neutral {
                        decisions.push(BotDecision {
                            bot_id,
                            kind: BotDecisionKind::Attack,
                            intent: GameplayIntent::Attack(AttackIntent {
                                target_owner,
                                troops: Some(p_send),
                            }),
                        });
                    }
                }
            }
        }

        processed += 1;
    }

    // Advance round-robin cursor for next tick
    self.ai_round_robin = (start + processed) % total.max(1);

    // ── Apply decisions deterministically ───────────────────────────────
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
