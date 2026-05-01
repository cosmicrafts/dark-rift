use crate::engine::DarkRiftEngine;
use crate::game::GameState;
use crate::building::Building;
use crate::warp_fleet::WarpFleet;
use crate::execution::AttackExecution;

/// Computes a lightweight, numeric checksum of the game state.
/// This mimics OpenFront's `hash()` method which simply sums primitive values
/// (troops, gold, coordinates) to detect desyncs with virtually zero overhead.
impl DarkRiftEngine {
    pub fn checksum(&self) -> u64 {
        compute_state_hash(
            &self.state,
            self.buildings.iter(),
            self.fleets.iter(),
            self.attacks.iter(),
        )
    }
}

pub fn compute_state_hash<'a>(
    game: &GameState,
    buildings: impl Iterator<Item = &'a Building>,
    fleets: impl Iterator<Item = &'a WarpFleet>,
    execs: impl Iterator<Item = &'a AttackExecution>,
) -> u64 {
    let mut hash: u64 = 0;

    // Hash Players
    for (i, p) in game.players.iter().enumerate() {
        if !p.alive {
            continue;
        }
        hash = hash.wrapping_add((i as u64).wrapping_mul(1000));
        hash = hash.wrapping_add(p.troops.to_bits());
        hash = hash.wrapping_add(p.gold.to_bits());
        hash = hash.wrapping_add(p.tile_count as u64);
        hash = hash.wrapping_add(p.sum_x as u64);
        hash = hash.wrapping_add(p.sum_y as u64);
    }

    // Hash Buildings
    for b in buildings {
        hash = hash.wrapping_add(b.id);
        hash = hash.wrapping_add((b.owner_id as u64).wrapping_mul(100));
        hash = hash.wrapping_add(b.tile_idx as u64);
        hash = hash.wrapping_add(b.level as u64);
        hash = hash.wrapping_add(if b.under_construction { 1 } else { 0 });
        hash = hash.wrapping_add(b.ticks_until_complete as u64);
    }

    // Hash Fleets
    for f in fleets {
        hash = hash.wrapping_add(f.id);
        hash = hash.wrapping_add(f.owner_id as u64);
        hash = hash.wrapping_add(f.troops.to_bits());
        hash = hash.wrapping_add(f.current_tile as u64);
        hash = hash.wrapping_add(f.dst_tile as u64);
    }

    // Hash Executions
    for e in execs {
        hash = hash.wrapping_add(e.id);
        hash = hash.wrapping_add(e.owner_id as u64);
        hash = hash.wrapping_add(e.troops.to_bits());
    }

    // Hash map ownership
    // 65k iterations vectorized -> fractions of a microsecond
    for (idx, &owner) in game.map.state.iter().enumerate() {
        if owner != 0 {
            hash = hash.wrapping_add((owner as u64).wrapping_mul(idx as u64));
        }
    }

    hash
}
