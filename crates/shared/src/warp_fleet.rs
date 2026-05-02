//! OpenFront-style transport ship as `WarpFleet` (water path + landing).
//!
//! The launch decision is gated by [`WaterComponents`]: a boat is only viable when
//! the sender owns a shoreline tile on the **same connected water body** as the
//! target's shoreline. This mirrors OpenFront's `SpatialQuery.closestShoreByWater`
//! + `WaterManager.getWaterComponent` pair; it correctly handles lakes, rivers,
//! and disjoint oceans instead of relying on an arbitrary Manhattan radius.

use std::collections::VecDeque;
use std::fmt;


use crate::map::{GameMap, TerrainType};
use crate::pathfinding::WaterPathfinderScratch;
use crate::water_components::WaterComponents;

/// Resolved water-only route for a transport launch (spawn shore → landing shore).
#[derive(Debug, Clone, PartialEq)]
pub struct FleetRoute {
    pub src_tile: u32,
    pub landing_tile: u32,
    pub path: Vec<u32>,
}

/// Typed failure for fleet routing (client preflight + sim must share this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetLaunchError {
    /// `target_tile` out of map bounds.
    InvalidTile,
    /// `target_owner == player_id`.
    SelfTarget,
    /// Launcher has no shoreline on any water component.
    NoWaterAccess,
    /// Enemy/neutral target requires a player row that is missing.
    TargetPlayerNotFound { target_owner: u16 },
    /// No landing shoreline reachable on shared water components.
    NoLandingShore,
    /// No owned launch shore on the landing tile's water component.
    NoLaunchShore { component: u32 },
    /// Water A* found no path between `src_tile` and `landing_tile`.
    NoWaterPath,
    /// No **ready** Port — OpenFront requires a port before transport launches.
    NoPort,
}

impl fmt::Display for FleetLaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FleetLaunchError::InvalidTile => write!(f, "invalid target tile"),
            FleetLaunchError::SelfTarget => write!(f, "cannot boat to own tile"),
            FleetLaunchError::NoWaterAccess => write!(f, "no shoreline water access"),
            FleetLaunchError::TargetPlayerNotFound { target_owner } => {
                write!(f, "target player {target_owner} not found")
            }
            FleetLaunchError::NoLandingShore => {
                write!(f, "no landing shore on shared water body")
            }
            FleetLaunchError::NoLaunchShore { component } => {
                write!(f, "no launch shore on water component {component}")
            }
            FleetLaunchError::NoWaterPath => write!(f, "no water path to landing"),
            FleetLaunchError::NoPort => write!(f, "fleet requires a completed port"),
        }
    }
}

/// Shared route resolution for `LaunchFleet` — used by simulation and client preflight
/// so right-click / KeyB cannot diverge from `apply_launch_fleet_intent`.
///
/// `target_border`: required when `target_owner != 0` (enemy player's `border_tiles`);
/// ignored for neutral (`target_owner == 0`).
pub fn resolve_fleet_route(
    map: &GameMap,
    water_components: &WaterComponents,
    path_scratch: &mut WaterPathfinderScratch,
    player_id: u16,
    target_owner: u16,
    target_tile: u32,
    border_tiles: &crate::bitset::DenseBitSet,
    target_border: Option<&crate::bitset::DenseBitSet>,
    has_completed_port: bool,
) -> Result<FleetRoute, FleetLaunchError> {
    let map_area = map.width.saturating_mul(map.height);
    if map_area == 0 || target_tile >= map_area {
        return Err(FleetLaunchError::InvalidTile);
    }
    if target_owner == player_id {
        return Err(FleetLaunchError::SelfTarget);
    }
    if !has_completed_port {
        return Err(FleetLaunchError::NoPort);
    }

    let my_comps = player_water_components(map, water_components, player_id, border_tiles);
    if my_comps.is_empty() {
        return Err(FleetLaunchError::NoWaterAccess);
    }

    let landing = if target_owner != 0 {
        let tb = target_border.ok_or(FleetLaunchError::TargetPlayerNotFound {
            target_owner,
        })?;
        closest_target_shore_for_player(
            map,
            water_components,
            &my_comps,
            target_owner,
            tb,
            target_tile,
        )
    } else {
        closest_neutral_shore_on_components(
            map,
            water_components,
            &my_comps,
            target_tile,
            200,
            &mut path_scratch.bfs_queue,
            &mut path_scratch.bfs_visited,
            &mut path_scratch.bfs_stamp,
        )
    };
    let landing = landing.ok_or(FleetLaunchError::NoLandingShore)?;

    let landing_comp = water_components.component_of(landing);
    let src = best_shore_spawn_for_transport(
        map,
        player_id,
        border_tiles,
        landing,
        Some((water_components, landing_comp)),
    )
    .ok_or(FleetLaunchError::NoLaunchShore {
        component: landing_comp,
    })?;

    let path = path_scratch
        .astar
        .find_path(map, &[src], landing)
        .ok_or(FleetLaunchError::NoWaterPath)?;

    Ok(FleetRoute {
        src_tile: src,
        landing_tile: landing,
        path,
    })
}

