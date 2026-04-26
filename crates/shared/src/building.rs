//! OpenFront-style structures (City, Factory, Port, …) as ECS components.
//! Build and upgrade costs use **gold**; see `structure_build_cost_gold`.

use bevy::prelude::*;

use crate::config;
use crate::game::{BuildingKind, GameEvent, GamePhase, GameState};
use crate::map::{GameMap, TerrainType};

#[derive(Component, Debug, Clone, Copy)]
pub struct Building {
    pub id: u64,
    pub owner_id: u16,
    /// Linear index `y * width + x`.
    pub tile_idx: u32,
    pub kind: BuildingKind,
    pub level: u8,
    pub under_construction: bool,
    pub ticks_until_complete: u32,
}

/// OpenFront `structureMinDist()` / search radius for land valid tiles.
pub const STRUCTURE_MIN_DIST: i32 = 15;
const STRUCTURE_MIN_DIST_SQ: i64 = (STRUCTURE_MIN_DIST as i64) * (STRUCTURE_MIN_DIST as i64);
const STRUCTURE_SEARCH_RADIUS_SQ: i64 = STRUCTURE_MIN_DIST_SQ;

/// OpenFront `radiusPortSpawn()`.
pub const PORT_SPAWN_MANHATTAN: i32 = 20;

#[inline]
fn idx_xy(idx: u32, w: u32) -> (u32, u32) {
    (idx % w, idx / w)
}

#[inline]
fn xy_idx(x: u32, y: u32, w: u32) -> u32 {
    y * w + x
}

#[inline]
fn euclid_sq(ax: i64, ay: i64, bx: i64, by: i64) -> i64 {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy
}

