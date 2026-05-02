use crate::map::{GameMap, TerrainType};
use crate::game::BuildingKind;
use super::core::BuildingGrid;
/// OpenFront `structureMinDist()` / search radius for land valid tiles.
pub const STRUCTURE_MIN_DIST: i32 = 15;
const STRUCTURE_MIN_DIST_SQ: i64 = (STRUCTURE_MIN_DIST as i64) * (STRUCTURE_MIN_DIST as i64);
const STRUCTURE_SEARCH_RADIUS_SQ: i64 = STRUCTURE_MIN_DIST_SQ;

/// OpenFront `radiusPortSpawn()`.
pub const PORT_SPAWN_MANHATTAN: i32 = 20;

#[inline]
pub fn idx_xy(idx: u32, w: u32) -> (u32, u32) {
    (idx % w, idx / w)
}

#[inline]
pub fn xy_idx(x: u32, y: u32, w: u32) -> u32 {
    y * w + x
}

#[inline]
pub fn euclid_sq(ax: i64, ay: i64, bx: i64, by: i64) -> i64 {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy
}

#[inline]
pub fn manhattan(ax: i32, ay: i32, bx: i32, by: i32) -> i32 {
    (ax - bx).abs() + (ay - by).abs()
}

/// Land shoreline tile: land terrain with shoreline bit (matches fleet/shore tests).
pub fn is_shore_land_tile(map: &GameMap, x: u32, y: u32) -> bool {
    let t = map.terrain[map.ref_id(x, y)];
    t.is_land() && t.is_shoreline()
}

fn is_land_structure_tile(map: &GameMap, x: u32, y: u32) -> bool {
    matches!(
        map.terrain_type(x, y),
        TerrainType::Land | TerrainType::Highland | TerrainType::Mountain
    )
}

/// Tiles within Euclidean 15 of `click_idx`, 4-connected, owned by `owner_id`, excluding tiles
/// within Euclidean 15 of any existing structure (OpenFront `validStructureSpawnTiles`).
pub fn valid_land_structure_indices(
    map: &GameMap,
    owner_id: u16,
    click_idx: u32,
    existing: &BuildingGrid,
    scratch: &mut crate::engine::PlacementScratch,
) -> Vec<u32> {
    let w = map.width;
    let (cx, cy) = idx_xy(click_idx, w);
    if !is_land_structure_tile(map, cx, cy) {
        return Vec::new();
    }
    if map.owner_id(cx, cy) != owner_id {
        return Vec::new();
    }

    let cx_i = cx as i64;
    let cy_i = cy as i64;
    let h = map.height;
    let area = (w * h) as usize;
    if scratch.visited_stamp.len() < area {
        scratch.visited_stamp.resize(area, 0);
    }
    scratch.stamp = scratch.stamp.wrapping_add(1);
    if scratch.stamp == 0 {
        scratch.visited_stamp.fill(0);
        scratch.stamp = 1;
    }
    let stamp = scratch.stamp;
    
    scratch.queue.clear();
    scratch.visited_stamp[click_idx as usize] = stamp;
    scratch.queue.push(click_idx);

    let mut out: Vec<u32> = Vec::new();

    let mut qi = 0usize;
    while qi < scratch.queue.len() {
        let idx = scratch.queue[qi];
        qi += 1;
        let (x, y) = idx_xy(idx, w);
        let xi = x as i64;
        let yi = y as i64;
        if euclid_sq(xi, yi, cx_i, cy_i) >= STRUCTURE_SEARCH_RADIUS_SQ {
            continue;
        }
        if !is_land_structure_tile(map, x, y) {
            continue;
        }
        if map.owner_id(x, y) != owner_id {
            continue;
        }

        let mut too_close = false;
        for (bx, by) in existing.iter_in_range(x, y, STRUCTURE_MIN_DIST as u32) {
            if euclid_sq(xi, yi, bx as i64, by as i64) < STRUCTURE_MIN_DIST_SQ {
                too_close = true;
                break;
            }
        }
        if !too_close {
            out.push(idx);
        }

        map.for_each_neighbor(x, y, |nx, ny| {
            let nidx = xy_idx(nx, ny, w) as usize;
            if scratch.visited_stamp[nidx] == stamp {
                return;
            }
            let nxi = nx as i64;
            let nyi = ny as i64;
            if euclid_sq(nxi, nyi, cx_i, cy_i) >= STRUCTURE_SEARCH_RADIUS_SQ {
                return;
            }
            if map.owner_id(nx, ny) != owner_id {
                return;
            }
            scratch.visited_stamp[nidx] = stamp;
            scratch.queue.push(nidx as u32);
        });
    }

    out.sort_by(|&a, &b| {
        let (ax, ay) = idx_xy(a, w);
        let (bx, by) = idx_xy(b, w);
        let da = euclid_sq(ax as i64, ay as i64, cx_i, cy_i);
        let db = euclid_sq(bx as i64, by as i64, cx_i, cy_i);
        da.cmp(&db).then_with(|| a.cmp(&b))
    });
    out
}

