use bevy::prelude::*;
use std::collections::BinaryHeap;
use crate::game::GameState;
use crate::engine::DarkRiftEngine;
use crate::protocol::*;
use crate::execution::{AttackExecution, PrioritizedTile};
use bevy_prng::WyRand;
use rand_core::SeedableRng;
use crate::rng::NextIntExt;
use crate::map::TerrainType;
pub fn merge_frontiers(
    mut a: BinaryHeap<PrioritizedTile>,
    b: BinaryHeap<PrioritizedTile>,
) -> BinaryHeap<PrioritizedTile> {
    if a.capacity() < b.len() {
        a.reserve(b.len());
    }
    a.extend(b);
    a
}

pub fn build_attack_frontier(
    game: &GameState,
    player_id: u16,
    target_owner: u16,
    border_tiles: &crate::bitset::DenseBitSet,
    rng: &mut WyRand,
    insert_seq_counter: &mut u32,
) -> BinaryHeap<PrioritizedTile> {
    let mut to_conquer = BinaryHeap::new();
    let map_w = game.map.width;
    for raw_idx in border_tiles.ones() {
        let (bx, by) = (raw_idx % map_w, raw_idx / map_w);
        game.map.for_each_neighbor(bx, by, |nx, ny| {
            if game.map.owner_id(nx, ny) != target_owner {
                return;
            }
            let mut num_owned_by_me = 0u32;
            game.map.for_each_neighbor(nx, ny, |nnx, nny| {
                if game.map.owner_id(nnx, nny) == player_id {
                    num_owned_by_me += 1;
                }
            });

            let terrain = game.map.terrain_type(nx, ny);
            let mag_x2 = match terrain {
                TerrainType::Land => 2,
                TerrainType::Highland => 3,
                TerrainType::Mountain => 4,
                TerrainType::Water | TerrainType::Lake => 3, // Fallback, won't happen normally
            };
            let r = rng.next_int(0, 7) as i64;
            // Formula scaled by 4 to maintain OpenFront quartiles in integer space
            let prio = (r + 10) * (4 - (num_owned_by_me as i64 * 2) + mag_x2)
                + (game.tick as i64 * 4);
            
            let seq = *insert_seq_counter;
            *insert_seq_counter = insert_seq_counter.wrapping_add(1);

            to_conquer.push(PrioritizedTile {
                priority: prio,
                insert_seq: seq,
                x: nx,
                y: ny,
            });
        });
    }
    to_conquer
}

pub fn spawn_or_merge_attack_for_fleet_arrival_pure(
    engine: &mut DarkRiftEngine,
    owner_id: u16,
    target_owner: u16,
    troops: f64,
    fleet_id: u64,
) {
    if troops < engine.state.config.attack_cost_neutral || !troops.is_finite() {
        if let Some(p) = engine.state.player_mut(owner_id) {
            p.troops = (p.troops + troops.max(0.0)).min(p.max_troops);
        }
        return;
    }

    let exec_seed = engine.state
        .seed
        .wrapping_add(engine.state.tick)
        .wrapping_add(fleet_id)
        .wrapping_add(0xB04F_0000);
    let mut rng = WyRand::seed_from_u64(exec_seed);
    let mut initial_seq = 0u32;
    let Some(player) = engine.state.player(owner_id) else {
        return;
    };
    let fresh = build_attack_frontier(
        &engine.state,
        owner_id,
        target_owner,
        &player.border_tiles,
        &mut rng,
        &mut initial_seq,
    );

    if fresh.is_empty() {
        if let Some(p) = engine.state.player_mut(owner_id) {
            p.troops = (p.troops + troops).min(p.max_troops);
        }
        return;
    }

    let mut merge_idx = None;
    for (i, ex) in engine.attacks.iter().enumerate() {
        if ex.owner_id == owner_id && ex.target_owner == target_owner && !ex.retreating {
            merge_idx = Some(i);
            break;
        }
    }

    if let Some(i) = merge_idx {
        let ex = &mut engine.attacks[i];
        ex.troops += troops;
        ex.initial_troops += troops;
        let merged = merge_frontiers(std::mem::take(&mut ex.to_conquer), fresh);
        ex.to_conquer = merged;
        return;
    }

    let exec_id = engine.state.next_attack_id;
    engine.state.next_attack_id = engine.state.next_attack_id.wrapping_add(1).max(1);

    engine.add_attack(AttackExecution {
        id: exec_id,
        owner_id,
        target_owner,
        troops,
        initial_troops: troops,
        to_conquer: fresh,
        insert_seq_counter: initial_seq,
        rng,
        retreating: false,
    });
}

