use bevy::prelude::*;
use dark_rift_shared::engine::DarkRiftEngine;

use dark_rift_shared::game::BuildingKind;
use crate::*;

pub fn building_kind_short(kind: BuildingKind) -> &'static str {
    match kind {
        BuildingKind::City => "C",
        BuildingKind::Factory => "F",
        BuildingKind::Port => "P",
        BuildingKind::DefensePost => "D",
        BuildingKind::SamLauncher => "S",
        BuildingKind::MissileSilo => "M",
    }
}

pub fn building_kind_icon_path(kind: BuildingKind) -> &'static str {
    match kind {
        BuildingKind::City => "icons/building_city.png",
        BuildingKind::Factory => "icons/building_factory.png",
        BuildingKind::Port => "icons/building_port.png",
        BuildingKind::DefensePost => "icons/building_defense.png",
        BuildingKind::SamLauncher => "icons/building_sam.png",
        BuildingKind::MissileSilo => "icons/building_silo.png",
    }
}

pub fn system_sync_structure_ghost(
    mut commands: Commands,
    engine_opt: Option<Res<DarkRiftEngine>>,
    my_id: Res<MyPlayerId>,
    mode: Res<BuildStructureMode>,
    hovered: Res<HoveredTile>,
    meshes: Option<Res<StructureMarkerMeshes>>,
    mut ghost_parts: ResMut<StructureGhostParts>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut scratch: Local<dark_rift_shared::engine::PlacementScratch>,
) {
    let despawn_all = |commands: &mut Commands, parts: &mut StructureGhostParts| {
        if let Some(e) = parts.marker.take() {
            commands.entity(e).try_despawn();
        }
        if let Some(e) = parts.range.take() {
            commands.entity(e).try_despawn();
        }
    };

    let Some(engine) = engine_opt.as_deref() else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };
    let Some(ms) = meshes.as_ref() else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };
    let Some(kind) = mode.0 else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };
    let Some((x, y)) = hovered.0 else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };

    let pid = my_id.0;
    if pid == 0 {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    }
    let Some(player) = engine.state.player(pid) else {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    };

    let w = engine.state.map.width;
    let mh = engine.state.map.height;
    let click_idx = y * w + x;
    if engine.state.map.owner_id(x, y) != pid {
        despawn_all(&mut commands, &mut ghost_parts);
        return;
    }

    let snapshot = &engine.buildings;
    let positions = existing_structure_positions_with_width(snapshot.iter(), w, None);
    let cost = structure_build_cost_gold(kind, pid, snapshot);
    let afford = player.gold >= cost && cost.is_finite();

    enum GhostKind {
        Upgrade,
        Place,
        Invalid,
    }

    let (tile_idx, ghost_kind, ok_afford) =
        if let Some(bid) = find_upgrade_target_id(&engine.state.map, pid, kind, click_idx, &snapshot) {
            let mut ti = None;
            for b in snapshot {
                if b.id == bid {
                    ti = Some(b.tile_idx);
                    break;
                }
            }
            let Some(ti) = ti else {
                despawn_all(&mut commands, &mut ghost_parts);
                return;
            };
            (ti, GhostKind::Upgrade, afford)
        } else if let Some(spawn) =
            resolve_structure_spawn_tile(&engine.state.map, pid, kind, click_idx, &positions, &mut *scratch)
        {
            (spawn, GhostKind::Place, afford)
        } else {
            (click_idx, GhostKind::Invalid, false)
        };

    let pos = tile_idx_to_world(tile_idx, w, mh);
    let color = match ghost_kind {
        GhostKind::Invalid => Color::srgba(0.95, 0.25, 0.2, 0.4),
        GhostKind::Upgrade if ok_afford => Color::srgba(0.2, 0.88, 0.95, 0.45),
        GhostKind::Upgrade => Color::srgba(0.95, 0.35, 0.2, 0.38),
        GhostKind::Place if ok_afford => Color::srgba(0.25, 0.95, 0.35, 0.42),
        GhostKind::Place => Color::srgba(0.95, 0.35, 0.2, 0.38),
    };
    let mat = materials.add(ColorMaterial::from_color(color));

    let wants_range = matches!(
        kind,
        BuildingKind::SamLauncher | BuildingKind::DefensePost
    );
    let range_mesh = if kind == BuildingKind::SamLauncher {
        Some(ms.sam_range.clone())
    } else if kind == BuildingKind::DefensePost {
        Some(ms.defense_range.clone())
    } else {
        None
    };
    let range_color = materials.add(ColorMaterial::from_color(Color::srgba(
        0.9, 0.9, 0.35, 0.22,
    )));

    if let Some(e) = ghost_parts.marker {
        if commands.get_entity(e).is_ok() {
            commands.entity(e).insert(MeshMaterial2d(mat.clone()));
            commands
                .entity(e)
                .insert(Transform::from_xyz(pos.x, pos.y, 275.0));
        } else {
            ghost_parts.marker = None;
        }
    }

    if ghost_parts.marker.is_none() {
        let e = commands
            .spawn((
                StructureGhostRoot,
                InGameEntity,
                Mesh2d(ms.place_ring.clone()),
                MeshMaterial2d(mat.clone()),
                Transform::from_xyz(pos.x, pos.y, 275.0),
                GlobalTransform::default(),
            ))
            .id();
        ghost_parts.marker = Some(e);
    }

    if wants_range {
        if let Some(rm) = range_mesh {
            if let Some(re) = ghost_parts.range {
                if commands.get_entity(re).is_ok() {
                    commands.entity(re).insert(Mesh2d(rm.clone()));
                    commands
                        .entity(re)
                        .insert(Transform::from_xyz(pos.x, pos.y, 274.0));
                    commands
                        .entity(re)
                        .insert(MeshMaterial2d(range_color.clone()));
                } else {
                    ghost_parts.range = None;
                }
            }
            if ghost_parts.range.is_none() {
                let re = commands
                    .spawn((
                        GhostRangeRing,
                        InGameEntity,
                        Mesh2d(rm),
                        MeshMaterial2d(range_color),
                        Transform::from_xyz(pos.x, pos.y, 274.0),
                        GlobalTransform::default(),
                    ))
                    .id();
                ghost_parts.range = Some(re);
            }
        }
    } else if let Some(re) = ghost_parts.range.take() {
        commands.entity(re).try_despawn();
    }
}