/// OpenFront `portSpawn`: closest owned shore (Manhattan) within `PORT_SPAWN_MANHATTAN` of click
/// that lies in `valid_land` (as tile indices).
pub fn resolve_port_spawn_tile(
    map: &GameMap,
    owner_id: u16,
    click_idx: u32,
    valid_land: &[u32],
) -> Option<u32> {
    let w = map.width;
    let h = map.height;
    let (cx, cy) = idx_xy(click_idx, w);

    let mut candidates: Vec<u32> = Vec::new();
    let r = PORT_SPAWN_MANHATTAN;
    let cx_i = cx as i32;
    let cy_i = cy as i32;
    let x_min = (cx_i - r).clamp(0, w.saturating_sub(1) as i32) as u32;
    let x_max = (cx_i + r).clamp(0, w.saturating_sub(1) as i32) as u32;
    let y_min = (cy_i - r).clamp(0, h.saturating_sub(1) as i32) as u32;
    let y_max = (cy_i + r).clamp(0, h.saturating_sub(1) as i32) as u32;
    for y in y_min..=y_max {
        for x in x_min..=x_max {
            if manhattan(x as i32, y as i32, cx_i, cy_i) > r {
                continue;
            }
            if map.owner_id(x, y) != owner_id {
                continue;
            }
            if !is_shore_land_tile(map, x, y) {
                continue;
            }
            candidates.push(xy_idx(x, y, w));
        }
    }

    candidates.sort_by(|&a, &b| {
        let (ax, ay) = idx_xy(a, w);
        let (bx, by) = idx_xy(b, w);
        let da = manhattan(ax as i32, ay as i32, cx as i32, cy as i32);
        let db = manhattan(bx as i32, by as i32, cx as i32, cy as i32);
        da.cmp(&db).then_with(|| a.cmp(&b))
    });

    candidates
        .into_iter()
        .find(|idx| valid_land.binary_search(idx).is_ok())
}

/// Resolve final spawn tile index for `kind` at `click_idx`, or `None` if illegal.
pub fn resolve_structure_spawn_tile(
    map: &GameMap,
    owner_id: u16,
    kind: BuildingKind,
    click_idx: u32,
    existing: &BuildingGrid,
    scratch: &mut crate::engine::PlacementScratch,
) -> Option<u32> {
    let w = map.width;
    let h = map.height;
    let max_idx = w.saturating_mul(h);
    if click_idx >= max_idx {
        return None;
    }

    let valid = valid_land_structure_indices(map, owner_id, click_idx, existing, scratch);
    if valid.is_empty() {
        return None;
    }

    match kind {
        BuildingKind::Port => resolve_port_spawn_tile(map, owner_id, click_idx, &valid),
        _ => valid.first().copied(),
    }
}
