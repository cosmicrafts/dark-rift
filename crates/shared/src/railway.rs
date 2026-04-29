//! OpenFront-style railways: stations on City/Port/Factory near factories, tile A* rails,
//! station-graph paths, factory train spawning, movement, and trade gold.
//! All logic is CPU-side and deterministic (no GPU).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, BTreeMap, BTreeSet, VecDeque};

use bevy_prng::WyRand;
use rand_core::SeedableRng;
use serde::{Deserialize, Serialize};

use crate::building::core::Building;
use crate::building::placement::{idx_xy, manhattan};
use crate::game::{BuildingKind, GameState};
use crate::map::GameMap;
use crate::rng::NextIntExt;

// --- OpenFront `DefaultConfig` parity ---------------------------------------

pub const RAILROAD_MAX_SIZE: usize = 120;
pub const TRAIN_STATION_MIN_RANGE: i32 = 15;
pub const TRAIN_STATION_MIN_RANGE_SQ: i32 = TRAIN_STATION_MIN_RANGE * TRAIN_STATION_MIN_RANGE;
pub const TRAIN_STATION_MAX_RANGE: i32 = 100;
pub const MAX_CONNECTION_DISTANCE: i32 = 4;
pub const STATION_RAIL_SNAP_RADIUS: i32 = 3;
pub const TRAIN_SPEED: usize = 2;
pub const TRAIN_SPACING: usize = 2;
pub const TRAIN_NUM_CARS: usize = 5;
pub const TRAIN_SPAWN_COOLDOWN_TICKS: u64 = 10;

const WATER_PENALTY: i32 = 5;
const DIRECTION_CHANGE_PENALTY: i32 = 3;
const HEURISTIC_WEIGHT: i32 = 2;

// --- Tile A* (OpenFront `AStarRail` / `RailAdapter`) ------------------------

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct TileSearchNode {
    neg_f: i32,
    neg_g: i32,
    tile: u32,
    seq: u32,
}

impl Ord for TileSearchNode {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.neg_f, self.neg_g, self.tile, self.seq).cmp(&(
            other.neg_f,
            other.neg_g,
            other.tile,
            other.seq,
        ))
    }
}

impl PartialOrd for TileSearchNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[inline]
fn tile_xy(tile: u32, w: u32) -> (i32, i32) {
    ((tile % w) as i32, (tile / w) as i32)
}

#[inline]
fn is_water(map: &GameMap, tile: u32) -> bool {
    map.terrain[tile as usize].is_water()
}

#[inline]
fn is_shoreline(map: &GameMap, tile: u32) -> bool {
    map.terrain[tile as usize].is_shoreline()
}

fn is_traversable(map: &GameMap, to: u32, from_shoreline: bool) -> bool {
    if !is_water(map, to) {
        return true;
    }
    from_shoreline || is_shoreline(map, to)
}

fn tile_move_cost(map: &GameMap, from: u32, to: u32, prev: Option<u32>, _w: u32) -> i32 {
    let penalized = is_water(map, to) || is_shoreline(map, to);
    let mut c = if penalized { 1 + WATER_PENALTY } else { 1 };
    if let Some(p) = prev {
        let d1 = from as i32 - p as i32;
        let d2 = to as i32 - from as i32;
        if d1 != d2 {
            c += DIRECTION_CHANGE_PENALTY;
        }
    }
    c
}

fn manhattan_heuristic(a: u32, b: u32, w: u32) -> i32 {
    let (ax, ay) = tile_xy(a, w);
    let (bx, by) = tile_xy(b, w);
    HEURISTIC_WEIGHT * ((ax - bx).abs() + (ay - by).abs())
}

/// Reusable A* engine for rail pathfinding.
/// Architecture mirrors the proven `WaterAStar` in `pathfinding.rs`:
/// - Two stamp arrays (`closed_stamp` + `gscore_stamp`) for proper closed-set tracking
/// - `came_from` as `Vec<i32>` with -1 sentinel (no redundant `prev_for_cost`)
/// - `BinaryHeap` stored as a field (reused across calls, not re-allocated)
/// - **No** `Serialize/Deserialize/Clone` — scratch buffers are transient compute artifacts
#[derive(Debug)]
pub struct RailPathScratch {
    closed_stamp: Vec<u32>,
    gscore_stamp: Vec<u32>,
    gscore: Vec<u32>,
    came_from: Vec<i32>,
    stamp: u32,
    capacity: usize,
    heap: BinaryHeap<TileSearchNode>,
}

impl Default for RailPathScratch {
    fn default() -> Self {
        Self {
            closed_stamp: Vec::new(),
            gscore_stamp: Vec::new(),
            gscore: Vec::new(),
            came_from: Vec::new(),
            stamp: 0,
            capacity: 0,
            heap: BinaryHeap::new(),
        }
    }
}

impl PartialEq for RailPathScratch {
    fn eq(&self, _other: &Self) -> bool {
        true // Scratch buffers are semantically equal — they hold no game state
    }
}

impl RailPathScratch {
    fn ensure_capacity(&mut self, n: usize) {
        if self.capacity >= n {
            return;
        }
        self.closed_stamp.resize(n, 0);
        self.gscore_stamp.resize(n, 0);
        self.gscore.resize(n, u32::MAX);
        self.came_from.resize(n, -1);
        self.capacity = n;
    }

