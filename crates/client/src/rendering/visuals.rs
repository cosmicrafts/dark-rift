use bevy::prelude::*;

use dark_rift_shared::engine::DarkRiftEngine;


use crate::{
    ClientSettings, InGameEntity, MyPlayerId, TILE_SIZE, tile_idx_to_world, clamp_ui_scale,
    FleetVisual,
};

#[derive(Resource)]
pub struct AttackRingMesh(pub Handle<Mesh>);

/// Root of the attack “ping”: scales as one unit; children are mesh + troop label.
#[derive(Component)]
pub struct AttackCommandRing {
    pub elapsed: f32,
    /// Territory RGB for the ring tint (matches `Player::color`).
    pub rgb: [f32; 3],
}

#[derive(Component)]
pub struct AttackPingMesh;

#[derive(Component)]
pub struct AttackPingTroopLabel;

#[derive(Component, Clone)]
pub struct AttackRingNodes {
    pub root: Entity,
    pub mesh: Entity,
    pub label: Entity,
}

#[derive(Resource, Default)]
pub struct AttackRingPool {
    pub unused: Vec<AttackRingNodes>,
}

impl AttackRingPool {
    pub fn clear(&mut self) {
        self.unused.clear();
    }
}

/// Meshes for local-only transport visuals (boat disc, wake speck, landing ring).
#[derive(Resource)]
pub struct FleetVisualMeshes {
    pub boat: Handle<Mesh>,
    pub wake_dot: Handle<Mesh>,
    pub target_ring: Handle<Mesh>,
}

/// Follows `WarpFleet.current_tile` for the local player.
#[derive(Component)]
pub struct FollowFleetBoat {
    pub fleet_id: u64,
    pub wake: Entity,
    pub prev_tile: Option<u32>,
}

/// One-tile “wake” behind the boat (OpenFront `UnitLayer` trail simplified).
#[derive(Component)]
pub struct FollowFleetWake;

/// Fixed at landing tile until the fleet entity is gone.
#[derive(Component)]
pub struct FollowFleetTarget {
    pub fleet_id: u64,
}

pub struct FleetNodes {
    pub root: Entity,
    pub wake: Entity,
    pub boat: Entity,
    pub target: Entity,
}

#[derive(Resource, Default)]
pub struct FleetVisualPool {
    pub unused: Vec<FleetNodes>,
    pub used: std::collections::HashMap<u64, FleetNodes>,
}

impl FleetVisualPool {
    pub fn clear(&mut self) {
        self.used.clear();
        self.unused.clear();
    }
}

