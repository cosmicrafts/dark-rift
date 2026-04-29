use crate::game::GameState;
use crate::building::Building;
use crate::warp_fleet::WarpFleet;
use crate::execution::AttackExecution;

/// Computes a lightweight, numeric checksum of the game state.
/// This mimics OpenFront's `hash()` method which simply sums primitive values
/// (troops, gold, coordinates) to detect desyncs with virtually zero overhead.
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

    // Railways + trains (OpenFront parity for desync detection)
    let rail = &game.rail;
    for s in &rail.stations {
        hash = hash.wrapping_add(s.id as u64);
        hash = hash.wrapping_add(s.building_id);
        hash = hash.wrapping_add(s.tile_idx as u64);
        hash = hash.wrapping_add(s.owner_id as u64);
    }
    for r in &rail.railroads {
        hash = hash.wrapping_add(r.id);
        hash = hash.wrapping_add(r.from_station as u64);
        hash = hash.wrapping_add(r.to_station as u64);
        for &t in &r.tiles {
            hash = hash.wrapping_add(t as u64);
        }
    }
    for t in &rail.trains {
        hash = hash.wrapping_add(t.id);
        hash = hash.wrapping_add(if t.active { 1 } else { 0 });
        hash = hash.wrapping_add(t.owner_id as u64);
        hash = hash.wrapping_add(t.current_leg as u64);
        hash = hash.wrapping_add(t.idx_in_leg as u64);
        hash = hash.wrapping_add(t.trade_stops_visited as u64);
        for &sid in &t.path_stations {
            hash = hash.wrapping_add(sid as u64);
        }
    }

    hash
}