    /// O(1) logical clear — bump the stamp. Physical clear only on u32 wraparound.
    fn clear(&mut self) {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.closed_stamp.fill(0);
            self.gscore_stamp.fill(0);
            self.stamp = 1;
        }
        self.heap.clear();
    }

    #[inline]
    fn is_closed(&self, idx: usize) -> bool {
        self.closed_stamp[idx] == self.stamp
    }

    #[inline]
    fn close(&mut self, idx: usize) {
        self.closed_stamp[idx] = self.stamp;
    }

    #[inline]
    fn g(&self, idx: usize) -> i32 {
        if self.gscore_stamp[idx] == self.stamp {
            self.gscore[idx] as i32
        } else {
            i32::MAX
        }
    }

    #[inline]
    fn set_g(&mut self, idx: usize, val: i32) {
        self.gscore[idx] = val as u32;
        self.gscore_stamp[idx] = self.stamp;
    }

    #[inline]
    fn set_came_from(&mut self, idx: usize, from: i32) {
        self.came_from[idx] = from;
    }

    #[inline]
    fn get_came_from(&self, idx: usize) -> Option<u32> {
        if self.gscore_stamp[idx] == self.stamp && self.came_from[idx] >= 0 {
            Some(self.came_from[idx] as u32)
        } else {
            None
        }
    }

    /// Read the previous node for direction-change cost (derived from came_from chain).
    #[inline]
    fn get_prev(&self, idx: usize) -> Option<u32> {
        self.get_came_from(idx)
    }
}

/// Returns a path **including** endpoints, or empty if unreachable / too long.
/// Uses the reusable `scratch` buffers to avoid per-call allocation.
pub fn find_rail_tile_path(map: &GameMap, from: u32, to: u32, scratch: &mut RailPathScratch) -> Vec<u32> {
    let w = map.width;
    let h = map.height;
    let n = (w * h) as usize;
    if from == to {
        return vec![from];
    }

    scratch.ensure_capacity(n);
    scratch.clear();
    scratch.set_g(from as usize, 0);
    scratch.set_came_from(from as usize, -1);

    let mut seq: u32 = 0;
    scratch.heap.push(TileSearchNode {
        neg_f: -manhattan_heuristic(from, to, w),
        neg_g: 0,
        tile: from,
        seq,
    });
    seq += 1;

    let mut iterations = 20_000;

    while let Some(TileSearchNode { tile: current, .. }) = scratch.heap.pop() {
        if iterations == 0 {
            return Vec::new();
        }
        iterations -= 1;

        let ci = current as usize;

        // Skip already-closed nodes (proper closed-set check)
        if scratch.is_closed(ci) {
            continue;
        }
        scratch.close(ci);

        if current == to {
            break;
        }

        let current_g = scratch.g(ci);
        if current_g == i32::MAX {
            continue;
        }

        let (cx, cy) = tile_xy(current, w);
        let from_shoreline = is_shoreline(map, current);
        // Read prev from came_from chain (no redundant prev_for_cost array)
        let prev = scratch.get_prev(ci);

        // Neighbor expansion: N, S, W, E
        let neighbors: [(bool, u32); 4] = [
            (cy > 0,            current.wrapping_sub(w)),
            (cy + 1 < h as i32, current.wrapping_add(w)),
            (cx > 0,            current.wrapping_sub(1)),
            (cx + 1 < w as i32, current.wrapping_add(1)),
        ];

        for &(valid, next) in &neighbors {
            if !valid {
                continue;
            }
            let ni = next as usize;
            if scratch.is_closed(ni) {
                continue;
            }
            if !is_traversable(map, next, from_shoreline) {
                continue;
            }
            let step = tile_move_cost(map, current, next, prev, w);
            let tentative = current_g.saturating_add(step);
            if tentative < scratch.g(ni) {
                scratch.set_g(ni, tentative);
                scratch.set_came_from(ni, current as i32);
                let f = tentative + manhattan_heuristic(next, to, w);
                scratch.heap.push(TileSearchNode {
                    neg_f: -f,
                    neg_g: -tentative,
                    tile: next,
                    seq,
                });
                seq += 1;
            }
        }
    }

    if scratch.g(to as usize) == i32::MAX {
        return Vec::new();
    }

    let mut path = Vec::new();
    let mut cur = Some(to);
    while let Some(t) = cur {
        path.push(t);
        if t == from {
            break;
        }
        cur = scratch.get_came_from(t as usize);
    }
    path.reverse();
    if path.first() != Some(&from) {
        return Vec::new();
    }
    if path.len() >= RAILROAD_MAX_SIZE {
        return Vec::new();
    }
    path
}

// --- Station graph A* -------------------------------------------------------

#[derive(Copy, Clone, Eq, PartialEq)]
struct StationSearchNode {
    neg_f: i32,
    neg_g: i32,
    station_idx: usize,
    seq: u32,
}

impl Ord for StationSearchNode {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            self.neg_f,
            self.neg_g,
            self.station_idx,
            self.seq,
        )
            .cmp(&(other.neg_f, other.neg_g, other.station_idx, other.seq))
    }
}

impl PartialOrd for StationSearchNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn find_station_path_tile_heuristic(
    map: &GameMap,
    stations: &[Station],
    from_idx: usize,
    to_idx: usize,
) -> i32 {
    let w = map.width;
    let sa = stations[from_idx].tile_idx;
    let sb = stations[to_idx].tile_idx;
    let (ax, ay) = tile_xy(sa, w);
    let (bx, by) = tile_xy(sb, w);
    (ax - bx).abs() + (ay - by).abs()
}

