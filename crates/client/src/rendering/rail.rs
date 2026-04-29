//! OpenFront-style rails rendered as one GPU-friendly mesh + pooled train sprites.
//! Geometry follows `RailroadView.ts` + `RailroadSprites.ts` semantics.

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

use dark_rift_shared::game::{GameEvent, GameState};

use crate::{InGameEntity, TILE_SIZE};

const Z_RAIL_BRIDGE: f32 = 83.0;
const Z_RAIL_TRACK: f32 = 84.0;
const Z_RAIL_TRAIN: f32 = 86.0;

const BRIDGE_RGB: [f32; 4] = [197.0 / 255.0, 69.0 / 255.0, 72.0 / 255.0, 1.0];

#[derive(Resource)]
pub struct RailTrainSprites {
    pub engine: Handle<Image>,
    pub carriage: Handle<Image>,
    pub loaded_carriage: Handle<Image>,
}

#[derive(Component)]
pub struct RailTrainVis {
    pub train_id: u64,
}

#[derive(Component)]
pub(crate) struct RailNetworkMesh;

#[derive(Default, Clone, Copy)]
struct TileVertexRange {
    start: usize,
    end: usize,
}

#[derive(Resource, Default)]
pub struct RailRenderCache {
    topo_hash: (u64, usize),
    mesh_entity: Option<Entity>,
    mesh_handle: Option<Handle<Mesh>>,
    tile_ranges: HashMap<u32, Vec<TileVertexRange>>,
    tiles: Vec<u32>,
    tile_owner_cache: HashMap<u32, [u8; 4]>,
    next_check: usize,
}

#[derive(Resource, Default)]
pub struct RailColorDirty {
    pub tiles: HashSet<u32>,
}

#[derive(Resource, Default)]
pub struct RailTrainPool {
    pub by_train_id: HashMap<u64, Entity>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RailType {
    Vertical,
    Horizontal,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[inline]
fn tile_xy(tile: u32, w: u32) -> (i32, i32) {
    ((tile % w) as i32, (tile / w) as i32)
}

fn compute_extremity_direction(tile: u32, next: u32, w: u32) -> RailType {
    let (x, y) = tile_xy(tile, w);
    let (nx, ny) = tile_xy(next, w);
    let dx = nx - x;
    let dy = ny - y;
    if dx == 0 && dy == 0 {
        return RailType::Vertical;
    }
    if dx == 0 {
        RailType::Vertical
    } else if dy == 0 {
        RailType::Horizontal
    } else {
        RailType::Vertical
    }
}

fn compute_direction(prev: u32, current: u32, next: u32, w: u32) -> RailType {
    let (x1, y1) = tile_xy(prev, w);
    let (x2, y2) = tile_xy(current, w);
    let (x3, y3) = tile_xy(next, w);
    let dx1 = x2 - x1;
    let dy1 = y2 - y1;
    let dx2 = x3 - x2;
    let dy2 = y3 - y2;

    if dx1 == dx2 && dy1 == dy2 {
        if dx1 != 0 {
            return RailType::Horizontal;
        }
        if dy1 != 0 {
            return RailType::Vertical;
        }
    }

    if (dx1 == 0 && dx2 != 0) || (dx1 != 0 && dx2 == 0) {
        if dx1 == 0 && dx2 == 1 && dy1 == -1 {
            return RailType::BottomRight;
        }
        if dx1 == 0 && dx2 == -1 && dy1 == -1 {
            return RailType::BottomLeft;
        }
        if dx1 == 0 && dx2 == 1 && dy1 == 1 {
            return RailType::TopRight;
        }
        if dx1 == 0 && dx2 == -1 && dy1 == 1 {
            return RailType::TopLeft;
        }

        if dx1 == 1 && dx2 == 0 && dy2 == -1 {
            return RailType::TopLeft;
        }
        if dx1 == -1 && dx2 == 0 && dy2 == -1 {
            return RailType::TopRight;
        }
        if dx1 == 1 && dx2 == 0 && dy2 == 1 {
            return RailType::BottomLeft;
        }
        if dx1 == -1 && dx2 == 0 && dy2 == 1 {
            return RailType::BottomRight;
        }
    }
    RailType::Vertical
}

fn compute_rail_tiles(w: u32, tiles: &[u32]) -> Vec<(u32, RailType)> {
    if tiles.is_empty() {
        return Vec::new();
    }
    if tiles.len() == 1 {
        return vec![(tiles[0], RailType::Vertical)];
    }
    let mut out = Vec::with_capacity(tiles.len());
    out.push((
        tiles[0],
        compute_extremity_direction(tiles[0], tiles[1], w),
    ));
    for i in 1..tiles.len() - 1 {
        out.push((
            tiles[i],
            compute_direction(tiles[i - 1], tiles[i], tiles[i + 1], w),
        ));
    }
    out.push((
        tiles[tiles.len() - 1],
        compute_extremity_direction(tiles[tiles.len() - 1], tiles[tiles.len() - 2], w),
    ));
    out
}

fn railroad_rects(kind: RailType) -> &'static [[i32; 4]] {
    match kind {
        RailType::Horizontal => &[[-1, -1, 2, 1], [-1, 1, 2, 1], [-1, 0, 1, 1]],
        RailType::Vertical => &[[-1, -1, 1, 2], [1, -1, 1, 2], [0, 0, 1, 1]],
        RailType::TopRight => &[[-1, -1, 1, 1], [0, -1, 1, 2], [1, -1, 1, 3]],
        RailType::TopLeft => &[[-1, -1, 1, 3], [0, -1, 1, 2], [1, -1, 1, 1]],
        RailType::BottomRight => &[[-1, 1, 1, 1], [0, 0, 1, 2], [1, -1, 1, 3]],
        RailType::BottomLeft => &[[-1, -1, 1, 3], [0, 0, 1, 2], [1, 1, 1, 1]],
    }
}

fn bridge_rects(kind: RailType) -> &'static [[i32; 4]] {
    match kind {
        RailType::Horizontal => &[[-1, -2, 3, 1], [-1, 2, 3, 1], [-1, 3, 1, 1], [1, 3, 1, 1]],
        RailType::Vertical => &[[-2, -1, 1, 3], [2, -1, 1, 3]],
        RailType::TopRight => &[[-2, -2, 1, 2], [-1, 0, 1, 1], [0, 1, 1, 1], [1, 2, 2, 1], [2, -2, 1, 1]],
        RailType::BottomLeft => &[[-2, -2, 2, 1], [0, -1, 1, 1], [1, 0, 1, 1], [2, 1, 1, 2], [-2, 2, 1, 1]],
        RailType::TopLeft => &[[-2, -2, 1, 1], [-2, 2, 2, 1], [0, 1, 1, 1], [1, 0, 1, 1], [2, -2, 1, 2]],
        RailType::BottomRight => &[[-2, 1, 1, 2], [-1, 0, 1, 1], [0, -1, 1, 1], [1, -2, 2, 1], [2, 2, 1, 1]],
    }
}