impl DarkRiftEngine {
pub(super) fn apply_attack_intent(
    &mut self,
    player_id: u16,
    attack: &AttackIntent,
    intent_index: u32,
) {
    let target_owner = attack.target_owner;

    let Some(player) = self.state.player(player_id) else {
        bevy::log::warn!("apply_attack_intent: player {} not found", player_id);
        return;
    };

    if !player.alive {
        return;
    }

    let pool_cap = player.troops;
    let requested = attack
        .troops
        .unwrap_or(pool_cap)
        .max(0.0)
        .min(pool_cap);

    if requested < self.state.config.attack_cost_neutral {
        return;
    }

    let Some(p) = self.state.player_mut(player_id) else {
        return;
    };
    p.troops -= requested;
    p.troops = p.troops.max(0.0);

    let mut remaining = requested;

    // Mutual annihilation with enemy attacks that target us (deterministic by enemy exec id).
    let mut enemy_hits: Vec<usize> = (0..self.attacks.len())
        .filter(|&i| {
            let ex = &self.attacks[i];
            ex.owner_id == target_owner && ex.target_owner == player_id && !ex.retreating
        })
        .collect();
    enemy_hits.sort_by_key(|&i| self.attacks[i].id);

    for &e_enemy in &enemy_hits {
        if remaining < self.state.config.attack_cost_neutral {
            break;
        }
        let enemy_ex = &mut self.attacks[e_enemy];
        
        if enemy_ex.troops < self.state.config.attack_cost_enemy {
            continue;
        }
        let clash = enemy_ex.troops.min(remaining);
        enemy_ex.troops -= clash;
        enemy_ex.troops = enemy_ex.troops.max(0.0);
        remaining -= clash;
    }

    let mut to_remove = Vec::new();
    for &e_enemy in &enemy_hits {
        if self.attacks[e_enemy].troops < self.state.config.attack_cost_neutral {
            to_remove.push(e_enemy);
        }
    }
    
    // Sort reverse so we can remove safely
    to_remove.sort_unstable_by(|a, b| b.cmp(a));
    let has_removals = !to_remove.is_empty();
    for i in to_remove {
        self.attacks.swap_remove(i);
    }
    if has_removals {
        self.attacks.sort_unstable_by_key(|a| a.id);
    }

    if remaining < self.state.config.attack_cost_neutral {
        if let Some(p) = self.state.player_mut(player_id) {
            p.troops = (p.troops + remaining).min(p.max_troops);
        }
        return;
    }

    let mut rng = WyRand::seed_from_u64(
        self.state
            .seed
            .wrapping_add(self.state.tick)
            .wrapping_add(intent_index as u64),
    );
    let mut initial_seq = 0;
    let player = self.state.player(player_id).unwrap();
    let fresh = build_attack_frontier(
        &self.state,
        player_id,
        target_owner,
        &player.border_tiles,
        &mut rng,
        &mut initial_seq,
    );

    if fresh.is_empty() {
        if let Some(p) = self.state.player_mut(player_id) {
            p.troops = (p.troops + remaining).min(p.max_troops);
        }
        return;
    }

    // Merge into existing outgoing attack to same target (lowest exec id wins).
    let mut merge_idx = None;
    for (i, ex) in self.attacks.iter().enumerate() {
        if ex.owner_id == player_id && ex.target_owner == target_owner && !ex.retreating {
            merge_idx = Some(i);
            break;
        }
    }

    if let Some(i) = merge_idx {
        let ex = &mut self.attacks[i];
        ex.troops += remaining;
        ex.initial_troops += remaining;
        let merged = merge_frontiers(std::mem::take(&mut ex.to_conquer), fresh);
        ex.to_conquer = merged;
        return;
    }

    let exec_id = self.state.next_attack_id;
    self.state.next_attack_id = self.state.next_attack_id.wrapping_add(1).max(1);
    self.add_attack(AttackExecution {
        id: exec_id,
        owner_id: player_id,
        target_owner,
        troops: remaining,
        initial_troops: remaining,
        to_conquer: fresh,
        insert_seq_counter: initial_seq,
        rng,
        retreating: false,
    });
}
}
