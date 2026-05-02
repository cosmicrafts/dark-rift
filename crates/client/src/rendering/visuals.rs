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


pub fn system_sync_fleet_visuals(
    mut commands: Commands,
    engine_opt: Option<Res<DarkRiftEngine>>,
    meshes: Option<Res<FleetVisualMeshes>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    my_id: Res<MyPlayerId>,
    q_existing: Query<(Entity, &FleetVisual)>,
    mut boat_q: Query<(Entity, &mut FollowFleetBoat, &mut Transform), (Without<FollowFleetWake>, Without<FollowFleetTarget>)>,
    mut wake_q: Query<(&FollowFleetWake, &mut Transform, &mut Visibility), (Without<FollowFleetBoat>, Without<FollowFleetTarget>)>,
    mut target_q: Query<(Entity, &FollowFleetTarget, &mut Transform), (Without<FollowFleetBoat>, Without<FollowFleetWake>)>,
    mut local_existing: Local<Vec<u64>>,
) {
    let Some(engine) = engine_opt.as_deref() else { return; };
    let Some(m) = meshes.as_ref() else { return; };
    let pid = my_id.0;
    if pid == 0 { return; }
    let mw = engine.state.map.width;
    let mh = engine.state.map.height;

    local_existing.clear();
    for (_, vis) in &q_existing {
        local_existing.push(vis.0);
    }
    local_existing.sort_unstable();

    for (e, vis) in &q_existing {
        if engine.fleets.binary_search_by_key(&vis.0, |f| f.id).is_err() {
            commands.entity(e).try_despawn();
        }
    }

    for f in engine.fleets.iter() {
        if f.owner_id == pid && local_existing.binary_search(&f.id).is_err() {
            let boat_mat = materials.add(ColorMaterial::from_color(Color::srgba(0.12, 0.82, 1.0, 0.9)));
            let wake_mat = materials.add(ColorMaterial::from_color(Color::srgba(0.35, 0.95, 1.0, 0.4)));
            let ring_mat = materials.add(ColorMaterial::from_color(Color::srgba(1.0, 0.1, 0.06, 0.42)));
            let p0 = tile_idx_to_world(f.current_tile, mw, mh);
            
            let wake_e = commands.spawn((
                FollowFleetWake,
                InGameEntity,
                Mesh2d(m.wake_dot.clone()),
                MeshMaterial2d(wake_mat),
                Transform::from_xyz(p0.x, p0.y, 269.0).with_scale(Vec3::ZERO),
                GlobalTransform::default(),
                Visibility::Hidden,
            )).id();
            
            let root_e = commands.spawn((
                FleetVisual(f.id),
                InGameEntity,
                Transform::default(),
                GlobalTransform::default(),
                Visibility::default(),
            )).id();
            
            commands.entity(root_e).with_children(|ch| {
                ch.spawn((
                    FollowFleetBoat {
                        fleet_id: f.id,
                        wake: wake_e,
                        prev_tile: None,
                    },
                    Mesh2d(m.boat.clone()),
                    MeshMaterial2d(boat_mat),
                    Transform::from_xyz(p0.x, p0.y, 272.0),
                    GlobalTransform::default(),
                    Visibility::default(),
                ));
                let pl = tile_idx_to_world(f.dst_tile, mw, mh);
                ch.spawn((
                    FollowFleetTarget { fleet_id: f.id },
                    Mesh2d(m.target_ring.clone()),
                    MeshMaterial2d(ring_mat),
                    Transform::from_xyz(pl.x, pl.y, 264.0).with_scale(Vec3::splat(1.12)),
                    GlobalTransform::default(),
                    Visibility::default(),
                ));
            });
        }
    }

    for (e, mut fb, mut tf) in boat_q.iter_mut() {
        let Some(f) = engine.fleets.iter().find(|x| x.id == fb.fleet_id) else {
            commands.entity(fb.wake).try_despawn();
            commands.entity(e).try_despawn();
            continue;
        };
        let cur = f.current_tile;
        let p = tile_idx_to_world(cur, mw, mh);
        tf.translation = Vec3::new(p.x, p.y, 272.0);
        if let Ok((_, mut wtf, mut vis)) = wake_q.get_mut(fb.wake) {
            if let Some(prev) = fb.prev_tile {
                if prev != cur {
                    let wp = tile_idx_to_world(prev, mw, mh);
                    wtf.translation = Vec3::new(wp.x, wp.y, 269.0);
                    wtf.scale = Vec3::ONE;
                    *vis = Visibility::Visible;
                }
            }
            fb.prev_tile = Some(cur);
        }
    }
    for (e, ft, _tf) in target_q.iter_mut() {
        if !engine.fleets.iter().any(|x| x.id == ft.fleet_id) {
            commands.entity(e).try_despawn();
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
    mut q: Query<(Entity, &mut AttackCommandRing, &mut Transform, &Children)>,
    mesh_q: Query<&MeshMaterial2d<ColorMaterial>, With<AttackPingMesh>>,
    mut text_color_q: Query<&mut TextColor, With<AttackPingTroopLabel>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let dt = time.delta_secs();
    for (entity, mut ring, mut transform, children) in &mut q {
        ring.elapsed += dt;
        let t = ring.elapsed;
        if t >= ATTACK_COMMAND_RING_LIFETIME {
            commands.entity(entity).try_despawn();
            continue;
        }

        let life = t / ATTACK_COMMAND_RING_LIFETIME;
        let damp = (1.0 - life).clamp(0.0, 1.0);

        let pop_u = (t / ATTACK_RING_POP_SECS).min(1.0);
        let pop = ease_out_back_scaled(pop_u);

        let wobble = 1.0 + 0.055 * damp * (t * std::f32::consts::TAU * 4.0).sin();
        transform.scale = Vec3::splat(pop * wobble);

        let fade = damp.powf(1.15);
        let [r, g, b] = ring.rgb;

        for child in children.iter() {
            if let Ok(mat_h) = mesh_q.get(child) {
                if let Some(mat) = materials.get_mut(&mat_h.0) {
                    mat.color = Color::srgba(r, g, b, 0.97 * fade);
                }
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
    let label = format!("{:.0}", send);
    let ls = clamp_ui_scale(settings.config.ui.click_troop_label_scale);
    let ox = TILE_SIZE * 1.75 * ls;
    let oy = TILE_SIZE * 0.35 * ls;
    let troop_font = 28.0 * ls;
    commands
        .spawn((
            AttackCommandRing {
                elapsed: 0.0,
                rgb: player_color,
            },
            InGameEntity,
            Transform::from_xyz(wx, wy, 260.0).with_scale(Vec3::ZERO),
            Visibility::default(),
            InheritedVisibility::default(),
            ViewVisibility::default(),
        ))
        .with_children(|root| {
            root.spawn((
                AttackPingMesh,
                Mesh2d(ring_mesh.0.clone()),
                MeshMaterial2d(mat),
                Transform::default(),
            ));
            root.spawn((
                AttackPingTroopLabel,
                Text2d::new(label),
                TextFont {
                    font: settings.font.clone(),
                    font_size: troop_font,
                    weight: FontWeight::BOLD,
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::new_with_justify(Justify::Center),
                Transform::from_xyz(ox, oy, 1.0),
            ));
        });
}