pub fn system_sync_fleet_visuals(
    mut commands: Commands,
    engine_opt: Option<Res<DarkRiftEngine>>,
    meshes: Option<Res<FleetVisualMeshes>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    my_id: Res<MyPlayerId>,
    q_existing: Query<(Entity, &FleetVisual)>,
    mut boat_q: Query<(Entity, &mut FollowFleetBoat, &mut Transform), (Without<FollowFleetWake>, Without<FollowFleetTarget>)>,
    mut wake_q: Query<(&FollowFleetWake, &mut Transform, &mut Visibility), (Without<FollowFleetBoat>, Without<FollowFleetTarget>)>,
    mut target_q: Query<(Entity, &mut FollowFleetTarget, &mut Transform), (Without<FollowFleetBoat>, Without<FollowFleetWake>)>,
    mut pool: ResMut<FleetVisualPool>,
) {
    let Some(engine) = engine_opt.as_deref() else { return; };
    let Some(m) = meshes.as_ref() else { return; };
    let pid = my_id.0;
    if pid == 0 { return; }
    let mw = engine.state.map.width;
    let mh = engine.state.map.height;

    // 1. Recycle dead fleets
    let mut to_recycle = Vec::new();
    for (&id, nodes) in pool.used.iter() {
        if engine.fleets.binary_search_by_key(&id, |f| f.id).is_err() {
            to_recycle.push(id);
        }
    }
    for id in to_recycle {
        if let Some(nodes) = pool.used.remove(&id) {
            commands.entity(nodes.root).insert(Visibility::Hidden);
            commands.entity(nodes.wake).insert(Visibility::Hidden);
            pool.unused.push(nodes);
        }
    }

    // 2. Spawn / Update active fleets
    for f in engine.fleets.iter() {
        if f.owner_id == pid {
            if pool.used.contains_key(&f.id) { continue; } // Already spawned

            let boat_mat = materials.add(ColorMaterial::from_color(Color::srgba(0.12, 0.82, 1.0, 0.9)));
            let wake_mat = materials.add(ColorMaterial::from_color(Color::srgba(0.35, 0.95, 1.0, 0.4)));
            let ring_mat = materials.add(ColorMaterial::from_color(Color::srgba(1.0, 0.1, 0.06, 0.42)));
            let p0 = tile_idx_to_world(f.current_tile, mw, mh);
            let pl = tile_idx_to_world(f.dst_tile, mw, mh);

            if let Some(nodes) = pool.unused.pop() {
                commands.entity(nodes.root).insert((
                    FleetVisual(f.id),
                    Visibility::Inherited,
                ));
                commands.entity(nodes.wake).insert((
                    MeshMaterial2d(wake_mat),
                    Transform::from_xyz(p0.x, p0.y, 269.0).with_scale(Vec3::ZERO),
                    Visibility::Hidden, // hidden until it moves
                ));
                commands.entity(nodes.boat).insert((
                    FollowFleetBoat { fleet_id: f.id, wake: nodes.wake, prev_tile: None },
                    MeshMaterial2d(boat_mat),
                    Transform::from_xyz(p0.x, p0.y, 272.0),
                ));
                commands.entity(nodes.target).insert((
                    FollowFleetTarget { fleet_id: f.id },
                    MeshMaterial2d(ring_mat),
                    Transform::from_xyz(pl.x, pl.y, 264.0).with_scale(Vec3::splat(1.12)),
                ));
                pool.used.insert(f.id, nodes);
            } else {
                let wake = commands.spawn((
                    FollowFleetWake,
                    InGameEntity,
                    Mesh2d(m.wake_dot.clone()),
                    MeshMaterial2d(wake_mat),
                    Transform::from_xyz(p0.x, p0.y, 269.0).with_scale(Vec3::ZERO),
                    GlobalTransform::default(),
                    Visibility::Hidden,
                )).id();
                
                let root = commands.spawn((
                    FleetVisual(f.id),
                    InGameEntity,
                    Transform::default(),
                    GlobalTransform::default(),
                    Visibility::Inherited,
                )).id();
                
                let mut boat = Entity::PLACEHOLDER;
                let mut target = Entity::PLACEHOLDER;
                commands.entity(root).with_children(|ch| {
                    boat = ch.spawn((
                        FollowFleetBoat { fleet_id: f.id, wake, prev_tile: None },
                        Mesh2d(m.boat.clone()),
                        MeshMaterial2d(boat_mat),
                        Transform::from_xyz(p0.x, p0.y, 272.0),
                        GlobalTransform::default(),
                        Visibility::Inherited,
                    )).id();
                    target = ch.spawn((
                        FollowFleetTarget { fleet_id: f.id },
                        Mesh2d(m.target_ring.clone()),
                        MeshMaterial2d(ring_mat),
                        Transform::from_xyz(pl.x, pl.y, 264.0).with_scale(Vec3::splat(1.12)),
                        GlobalTransform::default(),
                        Visibility::Inherited,
                    )).id();
                });
                pool.used.insert(f.id, FleetNodes { root, wake, boat, target });
            }
        }
    }

    for (e, mut fb, mut tf) in boat_q.iter_mut() {
        let Ok(fi) = engine.fleets.binary_search_by_key(&fb.fleet_id, |x| x.id) else {
            // Recycling is handled in the phase above, skip manual despawning
            fb.fleet_id = 0; // mark invalid until recycled
            continue;
        };
        let f = &engine.fleets[fi];
        let cur = f.current_tile;
        let p = tile_idx_to_world(cur, mw, mh);
        tf.translation = Vec3::new(p.x, p.y, 272.0);
        if let Ok((_, mut wtf, mut vis)) = wake_q.get_mut(fb.wake) {
            if let Some(prev) = fb.prev_tile
                && prev != cur {
                    let wp = tile_idx_to_world(prev, mw, mh);
                    wtf.translation = Vec3::new(wp.x, wp.y, 269.0);
                    wtf.scale = Vec3::ONE;
                    *vis = Visibility::Inherited;
                }
            fb.prev_tile = Some(cur);
        }
    }
    for (e, mut ft, _tf) in target_q.iter_mut() {
        if engine
            .fleets
            .binary_search_by_key(&ft.fleet_id, |x| x.id)
            .is_err()
        {
            ft.fleet_id = 0;
        }
    }
}