/// Station IDs along the route from `from` to `to` (inclusive), same cluster only.
pub fn find_stations_path(
    map: &GameMap,
    stations: &[Station],
    uf: &mut UnionFind,
    from_sid: u32,
    to_sid: u32,
) -> Option<Vec<u32>> {
    if from_sid == to_sid {
        return Some(vec![from_sid]);
    }
    
    // Create a fast lookup map for ID -> Index once
    let mut id_to_idx = std::collections::HashMap::new();
    for (i, s) in stations.iter().enumerate() {
        id_to_idx.insert(s.id, i);
    }
    
    let ia = *id_to_idx.get(&from_sid)?;
    let ib = *id_to_idx.get(&to_sid)?;
    if uf.find(ia) != uf.find(ib) {
        return None;
    }

    let n = stations.len();
    let mut g = vec![i32::MAX; n];
    let mut came: Vec<Option<usize>> = vec![None; n];
    g[ia] = 0;
    let mut heap = BinaryHeap::new();
    let mut seq = 0u32;
    heap.push(StationSearchNode {
        neg_f: -find_station_path_tile_heuristic(map, stations, ia, ib),
        neg_g: 0,
        station_idx: ia,
        seq,
    });
    seq += 1;

    while let Some(StationSearchNode { station_idx: cur_i, .. }) = heap.pop() {
        if cur_i == ib {
            break;
        }
        let cur_g = g[cur_i];

        let neighbors: Vec<u32> = stations[cur_i].neighbors.keys().copied().collect();
        for nb in neighbors {
            let Some(&ni) = id_to_idx.get(&nb) else { continue };
            let tentative = cur_g.saturating_add(1);
            if tentative < g[ni] {
                g[ni] = tentative;
                came[ni] = Some(cur_i);
                let f = tentative + find_station_path_tile_heuristic(map, stations, ni, ib);
                heap.push(StationSearchNode {
                    neg_f: -f,
                    neg_g: -tentative,
                    station_idx: ni,
                    seq,
                });
                seq += 1;
            }
        }
    }

    let end_i = ib;
    if g[end_i] == i32::MAX {
        return None;
    }

    let mut idx_path: Vec<usize> = Vec::new();
    let mut cur = Some(end_i);
    while let Some(i) = cur {
        idx_path.push(i);
        if i == ia {
            break;
        }
        cur = came[i];
    }
    idx_path.reverse();
    Some(idx_path.into_iter().map(|i| stations[i].id).collect())
}

// --- Union-find -------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    pub fn new() -> Self {
        Self { parent: Vec::new() }
    }

    pub fn push_station(&mut self) -> usize {
        let i = self.parent.len();
        self.parent.push(i);
        i
    }

    pub fn find(&mut self, mut i: usize) -> usize {
        let mut root = i;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        while self.parent[i] != i {
            let n = self.parent[i];
            self.parent[i] = root;
            i = n;
        }
        root
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        let (new_root, old) = if ra < rb { (ra, rb) } else { (rb, ra) };
        self.parent[old] = new_root;
    }
}