#[inline]
fn owner_rgba(game: &GameState, x: u32, y: u32) -> [f32; 4] {
    let id = game.map.owner_id(x, y);
    if id == 0 {
        return [1.0, 1.0, 1.0, 1.0];
    }
    game.players
        .iter()
        .find(|p| p.id == id)
        .map(|p| [p.color[0], p.color[1], p.color[2], 1.0])
        .unwrap_or([1.0, 1.0, 1.0, 1.0])
}

fn owner_key(game: &GameState, tile_idx: u32, w: u32) -> [u8; 4] {
    let x = tile_idx % w;
    let y = tile_idx / w;
    let c = owner_rgba(game, x, y);
    [
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
        255,
    ]
}

#[inline]
fn tile_top_left_world(tx: u32, ty: u32, map_h: u32) -> Vec2 {
    Vec2::new(
        tx as f32 * TILE_SIZE,
        map_h as f32 * TILE_SIZE - ty as f32 * TILE_SIZE,
    )
}

fn push_rect(
    positions: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    tx: u32,
    ty: u32,
    map_h: u32,
    rect: [i32; 4],
    rgba: [f32; 4],
    z: f32,
) {
    let tl = tile_top_left_world(tx, ty, map_h);
    let half = TILE_SIZE * 0.5;
    let wx = tl.x + rect[0] as f32 * half;
    let wy = tl.y - rect[1] as f32 * half;
    let ww = rect[2] as f32 * half;
    let wh = rect[3] as f32 * half;
    let base = positions.len() as u32;

    positions.push([wx, wy, z]);
    positions.push([wx + ww, wy, z]);
    positions.push([wx + ww, wy - wh, z]);
    positions.push([wx, wy - wh, z]);

    colors.push(rgba);
    colors.push(rgba);
    colors.push(rgba);
    colors.push(rgba);

    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

pub fn setup_rail_visual_meshes(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(RailTrainSprites {
        engine: asset_server.load("sprites/trainEngine.png"),
        carriage: asset_server.load("sprites/trainCarriage.png"),
        loaded_carriage: asset_server.load("sprites/trainCarriageLoaded.png"),
    });
    commands.init_resource::<RailRenderCache>();
    commands.init_resource::<RailColorDirty>();
    commands.init_resource::<RailTrainPool>();
}

pub(crate) fn system_mark_rail_color_dirty(
    game_opt: Option<Res<GameState>>,
    mut dirty: ResMut<RailColorDirty>,
) {
    let Some(game) = game_opt.as_deref() else {
        return;
    };
    let w = game.map.width;
    for ev in &game.events {
        match ev {
            GameEvent::TileCaptured { x, y, .. } | GameEvent::TileExpanded { x, y, .. } => {
                dirty.tiles.insert((*y * w) + *x);
            }
            _ => {}
        }
    }
}

pub(crate) fn system_rail_topology_rebuild(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    mut cache: ResMut<RailRenderCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    q_rail_mesh: Query<Entity, With<RailNetworkMesh>>,
) {
    let Some(game) = game_opt.as_deref() else {
        return;
    };

    let topo_hash = (game.rail.next_rail_id, game.rail.railroads.len());
    if cache.topo_hash == topo_hash {
        return;
    }

    cache.tile_ranges.clear();
    cache.tiles.clear();
    cache.tile_owner_cache.clear();
    cache.next_check = 0;

    let mw = game.map.width;
    let mh = game.map.height;

    if game.rail.railroads.is_empty() {
        // No rails — despawn any existing mesh entity and clear handles.
        for e in q_rail_mesh.iter() {
            commands.entity(e).despawn();
        }
        cache.mesh_entity = None;
        cache.mesh_handle = None;
        cache.topo_hash = topo_hash;
        return;
    }

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for railroad in &game.rail.railroads {
        for (tile_idx, kind) in compute_rail_tiles(mw, &railroad.tiles) {
            let tx = tile_idx % mw;
            let ty = tile_idx / mw;
            if game.map.terrain[tile_idx as usize].is_water() {
                for &r in bridge_rects(kind) {
                    push_rect(
                        &mut positions,
                        &mut colors,
                        &mut indices,
                        tx,
                        ty,
                        mh,
                        r,
                        BRIDGE_RGB,
                        Z_RAIL_BRIDGE,
                    );
                }
            }

            let start = colors.len();
            let tile_color = owner_rgba(game, tx, ty);
            for &r in railroad_rects(kind) {
                push_rect(
                    &mut positions,
                    &mut colors,
                    &mut indices,
                    tx,
                    ty,
                    mh,
                    r,
                    tile_color,
                    Z_RAIL_TRACK,
                );
            }
            let end = colors.len();
            cache
                .tile_ranges
                .entry(tile_idx)
                .or_default()
                .push(TileVertexRange { start, end });
            cache.tile_owner_cache.insert(tile_idx, owner_key(game, tile_idx, mw));
        }
    }

    cache.tiles = cache.tile_ranges.keys().copied().collect();
    cache.tiles.sort_unstable();

    // Build or mutate the mesh — reuse the existing handle to avoid WASM memory bloat.
    let mut new_mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    new_mesh.insert_indices(Indices::U32(indices));

    if let Some(ref h) = cache.mesh_handle {
        // Reuse the existing handle — overwrite the mesh data in-place.
        // Only one AssetEvent::Modified, and no new allocation in Bevy's asset store.
        if let Some(existing) = meshes.get_mut(h) {
            *existing = new_mesh;
        }
    } else {
        // First time — create handle, material, and entity.
        let mesh_h = meshes.add(new_mesh);
        let mat_h = materials.add(ColorMaterial::from_color(Color::WHITE));
        let entity = commands
            .spawn((
                RailNetworkMesh,
                InGameEntity,
                Mesh2d(mesh_h.clone()),
                MeshMaterial2d(mat_h),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        cache.mesh_entity = Some(entity);
        cache.mesh_handle = Some(mesh_h);
    }

    cache.topo_hash = topo_hash;
    // Colors are already correct from the rebuild — no need to mark dirty.
}

fn set_tile_vertex_color(
    mesh: &mut Mesh,
    ranges: &[TileVertexRange],
    color: [f32; 4],
) {
    let Some(VertexAttributeValues::Float32x4(cols)) = mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR)
    else {
        return;
    };
    for range in ranges {
        let end = range.end.min(cols.len());
        let start = range.start.min(end);
        for c in &mut cols[start..end] {
            *c = color;
        }
    }
}

pub(crate) fn system_rail_color_refresh(
    game_opt: Option<Res<GameState>>,
    mut cache: ResMut<RailRenderCache>,
    mut dirty: ResMut<RailColorDirty>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(game) = game_opt.as_deref() else {
        return;
    };
    let Some(mesh_h) = cache.mesh_handle.clone() else {
        return;
    };
    if cache.tiles.is_empty() && dirty.tiles.is_empty() {
        return;
    }

    let mw = game.map.width;

    // --- Phase 1: Pre-scan without touching the mesh ---
    // Collect tiles that actually need a color update, using only read access.
    // This avoids the AssetEvent::Modified that get_mut emits unconditionally.

    let mut tiles_to_update: Vec<(u32, [f32; 4])> = Vec::new();

    // Event-driven tiles (always dirty).
    for &tile_idx in &dirty.tiles {
        if cache.tile_ranges.contains_key(&tile_idx) {
            let x = tile_idx % mw;
            let y = tile_idx / mw;
            tiles_to_update.push((tile_idx, owner_rgba(game, x, y)));
        }
    }

    // Budgeted periodic scan: check a slice of tiles for owner changes.
    if !cache.tiles.is_empty() {
        let tiles_per_frame = ((cache.tiles.len() + 119) / 120).max(1);
        for _ in 0..tiles_per_frame {
            if cache.next_check >= cache.tiles.len() {
                cache.next_check = 0;
            }
            let tile_idx = cache.tiles[cache.next_check];
            cache.next_check += 1;

            // Skip tiles already queued from dirty set.
            if dirty.tiles.contains(&tile_idx) {
                continue;
            }

            let new_key = owner_key(game, tile_idx, mw);
            let old_key = cache.tile_owner_cache.get(&tile_idx).copied();
            if old_key != Some(new_key) {
                let x = tile_idx % mw;
                let y = tile_idx / mw;
                tiles_to_update.push((tile_idx, owner_rgba(game, x, y)));
            }
        }
    }

    // Clear dirty set now that we've consumed it.
    dirty.tiles.clear();

    // --- Phase 2: Only call get_mut if there is real work to do ---
    // This is the key fix: get_mut emits AssetEvent::Modified unconditionally
    // in Bevy 0.18, which forces a full GPU buffer re-upload every frame.
    if tiles_to_update.is_empty() {
        return;
    }

    let Some(mesh) = meshes.get_mut(&mesh_h) else {
        return;
    };

    for (tile_idx, color) in tiles_to_update {
        if let Some(ranges) = cache.tile_ranges.get(&tile_idx) {
            set_tile_vertex_color(mesh, ranges, color);
            cache.tile_owner_cache.insert(tile_idx, owner_key(game, tile_idx, mw));
        }
    }
}

pub(crate) fn system_train_visuals_pool(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    sprites: Res<RailTrainSprites>,
    mut pool: ResMut<RailTrainPool>,
    mut q_train: Query<(Entity, &RailTrainVis, &mut Transform, &mut Sprite)>,
) {
    let Some(game) = game_opt.as_deref() else {
        return;
    };

    let mw = game.map.width;
    let mh = game.map.height;
    let mut active_ids: HashSet<u64> = HashSet::new();

    for tr in &game.rail.trains {
        if !tr.active {
            continue;
        }
        let Some(tile) = tr.display_engine_tile(&game.rail) else {
            continue;
        };
        let p = crate::tile_idx_to_world(tile, mw, mh);
        let target = Vec3::new(p.x, p.y, Z_RAIL_TRAIN);
        active_ids.insert(tr.id);

        if let Some(&entity) = pool.by_train_id.get(&tr.id)
            && let Ok((_, _, mut t, mut s)) = q_train.get_mut(entity)
        {
            s.flip_x = target.x < t.translation.x;
            t.translation = t.translation.lerp(target, 0.4);
            continue;
        }

        let e = commands
            .spawn((
                RailTrainVis { train_id: tr.id },
                InGameEntity,
                Sprite {
                    image: sprites.engine.clone(),
                    custom_size: Some(Vec2::splat(TILE_SIZE * 1.1)),
                    ..default()
                },
                Transform::from_translation(target),
                GlobalTransform::default(),
            ))
            .id();
        pool.by_train_id.insert(tr.id, e);
    }

    let stale_ids: Vec<u64> = pool
        .by_train_id
        .keys()
        .filter(|id| !active_ids.contains(id))
        .copied()
        .collect();
    for id in stale_ids {
        if let Some(ent) = pool.by_train_id.remove(&id) {
            commands.entity(ent).despawn();
        }
    }
}