/// Overshoot ease 0→1 in `t ∈ [0,1]` (landing exactly at 1).
pub fn ease_out_back_scaled(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.701_58_f32;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// Quick 0→full scale pop (rest of lifetime = settle + fade).
const ATTACK_RING_POP_SECS: f32 = 0.1;
const ATTACK_COMMAND_RING_LIFETIME: f32 = 0.38;

pub fn system_animate_attack_command_rings(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut AttackCommandRing, &mut Transform, &Children, Option<&AttackRingNodes>)>,
    mesh_q: Query<&MeshMaterial2d<ColorMaterial>, With<AttackPingMesh>>,
    mut text_color_q: Query<&mut TextColor, With<AttackPingTroopLabel>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut pool: ResMut<AttackRingPool>,
) {
    let dt = time.delta_secs();
    for (entity, mut ring, mut transform, children, opt_nodes) in &mut q {
        ring.elapsed += dt;
        let t = ring.elapsed;
        if t >= ATTACK_COMMAND_RING_LIFETIME {
            if t < 900.0 {
                commands.entity(entity).insert(Visibility::Hidden);
                if let Some(nodes) = opt_nodes {
                    pool.unused.push(nodes.clone());
                }
                ring.elapsed = 999.0; // mark as dead
            }
            continue;
        }
        if t > 10.0 { continue; } // already dead

        let life = t / ATTACK_COMMAND_RING_LIFETIME;
        let damp = (1.0 - life).clamp(0.0, 1.0);

        let pop_u = (t / ATTACK_RING_POP_SECS).min(1.0);
        let pop = ease_out_back_scaled(pop_u);

        let wobble = 1.0 + 0.055 * damp * (t * std::f32::consts::TAU * 4.0).sin();
        transform.scale = Vec3::splat(pop * wobble);

        let fade = damp.powf(1.15);
        let [r, g, b] = ring.rgb;

        for child in children.iter() {
            if let Ok(mat_h) = mesh_q.get(child)
                && let Some(mat) = materials.get_mut(&mat_h.0) {
                    mat.color = Color::srgba(r, g, b, 0.97 * fade);
                }
            if let Ok(mut tc) = text_color_q.get_mut(child) {
                *tc = TextColor(Color::srgba(
                    1.0,
                    1.0,
                    1.0,
                    (0.25 + 0.75 * fade).clamp(0.0, 1.0),
                ));
            }
        }
    }
}

pub fn spawn_attack_command_ring(
    commands: &mut Commands,
    pool: &mut AttackRingPool,
    ring_mesh: &AttackRingMesh,
    materials: &mut Assets<ColorMaterial>,
    settings: &ClientSettings,
    player_color: [f32; 3],
    x: u32,
    y: u32,
    map_h: u32,
    send: f64,
) {
    let wx = (x as f32 + 0.5) * TILE_SIZE;
    let wy = (map_h as f32 - y as f32 - 0.5) * TILE_SIZE;
    let [r, g, b] = player_color;
    let mat = materials.add(ColorMaterial::from_color(Color::srgba(r, g, b, 0.97)));
    let label_str = format!("{:.0}", send);
    let ls = clamp_ui_scale(settings.config.ui.click_troop_label_scale);
    let ox = TILE_SIZE * 1.75 * ls;
    let oy = TILE_SIZE * 0.35 * ls;
    let troop_font = 28.0 * ls;

    if let Some(nodes) = pool.unused.pop() {
        commands.entity(nodes.root).insert((
            AttackCommandRing { elapsed: 0.0, rgb: player_color },
            Transform::from_xyz(wx, wy, 260.0).with_scale(Vec3::ZERO),
            Visibility::Inherited,
        ));
        commands.entity(nodes.mesh).insert(MeshMaterial2d(mat));
        commands.entity(nodes.label).insert((
            Text2d::new(label_str),
            TextFont {
                font: settings.font.clone(),
                font_size: troop_font,
                weight: FontWeight::BOLD,
                ..default()
            },
            Transform::from_xyz(ox, oy, 1.0),
        ));
        return;
    }

    let mut mesh_e = Entity::PLACEHOLDER;
    let mut label_e = Entity::PLACEHOLDER;

    let root = commands
        .spawn((
            AttackCommandRing {
                elapsed: 0.0,
                rgb: player_color,
            },
            InGameEntity,
            Transform::from_xyz(wx, wy, 260.0).with_scale(Vec3::ZERO),
            Visibility::Inherited,
            InheritedVisibility::default(),
            ViewVisibility::default(),
        )).id();
        
    commands.entity(root).with_children(|ch| {
            mesh_e = ch.spawn((
                AttackPingMesh,
                Mesh2d(ring_mesh.0.clone()),
                MeshMaterial2d(mat),
                Transform::default(),
            )).id();
            label_e = ch.spawn((
                AttackPingTroopLabel,
                Text2d::new(label_str),
                TextFont {
                    font: settings.font.clone(),
                    font_size: troop_font,
                    weight: FontWeight::BOLD,
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::new_with_justify(Justify::Center),
                Transform::from_xyz(ox, oy, 1.0),
            )).id();
        });

    commands.entity(root).insert(AttackRingNodes { root, mesh: mesh_e, label: label_e });
}