// --- Core rail types --------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Station {
    pub id: u32,
    pub building_id: u64,
    pub owner_id: u16,
    pub tile_idx: u32,
    pub kind: BuildingKind,
    /// Neighbor station id -> railroad id
    pub neighbors: BTreeMap<u32, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Railroad {
    pub id: u64,
    pub from_station: u32,
    pub to_station: u32,
    pub tiles: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StationRunner {
    pub building_id: u64,
    pub spawn_trains: bool,
    pub activated: bool,
    pub last_spawn_tick: u64,
    pub spawn_rng: WyRand,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TrainRuntime {
    #[serde(default)]
    pub id: u64,
    pub active: bool,
    pub owner_id: u16,
    pub path_stations: Vec<u32>,
    pub current_leg: usize,
    pub idx_in_leg: usize,
    pub trade_stops_visited: u32,
    pub used_tiles_ring: std::collections::VecDeque<u32>,
}

impl TrainRuntime {
    /// Tile index used for client rendering (engine position along current leg).
    pub fn display_engine_tile(&self, world: &RailWorld) -> Option<u32> {
        if !self.active || self.path_stations.len() < self.current_leg + 2 {
            return None;
        }
        let from = self.path_stations[self.current_leg];
        let to = self.path_stations[self.current_leg + 1];
        let rail = world.get_rail_by_stations(from, to)?;
        if rail.tiles.is_empty() {
            return None;
        }
        let i = self.idx_in_leg.min(rail.tiles.len().saturating_sub(1));
        world.oriented_leg_tile(rail, from, i)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct RailWorld {
    pub stations: Vec<Station>,
    pub station_index_by_building: BTreeMap<u64, usize>,
    pub railroads: Vec<Railroad>,
    pub next_station_id: u32,
    pub next_rail_id: u64,
    pub uf: UnionFind,
    pub runners: Vec<StationRunner>,
    pub pending_factory_neighbor_factories: Vec<u64>,
    pub registered_runners: BTreeSet<u64>,
    #[serde(default)]
    pub next_train_id: u64,
    pub trains: Vec<TrainRuntime>,
}

impl Default for UnionFind {
    fn default() -> Self {
        Self::new()
    }
}

impl RailWorld {
    pub fn train_spawn_rate(num_player_factories: u32) -> u32 {
        (num_player_factories + 10) * 15
    }

    /// OpenFront `DefaultConfig.trainGold` scaled to `f64` gold.
    pub fn train_gold(rel_self_trade: bool, cities_visited: u32, gold_mult: f64) -> f64 {
        let v = cities_visited.saturating_sub(9);
        let base = if rel_self_trade {
            10_000.0
        } else {
            25_000.0
        };
        let dist_pen = v as f64 * 5_000.0;
        (5_000.0_f64).max(base - dist_pen) * gold_mult
    }

    pub fn trade_available(station_owner: u16, train_owner: u16) -> bool {
        station_owner == train_owner
    }

    fn station_idx_by_id(&self, sid: u32) -> Option<usize> {
        self.stations.iter().position(|s| s.id == sid)
    }

    pub fn distance_hops(&self, start_sid: u32, dest_sid: u32, max_distance: i32) -> i32 {
        if start_sid == dest_sid {
            return 0;
        }
        let mut visited = BTreeSet::new();
        let mut q = VecDeque::new();
        q.push_back((start_sid, 0));
        while let Some((u, d)) = q.pop_front() {
            if u == dest_sid {
                return d;
            }
            if !visited.insert(u) {
                continue;
            }
            if d >= max_distance {
                continue;
            }
            let Some(si) = self.station_idx_by_id(u) else {
                continue;
            };
            for nb in self.stations[si].neighbors.keys() {
                q.push_back((*nb, d + 1));
            }
        }
        -1
    }

    fn rail_near_tile(&self, map: &GameMap, tile_idx: u32, radius: i32) -> Vec<usize> {
        let (tx, ty) = idx_xy(tile_idx, map.width);
        let mut out = Vec::new();
        for (ri, rail) in self.railroads.iter().enumerate() {
            let mut hit = false;
            for &t in &rail.tiles {
                let (x, y) = idx_xy(t, map.width);
                if manhattan(tx as i32, ty as i32, x as i32, y as i32) <= radius {
                    hit = true;
                    break;
                }
            }
            if hit {
                out.push(ri);
            }
        }
        out
    }

    fn closest_tile_index_on_rail(map: &GameMap, rail: &Railroad, to_tile: u32) -> usize {
        let (tx, ty) = idx_xy(to_tile, map.width);
        let mut best_i = 0usize;
        let mut best_d = i32::MAX;
        for (i, &t) in rail.tiles.iter().enumerate() {
            let (x, y) = idx_xy(t, map.width);
            let d = (tx as i32 - x as i32).pow(2) + (ty as i32 - y as i32).pow(2);
            if d < best_d {
                best_d = d;
                best_i = i;
            }
        }
        best_i
    }

    fn remove_rail_internal(&mut self, rail_idx: usize) {
        if rail_idx >= self.railroads.len() {
            return;
        }
        self.railroads.swap_remove(rail_idx);
        self.rebuild_neighbor_maps();
        self.rebuild_uf_from_rails();
    }

    fn rebuild_uf_from_rails(&mut self) {
        self.uf = UnionFind::new();
        for _ in &self.stations {
            self.uf.push_station();
        }
        let pairs: Vec<(u32, u32)> = self
            .railroads
            .iter()
            .map(|r| (r.from_station, r.to_station))
            .collect();
        for (a, b) in pairs {
            if let (Some(ia), Some(ib)) = (self.station_idx_by_id(a), self.station_idx_by_id(b)) {
                self.uf.union(ia, ib);
            }
        }
    }

    fn rebuild_neighbor_maps(&mut self) {
        for s in &mut self.stations {
            s.neighbors.clear();
        }
        let id_by_idx: Vec<u32> = self.stations.iter().map(|s| s.id).collect();
        let mut new_nb: Vec<BTreeMap<u32, u64>> =
            (0..self.stations.len()).map(|_| BTreeMap::new()).collect();
        for r in &self.railroads {
            if let (Some(ia), Some(ib)) = (
                self.station_idx_by_id(r.from_station),
                self.station_idx_by_id(r.to_station),
            ) {
                new_nb[ia].insert(id_by_idx[ib], r.id);
                new_nb[ib].insert(id_by_idx[ia], r.id);
            }
        }
        for (s, nb) in self.stations.iter_mut().zip(new_nb.into_iter()) {
            s.neighbors = nb;
        }
    }

    fn connect_rail_record(
        &mut self,
        _map: &GameMap,
        from_sid: u32,
        to_sid: u32,
        tiles: Vec<u32>,
    ) {
        if tiles.is_empty() || tiles.len() >= RAILROAD_MAX_SIZE {
            return;
        }
        let id = self.next_rail_id;
        self.next_rail_id += 1;
        let rail = Railroad {
            id,
            from_station: from_sid,
            to_station: to_sid,
            tiles,
        };
        self.railroads.push(rail);
        let ia = self.station_idx_by_id(from_sid).unwrap();
        let ib = self.station_idx_by_id(to_sid).unwrap();
        self.stations[ia].neighbors.insert(to_sid, id);
        self.stations[ib].neighbors.insert(from_sid, id);
        self.uf.union(ia, ib);
    }

    fn try_snap_station_to_existing_rails(&mut self, map: &GameMap, new_station_idx: usize) -> bool {
        let tile = self.stations[new_station_idx].tile_idx;
        let hits = self.rail_near_tile(map, tile, STATION_RAIL_SNAP_RADIUS);
        if hits.is_empty() {
            return false;
        }
        // Deterministic: lowest rail id first
        let mut hits = hits;
        hits.sort_by_key(|&i| self.railroads.get(i).map(|r| r.id).unwrap_or(u64::MAX));

        let station_sid = self.stations[new_station_idx].id;
        let mut any = false;
        for rail_idx in hits {
            if rail_idx >= self.railroads.len() {
                continue;
            }
            let rail = self.railroads[rail_idx].clone();
            let closest = Self::closest_tile_index_on_rail(map, &rail, tile);
            if closest == 0 || closest >= rail.tiles.len().saturating_sub(1) {
                continue;
            }

            // Remove old rail from graph
            self.remove_rail_internal(rail_idx);

            let from_sid = rail.from_station;
            let to_sid = rail.to_station;
            let t1 = rail.tiles[..=closest].to_vec();
            let t2 = rail.tiles[closest..].to_vec();

            self.connect_rail_record(map, from_sid, station_sid, t1);
            self.connect_rail_record(map, station_sid, to_sid, t2);
            any = true;
            break;
        }
        any
    }

    fn try_connect_nearby_stations(&mut self, game: &GameState, new_station_idx: usize, scratch: &mut RailPathScratch) {
        let map = &game.map;
        let sid = self.stations[new_station_idx].id;
        let stile = self.stations[new_station_idx].tile_idx;
        let (sx, sy) = idx_xy(stile, map.width);

        let mut candidates: Vec<(i32, u64, u32, usize)> = Vec::new();
        for (i, o) in self.stations.iter().enumerate() {
            if i == new_station_idx {
                continue;
            }
            if !matches!(
                o.kind,
                BuildingKind::City | BuildingKind::Port | BuildingKind::Factory
            ) {
                continue;
            }
            let (ox, oy) = idx_xy(o.tile_idx, map.width);
            let d = manhattan(sx as i32, sy as i32, ox as i32, oy as i32);
            if d > TRAIN_STATION_MAX_RANGE {
                continue;
            }
            candidates.push((d, o.building_id, o.id, i));
        }
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });

        let mut edited_roots = BTreeSet::new();
        let mut pathfind_attempts = 0;
        for (_d, _bid, other_sid, other_i) in candidates {
            if other_sid == sid {
                continue;
            }
            let d_hops = self.distance_hops(other_sid, sid, MAX_CONNECTION_DISTANCE);
            let neighbor_cluster_root = self.uf.find(other_i);
            let connection_available = !(0..=MAX_CONNECTION_DISTANCE).contains(&d_hops);
            let dist_sq = {
                let (ox, oy) = idx_xy(self.stations[self.station_idx_by_id(other_sid).unwrap()].tile_idx, map.width);
                manhattan(sx as i32, sy as i32, ox as i32, oy as i32).pow(2)
            };
            if connection_available && dist_sq > TRAIN_STATION_MIN_RANGE_SQ {
                pathfind_attempts += 1;
                let path = find_rail_tile_path(map, stile, self.stations[other_i].tile_idx, scratch);
                if !path.is_empty() && path.len() < RAILROAD_MAX_SIZE {
                    self.connect_rail_record(map, sid, other_sid, path);
                    edited_roots.insert(neighbor_cluster_root);
                }
                if pathfind_attempts >= 5 {
                    break;
                }
            }
        }

        if edited_roots.len() > 1 {
            // already unioned by connect_rail_record
        } else if edited_roots.is_empty() {
            // singleton cluster: union self only already parent
            let _ = new_station_idx;
        }
    }

    /// OpenFront `RailNetworkImpl.connectStation`.
    pub fn connect_station(&mut self, game: &GameState, station_idx: usize, scratch: &mut RailPathScratch) {
        let map = &game.map;
        if !self.try_snap_station_to_existing_rails(map, station_idx) {
            self.try_connect_nearby_stations(game, station_idx, scratch);
        }
    }

    fn push_runner(&mut self, world_seed: u64, building_id: u64, spawn_trains: bool) {
        if !self.registered_runners.insert(building_id) {
            return;
        }
        let seed = world_seed
            .wrapping_add(building_id)
            .wrapping_add(if spawn_trains { 0xC001_FAC7 } else { 0x0051_A710 });
        self.runners.push(StationRunner {
            building_id,
            spawn_trains,
            activated: false,
            last_spawn_tick: 0,
            spawn_rng: WyRand::seed_from_u64(seed),
        });
    }

    /// Called when a structure finishes construction (`StructureReady`).
    pub fn on_structure_ready(
        &mut self,
        map: &GameMap,
        world_seed: u64,
        buildings: &[Building],
        b: &Building,
    ) {
        if b.under_construction {
            return;
        }
        match b.kind {
            BuildingKind::Factory => {
                self.push_runner(world_seed, b.id, true);
                self.pending_factory_neighbor_factories.push(b.id);
            }
            BuildingKind::City | BuildingKind::Port => {
                if has_factory_within_range(map, buildings, b.tile_idx, TRAIN_STATION_MAX_RANGE) {
                    self.push_runner(world_seed, b.id, false);
                }
            }
            _ => {}
        }
    }

    fn activate_runner(&mut self, game: &mut GameState, buildings: &[Building], runner_i: usize, scratch: &mut RailPathScratch) {
        let building_id = self.runners[runner_i].building_id;
        let Some(b) = buildings.iter().find(|x| x.id == building_id) else {
            return;
        };
        if b.under_construction {
            return;
        }
        if !matches!(
            b.kind,
            BuildingKind::City | BuildingKind::Port | BuildingKind::Factory
        ) {
            return;
        }
        if self.station_index_by_building.contains_key(&building_id) {
            self.runners[runner_i].activated = true;
            return;
        }

        let sid = self.next_station_id;
        self.next_station_id += 1;
        let si = self.stations.len();
        self.stations.push(Station {
            id: sid,
            building_id: b.id,
            owner_id: b.owner_id,
            tile_idx: b.tile_idx,
            kind: b.kind,
            neighbors: BTreeMap::new(),
        });
        self.station_index_by_building.insert(b.id, si);
        self.uf.push_station();
        self.connect_station(game, si, scratch);
        self.runners[runner_i].activated = true;
    }

    pub fn bootstrap_factory_neighbors(&mut self, game: &mut GameState, buildings: &[Building]) {
        let pending = std::mem::take(&mut self.pending_factory_neighbor_factories);
        for fid in pending {
            let Some(fb) = buildings.iter().find(|b| b.id == fid && b.kind == BuildingKind::Factory && !b.under_construction) else {
                continue;
            };
            let (fx, fy) = idx_xy(fb.tile_idx, game.map.width);
            for ob in buildings.iter().filter(|b| !b.under_construction) {
                if ob.id == fid {
                    continue;
                }
                if !matches!(
                    ob.kind,
                    BuildingKind::City | BuildingKind::Port | BuildingKind::Factory
                ) {
                    continue;
                }
                let (ox, oy) = idx_xy(ob.tile_idx, game.map.width);
                let d = manhattan(fx as i32, fy as i32, ox as i32, oy as i32);
                if d > TRAIN_STATION_MAX_RANGE {
                    continue;
                }
                if self.station_index_by_building.contains_key(&ob.id) {
                    continue;
                }
                self.push_runner(game.seed, ob.id, false);
            }
        }
    }

    fn eligible_trade_destinations(
        &mut self,
        train_owner: u16,
        factory_si: usize,
        trade_stations_by_root: &std::collections::BTreeMap<usize, Vec<usize>>,
    ) -> Vec<u32> {
        let root = self.uf.find(factory_si);
        let mut v = Vec::new();
        if let Some(stations) = trade_stations_by_root.get(&root) {
            for &i in stations {
                if Self::trade_available(self.stations[i].owner_id, train_owner) {
                    v.push(self.stations[i].id);
                }
            }
        }
        v.sort();
        v
    }

    fn reservoir_pick_destination(rng: &mut WyRand, eligible: &[u32]) -> Option<u32> {
        if eligible.is_empty() {
            return None;
        }
        let mut selected = eligible[0];
        let mut seen = 1u32;
        for &sid in eligible.iter().skip(1) {
            seen += 1;
            if rng.next_int(0, seen as i32) == 0 {
                selected = sid;
            }
        }
        Some(selected)
    }

    fn has_any_trade_destination(
        &mut self,
        train_owner: u16,
        factory_si: usize,
        trade_stations_by_root: &std::collections::BTreeMap<usize, Vec<usize>>,
    ) -> bool {
        let root = self.uf.find(factory_si);
        if let Some(stations) = trade_stations_by_root.get(&root) {
            stations.iter().any(|&i| {
                Self::trade_available(self.stations[i].owner_id, train_owner)
            })
        } else {
            false
        }
    }

    fn try_spawn_train_for_runner(
        &mut self,
        game: &GameState,
        buildings: &[Building],
        runner_i: usize,
        current_tick: u64,
        ready_factories_by_owner: &std::collections::BTreeMap<u16, u32>,
        trade_stations_by_root: &std::collections::BTreeMap<usize, Vec<usize>>,
    ) {
        if !self.runners[runner_i].spawn_trains || !self.runners[runner_i].activated {
            return;
        }
        if current_tick
            < self.runners[runner_i].last_spawn_tick + TRAIN_SPAWN_COOLDOWN_TICKS
        {
            return;
        }
        let building_id = self.runners[runner_i].building_id;
        let Some(b) = buildings.iter().find(|x| x.id == building_id) else {
            return;
        };
        if b.kind != BuildingKind::Factory || b.under_construction {
            return;
        }
        let Some(&factory_si) = self.station_index_by_building.get(&building_id) else {
            return;
        };
        let owner = b.owner_id;
        if !self.has_any_trade_destination(owner, factory_si, trade_stations_by_root) {
            return;
        }
        let num_factories = ready_factories_by_owner.get(&owner).copied().unwrap_or(0);
        let level = b.level.max(1);
        let rate = Self::train_spawn_rate(num_factories);
        let spawn_ok = {
            let runner = &mut self.runners[runner_i];
            let mut ok = false;
            for _ in 0..level {
                if runner.spawn_rng.chance(rate) {
                    ok = true;
                    break;
                }
            }
            ok
        };
        if !spawn_ok {
            return;
        }
        let eligible = self.eligible_trade_destinations(owner, factory_si, trade_stations_by_root);
        let dest_sid = {
            let runner = &mut self.runners[runner_i];
            match Self::reservoir_pick_destination(&mut runner.spawn_rng, &eligible) {
                Some(d) => d,
                None => return,
            }
        };
        let source_sid = self.stations[factory_si].id;
        if dest_sid == source_sid {
            return;
        }
        let path = {
            let stations = &self.stations;
            let uf = &mut self.uf;
            find_stations_path(&game.map, stations, uf, source_sid, dest_sid)
        };
        let Some(path) = path else { return };
        if path.len() <= 1 {
            return;
        }
        let train_id = self.next_train_id;
        self.next_train_id = self.next_train_id.wrapping_add(1);
        self.trains.push(TrainRuntime {
            id: train_id,
            active: true,
            owner_id: owner,
            path_stations: path,
            current_leg: 0,
            idx_in_leg: 0,
            trade_stops_visited: 0,
            used_tiles_ring: std::collections::VecDeque::new(),
        });
        self.runners[runner_i].last_spawn_tick = current_tick;
    }

    fn get_rail_by_stations(&self, from_sid: u32, to_sid: u32) -> Option<&Railroad> {
        let from_i = self.stations.iter().position(|s| s.id == from_sid)?;
        let rail_id = self.stations[from_i].neighbors.get(&to_sid)?;
        self.railroads.iter().find(|r| r.id == *rail_id)
    }

    fn oriented_leg_tile(&self, rail: &Railroad, from_sid: u32, idx: usize) -> Option<u32> {
        if idx >= rail.tiles.len() {
            return None;
        }
        let mapped_idx = if rail.from_station != from_sid {
            rail.tiles.len() - 1 - idx
        } else {
            idx
        };
        Some(rail.tiles[mapped_idx])
    }

    fn next_leg_trade_ok(&self, train: &TrainRuntime) -> bool {
        if train.path_stations.len() < train.current_leg + 2 {
            return false;
        }
        let next_sid = train.path_stations[train.current_leg + 1];
        let si = match self.station_idx_by_id(next_sid) {
            Some(i) => i,
            None => return false,
        };
        Self::trade_available(self.stations[si].owner_id, train.owner_id)
    }

    fn apply_station_stop_gold(&self, game: &mut GameState, train: &TrainRuntime, station_sid: u32) {
        let si = match self.station_idx_by_id(station_sid) {
            Some(i) => i,
            None => return,
        };
        let kind = self.stations[si].kind;
        if !matches!(kind, BuildingKind::City | BuildingKind::Port) {
            return;
        }
        let station_owner = self.stations[si].owner_id;
        let train_owner = train.owner_id;
        let rel_self = station_owner == train_owner;
        let gold = Self::train_gold(
            rel_self,
            train.trade_stops_visited,
            game.config.train_gold_multiplier,
        );
        if train_owner != station_owner && let Some(so) = game.player_mut(station_owner) {
            so.gold += gold;
        }
        if let Some(to) = game.player_mut(train_owner) {
            to.gold += gold;
        }
    }

    fn tick_one_train(&mut self, game: &mut GameState, train: &mut TrainRuntime) {
        if !train.active {
            return;
        }
        if !self.next_leg_trade_ok(train) {
            train.active = false;
            return;
        }

        let mut steps = TRAIN_SPEED;
        while steps > 0 {
            if !train.active {
                break;
            }
            let from_sid = train.path_stations[train.current_leg];
            let to_sid = train.path_stations[train.current_leg + 1];
            let rail = match self.get_rail_by_stations(from_sid, to_sid) {
                Some(r) if !r.tiles.is_empty() => r,
                _ => {
                    train.active = false;
                    return;
                }
            };
            let tiles_len = rail.tiles.len();

            let remaining = tiles_len.saturating_sub(train.idx_in_leg);
            if remaining == 0 {
                // Should not happen, but safety guard
                train.current_leg += 1;
                train.idx_in_leg = 0;
                if train.current_leg + 1 >= train.path_stations.len() {
                    train.active = false;
                    return;
                }
                continue;
            }

            let take = remaining.min(steps);
            
            // Save traversed tiles into ring buffer without allocations
            for i in train.idx_in_leg..(train.idx_in_leg + take) {
                if let Some(tile) = self.oriented_leg_tile(rail, from_sid, i) {
                    train.used_tiles_ring.push_back(tile);
                }
            }
            let cap = (TRAIN_NUM_CARS + 2) * TRAIN_SPACING + 3;
            while train.used_tiles_ring.len() > cap {
                train.used_tiles_ring.pop_front();
            }

            train.idx_in_leg += take;
            steps -= take;

            if train.idx_in_leg >= tiles_len {
                let reached = to_sid;
                if let Some(si) = self.station_idx_by_id(reached) {
                    let kind = self.stations[si].kind;
                    if matches!(kind, BuildingKind::City | BuildingKind::Port) {
                        self.apply_station_stop_gold(game, train, reached);
                        train.trade_stops_visited = train.trade_stops_visited.saturating_add(1);
                    }
                }
                train.current_leg += 1;
                train.idx_in_leg = 0;
                if train.current_leg + 1 >= train.path_stations.len() {
                    train.active = false;
                    return;
                }
                if !self.next_leg_trade_ok(train) {
                    train.active = false;
                    return;
                }
            }
        }
    }

    fn tick_trains(&mut self, game: &mut GameState) {
        let mut trains = core::mem::take(&mut self.trains);
        for train in &mut trains {
            if train.active {
                self.tick_one_train(game, train);
            }
        }
        trains.retain(|t| t.active);
        self.trains = trains;
    }

    /// Activate runners, bootstrap factory neighbors, spawn trains, advance trains.
    pub fn simulation_tick(&mut self, game: &mut GameState, buildings: &[Building], scratch: &mut RailPathScratch) {
        if game.phase != crate::game::GamePhase::Playing {
            return;
        }
        let tick = game.tick;
        self.bootstrap_factory_neighbors(game, buildings);
        let n_runners = self.runners.len();
        
        let mut ready_factories_by_owner = std::collections::BTreeMap::new();
        for b in buildings {
            if b.kind == BuildingKind::Factory && !b.under_construction {
                *ready_factories_by_owner.entry(b.owner_id).or_insert(0) += 1;
            }
        }

        let mut trade_stations_by_root: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
        for i in 0..self.stations.len() {
            if matches!(self.stations[i].kind, BuildingKind::City | BuildingKind::Port) {
                let root = self.uf.find(i);
                trade_stations_by_root.entry(root).or_default().push(i);
            }
        }

        let mut activations_this_tick = 0;
        for i in 0..n_runners {
            if !self.runners[i].activated {
                self.activate_runner(game, buildings, i, scratch);
                activations_this_tick += 1;
                if activations_this_tick >= 5 {
                    break;
                }
            }
        }
        for i in 0..n_runners {
            self.try_spawn_train_for_runner(game, buildings, i, tick, &ready_factories_by_owner, &trade_stations_by_root);
        }
        self.tick_trains(game);
    }
}

/// Runs rail simulation without holding `game.rail` while mutably borrowing `game`.
pub fn run_rail_simulation(game: &mut GameState, buildings: &[Building], scratch: &mut RailPathScratch) {
    let mut rail = core::mem::take(&mut game.rail);
    rail.simulation_tick(game, buildings, scratch);
    game.rail = rail;
}

fn has_factory_within_range(
    map: &GameMap,
    buildings: &[Building],
    tile_idx: u32,
    max_range: i32,
) -> bool {
    let (tx, ty) = idx_xy(tile_idx, map.width);
    buildings.iter().any(|b| {
        b.kind == BuildingKind::Factory
            && !b.under_construction
            && {
                let (ox, oy) = idx_xy(b.tile_idx, map.width);
                manhattan(tx as i32, ty as i32, ox as i32, oy as i32) <= max_range
            }
    })
}

#[cfg(test)]
mod railway_tests {
    use super::*;
    use crate::game::{GamePhase, GameState};
    use crate::game_config::GameConfig;

    fn playing_game(w: u32, h: u32) -> GameState {
        let mut g = GameState::new(0xC0FFEE, w, h, GameConfig::default());
        g.phase = GamePhase::Playing;
        g
    }

    #[test]
    fn rail_tile_path_on_flat_land() {
        let g = playing_game(14, 14);
        let goal = 13 + 13 * 14;
        let mut scratch = RailPathScratch::default();
        let p = find_rail_tile_path(&g.map, 0, goal, &mut scratch);
        assert!(p.len() >= 2);
        assert!(p.len() < RAILROAD_MAX_SIZE);
        assert_eq!(*p.first().unwrap(), 0);
        assert_eq!(*p.last().unwrap(), goal);
    }

    #[test]
    fn factory_and_city_in_range_create_stations_and_rail() {
        let mut g = playing_game(40, 40);
        let factory = Building {
            id: 1,
            owner_id: 1,
            tile_idx: 10 + 10 * 40,
            kind: BuildingKind::Factory,
            level: 2,
            under_construction: false,
            ticks_until_complete: 0,
        };
        // > trainStationMinRange (15) tiles apart so OpenFront allows a new rail link.
        let city = Building {
            id: 2,
            owner_id: 1,
            tile_idx: 10 + 28 * 40,
            kind: BuildingKind::City,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let both = vec![factory, city];
        let map_ref = &g.map;
        let seed = g.seed;
        g.rail.on_structure_ready(map_ref, seed, &both, &both[0]);
        g.rail.on_structure_ready(map_ref, seed, &both, &both[1]);
        let mut scratch = RailPathScratch::default();
        run_rail_simulation(&mut g, &both, &mut scratch);
        run_rail_simulation(&mut g, &both, &mut scratch);
        assert_eq!(g.rail.stations.len(), 2);
        assert!(!g.rail.railroads.is_empty());
    }

    #[test]
    fn parallel_games_identical_rail_after_ticks() {
        let mut a = playing_game(40, 40);
        let mut b = playing_game(40, 40);
        let factory = Building {
            id: 1,
            owner_id: 1,
            tile_idx: 8 + 8 * 40,
            kind: BuildingKind::Factory,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let port = Building {
            id: 2,
            owner_id: 1,
            tile_idx: 8 + 19 * 40,
            kind: BuildingKind::Port,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let bs = vec![factory, port];
        {
            let m = &a.map;
            let s = a.seed;
            a.rail.on_structure_ready(m, s, &bs, &bs[0]);
            a.rail.on_structure_ready(m, s, &bs, &bs[1]);
        }
        {
            let m = &b.map;
            let s = b.seed;
            b.rail.on_structure_ready(m, s, &bs, &bs[0]);
            b.rail.on_structure_ready(m, s, &bs, &bs[1]);
        }
        let mut scratch = RailPathScratch::default();
        for _ in 0..8 {
            run_rail_simulation(&mut a, &bs, &mut scratch);
            run_rail_simulation(&mut b, &bs, &mut scratch);
        }
        assert_eq!(a.rail, b.rail);
    }
}