/// Collect the set of water components this player can launch from (deduplicated,
/// sorted ascending for determinism). Empty iff the player owns no shore adjacent
/// to any water tile — the OpenFront "cannot build TransportShip" condition.
pub fn player_water_components(
    map: &GameMap,
    components: &WaterComponents,
    player_id: u16,
    border_tiles: &crate::bitset::DenseBitSet,
) -> Vec<u32> {
    let w = map.width;
    let mut out: Vec<u32> = Vec::new();
    for idx in border_tiles.ones() {
        let x = idx % w;
        let y = idx / w;
        if map.owner_id(x, y) != player_id {
            continue;
        }
        let t = map.terrain[idx as usize];
        if !t.is_land() || !t.is_shoreline() {
            continue;
        }
        let c = components.component_of(idx);
        if c == 0 {
            continue;
        }
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out.sort_unstable();
    out
}

/// OpenFront `canBuildTransportShip` gate: does this player have any shoreline
/// on at least one water component? Kept as a convenience wrapper around
/// [`player_water_components`] for callers that only need a boolean.
pub fn player_has_water_access(
    map: &GameMap,
    components: &WaterComponents,
    player_id: u16,
    border_tiles: &crate::bitset::DenseBitSet,
) -> bool {
    let w = map.width;
    for idx in border_tiles.ones() {
        let x = idx % w;
        let y = idx / w;
        if map.owner_id(x, y) != player_id {
            continue;
        }
        let t = map.terrain[idx as usize];
        if !t.is_land() || !t.is_shoreline() {
            continue;
        }
        if components.component_of(idx) != 0 {
            return true;
        }
    }
    false
}

/// OpenFront `Player.sharesBorderWith(other)`: scan my border tiles; return true if **any**
/// 4-neighbor is owned by `target_owner`. This is the correct gate for a land `Attack`
/// intent — a click on a far-away enemy interior still ground-attacks as long as we
/// touch them somewhere on the map.
pub fn shares_border_with(map: &GameMap, border_tiles: &crate::bitset::DenseBitSet, target_owner: u16) -> bool {
    let w = map.width;
    for idx in border_tiles.ones() {
        let x = idx % w;
        let y = idx / w;
        let mut found = false;
        map.for_each_neighbor(x, y, |nx, ny| {
            if !found && map.owner_id(nx, ny) == target_owner {
                found = true;
            }
        });
        if found {
            return true;
        }
    }
    false
}

/// OpenFront `canAttack` neutral branch (`PlayerImpl.canAttack`, TerraNullius case):
/// BFS from `click_tile` through contiguous neutral-land tiles (Manhattan ≤ 200).
/// Returns `true` if any visited tile has a 4-neighbor owned by `player_id` —
/// i.e. the clicked patch of no-man's-land is walk-adjacent to my territory.
///
/// Uses stamp-based scratch buffers to avoid per-click allocations; `queue` is
/// cleared on entry. N/S/W/E neighbor order matches `closest_neutral_shore_on_components`.
pub fn can_ground_attack_neutral(
    map: &GameMap,
    player_id: u16,
    click_tile: u32,
    max_dist: u32,
    queue: &mut VecDeque<u32>,
    visited: &mut Vec<u32>,
    visit_stamp: &mut u32,
) -> bool {
    let w = map.width;
    let h = map.height;
    let area = w.saturating_mul(h);
    if area == 0 || click_tile >= area {
        return false;
    }
    let cx = click_tile % w;
    let cy = click_tile / w;
    let click_t = map.terrain[click_tile as usize];
    if !click_t.is_land() || map.owner_id(cx, cy) != 0 {
        return false;
    }

    let need = area as usize;
    if visited.len() < need {
        visited.resize(need, 0);
    }
    *visit_stamp = visit_stamp.wrapping_add(1);
    if *visit_stamp == 0 {
        visited.fill(0);
        *visit_stamp = 1;
    }
    let stamp = *visit_stamp;

    queue.clear();
    visited[click_tile as usize] = stamp;
    queue.push_back(click_tile);

    while let Some(idx) = queue.pop_front() {
        let x = idx % w;
        let y = idx / w;
        // Found if any 4-neighbor is owned by me.
        let mut hit = false;
        map.for_each_neighbor(x, y, |nx, ny| {
            if !hit && map.owner_id(nx, ny) == player_id {
                hit = true;
            }
        });
        if hit {
            return true;
        }

        let neighbors = [
            y.checked_sub(1).map(|ny| (x, ny)),
            if y + 1 < h { Some((x, y + 1)) } else { None },
            x.checked_sub(1).map(|nx| (nx, y)),
            if x + 1 < w { Some((x + 1, y)) } else { None },
        ];
        for opt in neighbors.into_iter().flatten() {
            let (nx, ny) = opt;
            if cx.abs_diff(nx) + cy.abs_diff(ny) > max_dist {
                continue;
            }
            let nidx = ny * w + nx;
            let vi = nidx as usize;
            if visited[vi] == stamp {
                continue;
            }
            let nt = map.terrain[vi];
            if !nt.is_land() || map.owner_id(nx, ny) != 0 {
                continue;
            }
            visited[vi] = stamp;
            queue.push_back(nidx);
        }
    }
    false
}

#[inline]
fn manhattan_idx(w: u32, a: u32, b: u32) -> u32 {
    let ax = a % w;
    let ay = a / w;
    let bx = b % w;
    let by = b / w;
    ax.abs_diff(bx) + ay.abs_diff(by)
}

#[inline]
fn components_match(my_comps: &[u32], c: u32) -> bool {
    if c == 0 {
        return false;
    }
    my_comps.binary_search(&c).is_ok()
}

/// Resolve the landing tile for a player-owned target (OpenFront parity with
/// `closestShore(owner, click, 50)` + `closestShoreByWater` component check, but
/// with no arbitrary Manhattan radius — connectivity is the gate).
///
/// Iterates the target player's cached `border_tiles` (O(perimeter)), keeps only
/// shoreline tiles on a water component the caller can reach, returns the one
/// closest (Manhattan) to `click_tile`. Deterministic tie-break: smallest index.
pub fn closest_target_shore_for_player(
    map: &GameMap,
    components: &WaterComponents,
    my_water_comps: &[u32],
    target_owner: u16,
    target_border_tiles: &crate::bitset::DenseBitSet,
    click_tile: u32,
) -> Option<u32> {
    if my_water_comps.is_empty() {
        return None;
    }
    let w = map.width;
    let mut best: Option<(u32, u32)> = None;
    for idx in target_border_tiles.ones() {
        let x = idx % w;
        let y = idx / w;
        if map.owner_id(x, y) != target_owner {
            continue;
        }
        let t = map.terrain[idx as usize];
        if !t.is_land() || !t.is_shoreline() {
            continue;
        }
        let c = components.component_of(idx);
        if !components_match(my_water_comps, c) {
            continue;
        }
        let d = manhattan_idx(w, idx, click_tile);
        match best {
            None => best = Some((d, idx)),
            Some((bd, bi)) => {
                if d < bd || (d == bd && idx < bi) {
                    best = Some((d, idx));
                }
            }
        }
    }
    best.map(|(_, i)| i)
}

/// Resolve the landing tile for a neutral-owned target (OpenFront `TerraNullius`
/// path). We cannot iterate a player's perimeter so we BFS outward from the
/// click over 4-connected tiles, bounded by `max_dist` Manhattan, accepting any
/// neutral shoreline whose water component matches ours.
///
/// `max_dist = 200` mirrors OpenFront's `canAttack` Manhattan cap for neutral
/// (see `PlayerImpl.canAttack`); scratch buffers are reused across calls.
pub fn closest_neutral_shore_on_components(
    map: &GameMap,
    components: &WaterComponents,
    my_water_comps: &[u32],
    click_tile: u32,
    max_dist: u32,
    queue: &mut VecDeque<u32>,
    visited: &mut Vec<u32>,
    visit_stamp: &mut u32,
) -> Option<u32> {
    if my_water_comps.is_empty() {
        return None;
    }
    let w = map.width;
    let h = map.height;
    if w == 0 || h == 0 {
        return None;
    }
    let area = w * h;
    if click_tile >= area {
        return None;
    }

    let need = area as usize;
    if visited.len() < need {
        visited.resize(need, 0);
    }
    *visit_stamp = visit_stamp.wrapping_add(1);
    if *visit_stamp == 0 {
        visited.fill(0);
        *visit_stamp = 1;
    }
    let stamp = *visit_stamp;

    let cx = click_tile % w;
    let cy = click_tile / w;

    queue.clear();
    visited[click_tile as usize] = stamp;
    queue.push_back(click_tile);

    let mut best: Option<(u32, u32)> = None;

    while let Some(idx) = queue.pop_front() {
        let x = idx % w;
        let y = idx / w;
        let t = map.terrain[idx as usize];
        if t.is_land()
            && t.is_shoreline()
            && map.owner_id(x, y) == 0
            && components_match(my_water_comps, components.component_of(idx))
        {
            let d = cx.abs_diff(x) + cy.abs_diff(y);
            match best {
                None => best = Some((d, idx)),
                Some((bd, bi)) => {
                    if d < bd || (d == bd && idx < bi) {
                        best = Some((d, idx));
                    }
                }
            }
        }

        let neighbors = [
            y.checked_sub(1).map(|ny| (x, ny)),
            if y + 1 < h { Some((x, y + 1)) } else { None },
            x.checked_sub(1).map(|nx| (nx, y)),
            if x + 1 < w { Some((x + 1, y)) } else { None },
        ];
        for opt in neighbors.into_iter().flatten() {
            let (nx, ny) = opt;
            if cx.abs_diff(nx) + cy.abs_diff(ny) > max_dist {
                continue;
            }
            let nidx = ny * w + nx;
            let vi = nidx as usize;
            if visited[vi] == stamp {
                continue;
            }
            visited[vi] = stamp;
            queue.push_back(nidx);
        }
    }

    best.map(|(_, i)| i)
}

/// Troop loss fraction when a fleet returns to own shore (OpenFront `malusForRetreat = 25`).
pub const FLEET_RETREAT_SHORE_MALUS: f64 = 0.25;

/// Best owned shoreline tile, closest (Manhattan) to `reference_idx`, **and**
/// restricted to the same water component as `reference_idx` when
/// `components` is provided. This mirrors OpenFront
/// `SpatialQuery.closestShoreByWater`: a TransportShip can only launch from one
/// of my shores that actually touches the same water body as the landing.
///
/// Passing `components=None` falls back to the pure "touches water" check and
/// is only kept for the retreat path, where we want the nearest friendly shore
/// regardless of which component — a fleet can rebase on any of our coasts.
pub fn best_shore_spawn_for_transport(
    map: &GameMap,
    player_id: u16,
    border_tiles: &crate::bitset::DenseBitSet,
    reference_idx: u32,
    components: Option<(&WaterComponents, u32)>,
) -> Option<u32> {
    let w = map.width;
    let rx = reference_idx % w;
    let ry = reference_idx / w;

    let mut best: Option<(u32, u32)> = None;

    for idx in border_tiles.ones() {
        let x = idx % w;
        let y = idx / w;
        if map.owner_id(x, y) != player_id {
            continue;
        }
        let t = map.terrain[idx as usize];
        if !t.is_land() || !t.is_shoreline() {
            continue;
        }

        match components {
            Some((comps, target_comp)) => {
                if comps.component_of(idx) != target_comp {
                    continue;
                }
            }
            None => {
                let mut touches_water = false;
                map.for_each_neighbor(x, y, |nx, ny| {
                    let tt = map.terrain_type(nx, ny);
                    if tt == TerrainType::Water || tt == TerrainType::Lake {
                        touches_water = true;
                    }
                });
                if !touches_water {
                    continue;
                }
            }
        }

        let dist = rx.abs_diff(x) + ry.abs_diff(y);
        match best {
            None => best = Some((dist, idx)),
            Some((bd, bi)) => {
                if dist < bd || (dist == bd && idx < bi) {
                    best = Some((dist, idx));
                }
            }
        }
    }

    best.map(|(_, i)| i)
}

/// Moving fleet over water (1 tile / tick). Mirrors OpenFront `TransportShip` execution state.
#[derive(Debug, Clone)]
pub struct WarpFleet {
    pub id: u64,
    pub owner_id: u16,
    pub target_owner: u16,
    pub troops: f64,
    pub src_tile: u32,
    pub dst_tile: u32,
    pub retreat_dst: Option<u32>,
    pub path: Vec<u32>,
    pub path_cursor: usize,
    pub current_tile: u32,
    pub retreating: bool,
}

impl WarpFleet {
    pub fn new(
        id: u64,
        owner_id: u16,
        target_owner: u16,
        troops: f64,
        src_tile: u32,
        dst_tile: u32,
        path: Vec<u32>,
    ) -> Self {
        let current_tile = path.first().copied().unwrap_or(src_tile);
        let path_cursor = 0;
        Self {
            id,
            owner_id,
            target_owner,
            troops,
            src_tile,
            dst_tile,
            retreat_dst: None,
            path,
            path_cursor,
            current_tile,
            retreating: false,
        }
    }
}