#[inline]
fn manhattan(ax: i32, ay: i32, bx: i32, by: i32) -> i32 {
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
    existing: &[(u32, u32)],
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
    let mut visited = vec![false; area];
    let mut queue: Vec<u32> = Vec::new();
    visited[click_idx as usize] = true;
    queue.push(click_idx);

    let mut out: Vec<u32> = Vec::new();

    let mut qi = 0usize;
    while qi < queue.len() {
        let idx = queue[qi];
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
        for &(bx, by) in existing {
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
            if visited[nidx] {
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
            visited[nidx] = true;
            queue.push(nidx as u32);
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
    let valid_set: std::collections::HashSet<u32> = valid_land.iter().copied().collect();

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
        .find(|idx| valid_set.contains(idx))
}

/// Resolve final spawn tile index for `kind` at `click_idx`, or `None` if illegal.
pub fn resolve_structure_spawn_tile(
    map: &GameMap,
    owner_id: u16,
    kind: BuildingKind,
    click_idx: u32,
    existing: &[(u32, u32)],
) -> Option<u32> {
    let w = map.width;
    let h = map.height;
    let max_idx = w.saturating_mul(h);
    if click_idx >= max_idx {
        return None;
    }

    let valid = valid_land_structure_indices(map, owner_id, click_idx, existing);
    if valid.is_empty() {
        return None;
    }

    match kind {
        BuildingKind::Port => resolve_port_spawn_tile(map, owner_id, click_idx, &valid),
        _ => valid.first().copied(),
    }
}

/// Collect `(x,y)` for every building except `exclude_id` (use `None` when placing new).
pub fn existing_structure_positions_with_width<'a>(
    buildings: impl Iterator<Item = &'a Building>,
    map_width: u32,
    exclude_id: Option<u64>,
) -> Vec<(u32, u32)> {
    buildings
        .filter(|b| exclude_id.map(|id| b.id != id).unwrap_or(true))
        .map(|b| idx_xy(b.tile_idx, map_width))
        .collect()
}

/// OpenFront `findExistingUnitToUpgrade`: closest structure of `kind` within Manhattan
/// `STRUCTURE_MIN_DIST` of `click_idx`.
pub fn find_upgrade_target_id(
    map: &GameMap,
    owner_id: u16,
    kind: BuildingKind,
    click_idx: u32,
    buildings: &[Building],
) -> Option<u64> {
    let w = map.width;
    let (cx, cy) = idx_xy(click_idx, w);
    let mut best: Option<(i32, u64)> = None;
    for b in buildings {
        if b.owner_id != owner_id || b.kind != kind {
            continue;
        }
        if b.under_construction {
            continue;
        }
        if !kind.upgradable() {
            continue;
        }
        let (bx, by) = idx_xy(b.tile_idx, w);
        let d = manhattan(cx as i32, cy as i32, bx as i32, by as i32);
        if d > STRUCTURE_MIN_DIST {
            continue;
        }
        let cand = (d, b.id);
        match best {
            None => best = Some(cand),
            Some((bd, bid)) => {
                if d < bd || (d == bd && b.id < bid) {
                    best = Some(cand);
                }
            }
        }
    }
    best.map(|(_, id)| id)
}

/// Count buildings owned by `owner` of exact `kind` (for City-only scaling).
fn count_kind(buildings: &[Building], owner_id: u16, kind: BuildingKind) -> u32 {
    buildings
        .iter()
        .filter(|b| b.owner_id == owner_id && b.kind == kind)
        .count() as u32
}

/// Count Port + Factory for shared exponential scaling (OpenFront `costWrapper` for Port/Factory).
fn count_port_factory(buildings: &[Building], owner_id: u16) -> u32 {
    buildings
        .iter()
        .filter(|b| {
            b.owner_id == owner_id
                && matches!(b.kind, BuildingKind::Port | BuildingKind::Factory)
        })
        .count() as u32
}

#[inline]
fn scaled_pow2_cost(count: u32, base: u64, cap: u64, scale: f64) -> f64 {
    let steps = count.min(20);
    let mut val = base;
    for _ in 0..steps {
        val = val.saturating_mul(2);
        if val >= cap {
            val = cap;
            break;
        }
    }
    (val.min(cap)) as f64 / scale
}

/// Gold price for a new structure or one upgrade level (OpenFront-style scaling). Tunable.
pub fn structure_build_cost_gold(kind: BuildingKind, owner_id: u16, buildings: &[Building]) -> f64 {
    let s = config::OPENFRONT_GOLD_SCALE.max(1.0);
    match kind {
        BuildingKind::City => {
            let n = count_kind(buildings, owner_id, BuildingKind::City);
            scaled_pow2_cost(n, 125_000, 1_000_000, s)
        }
        BuildingKind::Factory | BuildingKind::Port => {
            let n = count_port_factory(buildings, owner_id);
            scaled_pow2_cost(n, 125_000, 1_000_000, s)
        }
        BuildingKind::DefensePost => {
            let n = count_kind(buildings, owner_id, BuildingKind::DefensePost);
            ((n as f64 + 1.0) * (50_000.0 / s)).min(250_000.0 / s)
        }
        BuildingKind::SamLauncher => {
            let n = count_kind(buildings, owner_id, BuildingKind::SamLauncher);
            ((n as f64 + 1.0) * (1_500_000.0 / s)).min(3_000_000.0 / s)
        }
        BuildingKind::MissileSilo => 1_000_000.0 / s,
    }
}

#[inline]
pub fn structure_kind_enabled(kind: BuildingKind) -> bool {
    match kind {
        BuildingKind::SamLauncher | BuildingKind::MissileSilo => config::ENABLE_MISSILE_STRUCTURES,
        _ => true,
    }
}

/// Sum of `level` for ready (not under construction) buildings of `kind` for `owner_id`.
pub fn count_kind_levels(buildings: &[Building], owner_id: u16, kind: BuildingKind) -> u32 {
    buildings
        .iter()
        .filter(|b| b.owner_id == owner_id && b.kind == kind && !b.under_construction)
        .map(|b| b.level as u32)
        .sum()
}

/// At least one **ready** Port owned by `player_id` (fleet launches require this).
#[inline]
pub fn player_has_completed_port(buildings: &[Building], player_id: u16) -> bool {
    buildings.iter().any(|b| {
        b.owner_id == player_id && b.kind == BuildingKind::Port && !b.under_construction
    })
}

/// Max sampled tiles per structure kind when estimating HUD `can_build` (full scan is O(map²)).
pub const PLACEMENT_HUD_MAX_SAMPLES: usize = 4096;

/// Whether `kind` can likely be placed somewhere on owned land (sampled; false negatives possible).
pub fn player_has_valid_placement_sampled(
    map: &GameMap,
    owner_id: u16,
    kind: BuildingKind,
    buildings: &[Building],
    max_samples: usize,
) -> bool {
    let w = map.width;
    let area = w.saturating_mul(map.height) as usize;
    if area == 0 {
        return false;
    }
    let positions = existing_structure_positions_with_width(buildings.iter(), w, None);
    let n = area.min(max_samples.max(1));
    let divisor = n.max(1);
    for k in 0..n {
        let idx = ((k as u64 * area as u64) / divisor as u64) as u32;
        let (x, y) = idx_xy(idx, w);
        if map.owner_id(x, y) != owner_id {
            continue;
        }
        if resolve_structure_spawn_tile(map, owner_id, kind, idx, &positions).is_some() {
            return true;
        }
    }
    false
}

/// Exhaustive scan (tests / tiny maps only).
pub fn player_has_valid_placement_scan(
    map: &GameMap,
    owner_id: u16,
    kind: BuildingKind,
    buildings: &[Building],
) -> bool {
    player_has_valid_placement_sampled(map, owner_id, kind, buildings, usize::MAX)
}

/// Per-player totals for income / fleet gates (only **ready** structures count).
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildingAggregate {
    pub city_levels: u32,
    pub factory_levels: u32,
    pub port_levels: u32,
    pub defense_levels: u32,
    /// True if any ready Port exists (same as `port_levels > 0` when ports are never level-0 ready without existing).
    pub has_completed_port: bool,
}

/// `out[v]` is aggregate for owner id `v` (resize to `max_player_id + 1`).
pub fn aggregate_buildings_per_player(
    buildings: impl Iterator<Item = Building>,
    max_player_id: usize,
) -> Vec<BuildingAggregate> {
    let mut out = vec![BuildingAggregate::default(); max_player_id.saturating_add(1)];
    for b in buildings {
        if b.under_construction {
            continue;
        }
        let i = b.owner_id as usize;
        if i >= out.len() {
            continue;
        }
        let a = &mut out[i];
        match b.kind {
            BuildingKind::City => a.city_levels += b.level as u32,
            BuildingKind::Factory => a.factory_levels += b.level as u32,
            BuildingKind::Port => {
                a.port_levels += b.level as u32;
                a.has_completed_port = true;
            }
            BuildingKind::DefensePost => a.defense_levels += b.level as u32,
            BuildingKind::SamLauncher | BuildingKind::MissileSilo => {}
        }
    }
    out
}

/// Extra frontier priority when conquering a tile near an enemy DefensePost (defender `target_owner`).
#[inline]
pub fn defense_post_priority_bonus(
    buildings: &[Building],
    defender_id: u16,
    tile_x: u32,
    tile_y: u32,
    map_width: u32,
) -> i64 {
    let mut bonus: i64 = 0;
    for b in buildings {
        if b.owner_id != defender_id || b.kind != BuildingKind::DefensePost || b.under_construction {
            continue;
        }
        let (bx, by) = idx_xy(b.tile_idx, map_width);
        let d = manhattan(tile_x as i32, tile_y as i32, bx as i32, by as i32);
        if d <= config::DEFENSE_POST_RANGE {
            bonus += b.level as i64 * config::DEFENSE_POST_PRIORITY_PER_LEVEL;
        }
    }
    bonus
}

#[derive(Clone, Copy, Debug)]
pub struct BuildableEntry {
    pub kind: BuildingKind,
    pub cost: f64,
    pub level_total: u32,
    pub count: u32,
    /// Sampled map check — independent of current gold balance.
    pub placement_feasible: bool,
    pub can_build: bool,
    pub can_upgrade: bool,
}

pub fn player_can_upgrade_kind(
    buildings: &[Building],
    owner_id: u16,
    kind: BuildingKind,
    player_gold: f64,
) -> bool {
    if !structure_kind_enabled(kind) {
        return false;
    }
    if !kind.upgradable() {
        return false;
    }
    let has_target = buildings.iter().any(|b| {
        b.owner_id == owner_id && b.kind == kind && !b.under_construction
    });
    if !has_target {
        return false;
    }
    let cost = structure_build_cost_gold(kind, owner_id, buildings);
    player_gold >= cost && cost.is_finite()
}

/// One HUD row per structure kind (OpenFront `buildableUnits` subset).
pub fn compute_buildables_for_player(
    map: &GameMap,
    owner_id: u16,
    player_gold: f64,
    buildings: &[Building],
) -> [BuildableEntry; 6] {
    let mut out = [BuildableEntry {
        kind: BuildingKind::City,
        cost: 0.0,
        level_total: 0,
        count: 0,
        placement_feasible: false,
        can_build: false,
        can_upgrade: false,
    }; 6];
    for (i, &kind) in BuildingKind::ALL.iter().enumerate() {
        let enabled = structure_kind_enabled(kind);
        let cost = structure_build_cost_gold(kind, owner_id, buildings);
        let level_total = count_kind_levels(buildings, owner_id, kind);
        let count = count_kind(buildings, owner_id, kind);
        let placement_feasible =
            player_has_valid_placement_sampled(map, owner_id, kind, buildings, PLACEMENT_HUD_MAX_SAMPLES);
        let can_build = enabled && player_gold >= cost && cost.is_finite() && placement_feasible;
        let can_upgrade = player_can_upgrade_kind(buildings, owner_id, kind, player_gold);
        out[i] = BuildableEntry {
            kind,
            cost,
            level_total,
            count,
            placement_feasible,
            can_build,
            can_upgrade,
        };
    }
    out
}

/// Refresh costs and afford flags after gold or building-count changes without re-scanning the map.
pub fn patch_buildable_entries_gold_and_counts(
    entries: &mut [BuildableEntry],
    owner_id: u16,
    player_gold: f64,
    buildings: &[Building],
) {
    for e in entries.iter_mut() {
        let kind = e.kind;
        e.cost = structure_build_cost_gold(kind, owner_id, buildings);
        e.level_total = count_kind_levels(buildings, owner_id, kind);
        e.count = count_kind(buildings, owner_id, kind);
        let afford = player_gold >= e.cost && e.cost.is_finite();
        let enabled = structure_kind_enabled(kind);
        e.can_build = enabled && afford && e.placement_feasible;
        e.can_upgrade = player_can_upgrade_kind(buildings, owner_id, kind, player_gold);
    }
}

/// Advance construction timers; emits [`GameEvent::StructureReady`].
/// Split out for unit tests (`advance_building_construction_tick`).
pub fn advance_building_construction_tick(
    game: &mut GameState,
    q: &mut Query<(Entity, &mut Building)>,
) {
    if game.phase != GamePhase::Playing {
        return;
    }

    let mut scratch: Vec<(Entity, u64)> = q.iter().map(|(e, b)| (e, b.id)).collect();
    scratch.sort_by_key(|&(_, id)| id);

    for (entity, _) in scratch {
        let Ok((_, mut b)) = q.get_mut(entity) else {
            continue;
        };
        if !b.under_construction {
            continue;
        }
        if b.ticks_until_complete > 0 {
            b.ticks_until_complete -= 1;
        }
        if b.ticks_until_complete == 0 {
            b.under_construction = false;
            game.events.push(GameEvent::StructureReady {
                id: b.id,
                tile_idx: b.tile_idx,
                kind: b.kind,
            });
        }
    }
}

/// Advance construction timers; emits [`GameEvent::StructureReady`].
pub fn system_building_construction(mut game: ResMut<GameState>, mut q: Query<(Entity, &mut Building)>) {
    advance_building_construction_tick(&mut *game, &mut q);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::World;
    use crate::game::BuildingKind;
    use crate::map::MapTile;

    fn tiny_owned_map() -> (GameMap, u16) {
        let mut m = GameMap::new(5, 1);
        let owner = 1u16;
        for x in 0..5 {
            m.set_owner_id(x, 0, owner);
            let ri = m.ref_id(x, 0);
            m.terrain[ri] = MapTile::from_byte(0b1000_0000); // land
        }
        // Shore at ends for port tests
        let r0 = m.ref_id(0, 0);
        m.terrain[r0] = MapTile::from_byte(0b1100_0000);
        let r4 = m.ref_id(4, 0);
        m.terrain[r4] = MapTile::from_byte(0b1100_0000);
        (m, owner)
    }

    #[test]
    fn valid_land_center_click() {
        let (map, owner) = tiny_owned_map();
        let existing: Vec<(u32, u32)> = Vec::new();
        let v = valid_land_structure_indices(&map, owner, 2, &existing);
        assert!(v.contains(&2));
    }

    #[test]
    fn spacing_excludes_nearby() {
        let w = 32u32;
        let mut m = GameMap::new(w, 1);
        let owner = 1u16;
        for x in 0..w {
            m.set_owner_id(x, 0, owner);
            let ri = m.ref_id(x, 0);
            m.terrain[ri] = MapTile::from_byte(0b1000_0000);
        }
        let click = 16u32;
        // Structure one tile away from click: click tile is excluded (too close).
        let existing = vec![(15u32, 0u32)];
        let v = valid_land_structure_indices(&m, owner, click, &existing);
        assert!(!v.contains(&15));
        assert!(!v.is_empty());
    }

    #[test]
    fn port_picks_shore_in_valid_set() {
        let (map, owner) = tiny_owned_map();
        let valid = valid_land_structure_indices(&map, owner, 2, &[]);
        let p = resolve_port_spawn_tile(&map, owner, 2, &valid);
        assert!(p.is_some());
        let (x, y) = idx_xy(p.unwrap(), map.width);
        assert!(is_shore_land_tile(&map, x, y));
    }

    #[test]
    fn upgrade_closest_by_manhattan() {
        let w = 20u32;
        let mut b1 = Building {
            id: 1,
            owner_id: 1,
            tile_idx: xy_idx(5, 5, w),
            kind: BuildingKind::City,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let b2 = Building {
            id: 2,
            owner_id: 1,
            tile_idx: xy_idx(6, 5, w),
            kind: BuildingKind::City,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let map = GameMap::new(w, 20);
        let click = xy_idx(5, 5, w);
        let id = find_upgrade_target_id(&map, 1, BuildingKind::City, click, &[b1, b2]);
        assert_eq!(id, Some(1));

        b1.under_construction = true;
        let id2 = find_upgrade_target_id(&map, 1, BuildingKind::City, click, &[b1, b2]);
        assert_eq!(id2, Some(2));
    }

    #[test]
    fn construction_tick_emits_structure_ready() {
        use bevy::ecs::system::SystemState;
        use crate::game::{GameEvent, GamePhase, GameState};
        use crate::player::Player;
        let mut world = World::new();
        let mut game = GameState::new(3, 5, 1);
        game.phase = GamePhase::Playing;
        let (map, owner) = tiny_owned_map();
        game.map = map;
        game.players.push(Player::new_human(owner, "c".into(), [1.0, 0.0, 0.0]));
        game.player_lookup.resize(owner as usize + 1, None);
        game.player_lookup[owner as usize] = Some(0);
        world.insert_resource(game);
        world.spawn(Building {
            id: 7,
            owner_id: owner,
            tile_idx: 2,
            kind: BuildingKind::City,
            level: 1,
            under_construction: true,
            ticks_until_complete: 1,
        });

        {
            let mut state =
                SystemState::<(ResMut<GameState>, Query<(Entity, &mut Building)>)>::new(&mut world);
            let (mut game, mut q) = state.get_mut(&mut world);
            advance_building_construction_tick(&mut game, &mut q);
        }
        world.flush();

        let game = world.resource::<GameState>();
        assert!(game
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::StructureReady { id: 7, .. })));
    }

    #[test]
    fn construction_ready_events_sorted_by_building_id() {
        use bevy::ecs::system::SystemState;
        use crate::game::{GameEvent, GamePhase, GameState};
        use crate::player::Player;
        let mut world = World::new();
        let mut game = GameState::new(4, 5, 1);
        game.phase = GamePhase::Playing;
        let (map, owner) = tiny_owned_map();
        game.map = map;
        game.players.push(Player::new_human(owner, "d".into(), [1.0, 0.0, 0.0]));
        game.player_lookup.resize(owner as usize + 1, None);
        game.player_lookup[owner as usize] = Some(0);
        world.insert_resource(game);
        world.spawn(Building {
            id: 2,
            owner_id: owner,
            tile_idx: 1,
            kind: BuildingKind::Factory,
            level: 1,
            under_construction: true,
            ticks_until_complete: 1,
        });
        world.spawn(Building {
            id: 1,
            owner_id: owner,
            tile_idx: 3,
            kind: BuildingKind::City,
            level: 1,
            under_construction: true,
            ticks_until_complete: 1,
        });

        {
            let mut state =
                SystemState::<(ResMut<GameState>, Query<(Entity, &mut Building)>)>::new(&mut world);
            let (mut game, mut q) = state.get_mut(&mut world);
            advance_building_construction_tick(&mut game, &mut q);
        }
        world.flush();

        let game = world.resource::<GameState>();
        let ready_ids: Vec<u64> = game
            .events
            .iter()
            .filter_map(|e| match e {
                GameEvent::StructureReady { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(ready_ids, vec![1, 2]);
    }

    #[test]
    fn compute_buildables_disables_when_too_poor() {
        let (map, owner) = tiny_owned_map();
        let rows = compute_buildables_for_player(&map, owner, 0.0, &[]);
        for e in rows {
            assert!(!e.can_build, "{:?}", e.kind);
        }
    }

    #[test]
    fn compute_buildables_port_invalid_without_shore() {
        let w = 6u32;
        let mut m = GameMap::new(w, 1);
        let owner = 1u16;
        for x in 0..w {
            m.set_owner_id(x, 0, owner);
            let ri = m.ref_id(x, 0);
            // Land but no shoreline bit — cannot place Port.
            m.terrain[ri] = MapTile::from_byte(0b1000_0000);
        }
        let rows = compute_buildables_for_player(&m, owner, 50_000.0, &[]);
        let port = rows.iter().find(|e| e.kind == BuildingKind::Port).unwrap();
        assert!(!port.can_build);
    }

    #[test]
    fn aggregate_ignores_under_construction() {
        let b = [
            Building {
                id: 1,
                owner_id: 1,
                tile_idx: 0,
                kind: BuildingKind::City,
                level: 5,
                under_construction: true,
                ticks_until_complete: 3,
            },
            Building {
                id: 2,
                owner_id: 1,
                tile_idx: 1,
                kind: BuildingKind::City,
                level: 2,
                under_construction: false,
                ticks_until_complete: 0,
            },
        ];
        let aggs = aggregate_buildings_per_player(b.into_iter(), 2);
        assert_eq!(aggs[1].city_levels, 2);
    }

    #[test]
    fn defense_post_bonus_scales_with_level() {
        let w = 20u32;
        let b = Building {
            id: 1,
            owner_id: 2,
            tile_idx: xy_idx(10, 10, w),
            kind: BuildingKind::DefensePost,
            level: 2,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let bonus = defense_post_priority_bonus(&[b], 2, 10, 10, w);
        assert_eq!(bonus, 2 * crate::config::DEFENSE_POST_PRIORITY_PER_LEVEL);
        let bonus_far = defense_post_priority_bonus(&[b], 2, 0, 0, w);
        assert_eq!(bonus_far, 0);
    }

    #[test]
    fn openfront_cost_scaling_city_and_port_factory_shared_counter() {
        let s = crate::config::OPENFRONT_GOLD_SCALE;
        let owner = 1u16;
        let city0 = structure_build_cost_gold(BuildingKind::City, owner, &[]);
        assert_eq!(city0, 125_000.0 / s);

        let one_city = [Building {
            id: 1,
            owner_id: owner,
            tile_idx: 0,
            kind: BuildingKind::City,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        }];
        let city1 = structure_build_cost_gold(BuildingKind::City, owner, &one_city);
        assert_eq!(city1, 250_000.0 / s);

        let pf = [
            Building {
                id: 2,
                owner_id: owner,
                tile_idx: 1,
                kind: BuildingKind::Port,
                level: 1,
                under_construction: false,
                ticks_until_complete: 0,
            },
            Building {
                id: 3,
                owner_id: owner,
                tile_idx: 2,
                kind: BuildingKind::Factory,
                level: 1,
                under_construction: false,
                ticks_until_complete: 0,
            },
        ];
        let next_pf = structure_build_cost_gold(BuildingKind::Factory, owner, &pf);
        assert_eq!(next_pf, 500_000.0 / s);
    }

    #[test]
    fn missile_structures_disabled_in_buildables_when_feature_off() {
        let (map, owner) = tiny_owned_map();
        let rows = compute_buildables_for_player(&map, owner, 1_000_000.0, &[]);
        let sam = rows
            .iter()
            .find(|e| e.kind == BuildingKind::SamLauncher)
            .unwrap();
        let silo = rows
            .iter()
            .find(|e| e.kind == BuildingKind::MissileSilo)
            .unwrap();
        assert!(!sam.can_build);
        assert!(!silo.can_build);
        assert!(!sam.can_upgrade);
        assert!(!silo.can_upgrade);
    }
}