pub fn system_sync_building_visuals(
    mut commands: Commands,
    engine_opt: Option<Res<DarkRiftEngine>>,
    mesh: Option<Res<StructureMarkerMeshes>>,
    settings: Res<ClientSettings>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    q_existing: Query<(Entity, &BuildingVisual)>,
    mut local_existing: Local<Vec<u64>>,
) {
    let Some(engine) = engine_opt.as_deref() else {
        return;
    };
    let Some(marker) = mesh.as_ref() else {
        return;
    };
    let mw = engine.state.map.width;
    let mh = engine.state.map.height;

    local_existing.clear();
    for (_, vis) in &q_existing {
        local_existing.push(vis.0);
    }
    local_existing.sort_unstable();

    for (e, vis) in &q_existing {
        if engine.buildings.binary_search_by_key(&vis.0, |b| b.id).is_err() {
            commands.entity(e).try_despawn();
        }
    }

    for b in engine.buildings.iter() {
        if local_existing.binary_search(&b.id).is_ok() {
            continue;
        }
        let rgb = engine.state
            .player(b.owner_id)
            .map(|p| p.color)
            .unwrap_or([0.7, 0.7, 0.7]);
        let mat = materials.add(ColorMaterial::from_color(Color::srgba(
            rgb[0], rgb[1], rgb[2], 0.88,
        )));
        let pos = tile_idx_to_world(b.tile_idx, mw, mh);
        
        let e = commands.spawn((
            BuildingVisual(b.id),
            BuildingMapVisualRoot,
            InGameEntity,
            Transform::from_xyz(pos.x, pos.y, 268.0),
            GlobalTransform::default(),
            Visibility::default(),
        )).id();
        
        let font = settings.font.clone();
        commands.entity(e).with_children(|ch| {
            ch.spawn((
                InGameEntity,
                Mesh2d(marker.place_ring.clone()),
                MeshMaterial2d(mat),
                Transform::default(),
                GlobalTransform::default(),
                Visibility::default(),
            ));
            ch.spawn((
                InGameEntity,
                Sprite {
                    image: asset_server.load(building_kind_icon_path(b.kind)),
                    custom_size: Some(Vec2::splat(TILE_SIZE * 2.4)),
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, 1.0),
                GlobalTransform::default(),
                Visibility::default(),
            ));
            ch.spawn((
                BuildingMapBadgeRoot(b.id),
                Visibility::Hidden,
                Transform::from_xyz(0.0, TILE_SIZE * 0.42, 2.0),
                GlobalTransform::default(),
            ))
            .with_children(|badge| {
                badge.spawn((
                    InGameEntity,
                    Text2d::new(String::new()),
                    TextFont {
                        font: font.clone(),
                        font_size: 11.0,
                        weight: FontWeight::SEMIBOLD,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    TextLayout::new_with_justify(Justify::Center),
                    Transform::default(),
                    GlobalTransform::default(),
                    Visibility::default(),
                ));
            });
            ch.spawn((
                InGameEntity,
                BuildingLabelTarget(e),
                Text2d::new(format!(
                    "{}{}",
                    building_kind_short(b.kind),
                    b.level
                )),
                TextFont {
                    font,
                    font_size: 11.0,
                    weight: FontWeight::SEMIBOLD,
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::new_with_justify(Justify::Center),
                Transform::from_xyz(0.0, TILE_SIZE * 0.42, 3.0),
            ));
        });
    }
}

pub fn system_sync_building_visual_lod(
    q_camera: Query<&Projection, With<Camera2d>>,
    mut q_visuals: Query<&mut Transform, With<BuildingMapVisualRoot>>,
) {
    let Ok(projection) = q_camera.single() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection else {
        return;
    };

    // OpenFront keeps structure symbols readable by scaling icon stages with zoom.
    // Bevy orthographic scale grows as the camera zooms out, so compensate the
    // world-space visual root only; the deterministic simulation never sees this.
    let visual_scale = (ortho.scale / 1.4).clamp(1.0, 7.0);
    let scale = Vec3::splat(visual_scale);
    for mut transform in &mut q_visuals {
        if transform.scale != scale {
            transform.scale = scale;
        }
    }
}