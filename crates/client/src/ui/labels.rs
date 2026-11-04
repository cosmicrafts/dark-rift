use bevy::prelude::*;
use dark_rift_shared::engine::DarkRiftEngine;
use crate::{
    BuildingMapBadgeRoot, ClientSettings, InGameEntity, TILE_SIZE, player_label_scale,
};
use crate::ui::write_guard::set_text_font_size_if_changed;
use crate::core::EntityPool;

#[derive(Component)]
pub struct PlayerLabel(pub u16);

#[derive(Component)]
pub struct PlayerLabelRenderCache {
    pub font_px: f32,
    pub troops: i64,
}

pub fn system_sync_building_map_labels(
    engine_opt: Option<Res<DarkRiftEngine>>,
    mut q_badge: Query<(&BuildingMapBadgeRoot, &mut Visibility, &Children)>,
    mut q_text: Query<&mut Text2d>,
) {
    let Some(engine) = engine_opt.as_deref() else {
        return;
    };
    for (marker, mut vis, children) in q_badge.iter_mut() {
        let Ok(idx) = engine.buildings.binary_search_by_key(&marker.0, |b| b.id) else {
            continue;
        };
        let b = &engine.buildings[idx];
        let line = match (b.under_construction, b.level) {
            (true, _) => "…".to_string(),
            (false, lvl) if lvl > 1 => lvl.to_string(),
            _ => String::new(),
        };
        let next_vis = if line.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if *vis != next_vis {
            *vis = next_vis;
        }
        for child in children.iter() {
            if let Ok(mut text) = q_text.get_mut(child)
                && text.0 != line {
                    text.0 = line.clone();
                }
        }
    }
}
pub fn system_sync_player_labels(
    mut commands: Commands,
    engine_opt: Option<Res<DarkRiftEngine>>,
    ui_opt: Option<Res<ClientSettings>>,

    mut q_labels: Query<(
        Entity,
        &mut Text2d,
        &mut TextFont,
        &mut Transform,
        &mut TextColor,
        &mut PlayerLabel,
        &mut PlayerLabelRenderCache,
        &mut Visibility,
    )>,
    mut pool: ResMut<EntityPool>,
    q_alive: Query<Entity, With<InGameEntity>>,
) {
    let Some(engine) = engine_opt.as_deref() else { return };
    let game = &engine.state;
    let Some(ui) = ui_opt else { return };

    let alive: std::collections::HashSet<Entity> = q_alive.iter().collect();
    pool.retain_alive(&alive);

    // 1. Recycle labels for dead players
    for (entity, _, _, _, _, label, _, mut vis) in &mut q_labels {
        if label.0 == 0 { continue; } // Already recycled
        let alive = game.players.iter().any(|p| p.id == label.0 && p.alive);
        if !alive {
            pool.set_unused(entity);
            *vis = Visibility::Hidden;
        }
    }

    // 2. Spawn or reuse labels for alive players
    for player in game.players.iter().filter(|p| p.alive) {
        let exists = q_labels
            .iter()
            .any(|(_, _, _, _, _, label, _, _)| label.0 == player.id);
        if !exists {
            let base_font = ui.config.ui.font_size;
            let tiles = player.tile_count;
            let mul = player_label_scale(
                tiles,
                ui.config.ui.label_size_ref_tiles,
                ui.config.ui.label_max_scale,
            );
            let font_px = (base_font * mul).round().max(1.0);
            let label_text = format!("{}\n[{:.0}]", player.name, player.troops);

            let new_entity = pool.get_unused_or_spawn(|| {
                commands.spawn((
                    PlayerLabel(0), // Will be set below
                    InGameEntity,
                    PlayerLabelRenderCache {
                        font_px: 0.0,
                        troops: 0,
                    },
                    Text2d::new(""),
                    TextFont {
                        font: ui.font.clone(),
                        font_size: 10.0,
                        ..default()
                    },
                    TextColor(Color::BLACK),
                    TextLayout::new_with_justify(Justify::Center),
                    Transform::from_xyz(0.0, 0.0, 150.0),
                    Visibility::Inherited,
                )).id()
            });

            if let Ok((_, mut text, mut p_font, mut transform, mut text_color, mut label, mut cache, mut vis)) = q_labels.get_mut(new_entity) {
                label.0 = player.id;
                text.0 = label_text;
                set_text_font_size_if_changed(&mut p_font, font_px);
                cache.font_px = font_px;
                cache.troops = player.troops.round() as i64;
                text_color.0 = Color::BLACK;
                transform.translation.z = 150.0;
                *vis = Visibility::Inherited;
            } else {
                // First frame after spawn, components aren't in query yet. We already set them in the spawn bundle anyway (except player id).
                commands.entity(new_entity).insert((
                    PlayerLabel(player.id),
                    PlayerLabelRenderCache { font_px, troops: player.troops.round() as i64 },
                    Text2d::new(label_text),
                    TextFont { font: ui.font.clone(), font_size: font_px, ..default() },
                    Visibility::Inherited,
                ));
            }
        }
    }

    for (
        _entity,
        mut text,
        mut p_font,
        mut transform,
        mut text_color,
        label,
        mut cache,
        vis,
    ) in q_labels.iter_mut()
    {
        if *vis == Visibility::Hidden || label.0 == 0 { continue; }
        if text_color.0 != Color::BLACK {
            text_color.0 = Color::BLACK;
        }
        let Some(player) = game.player(label.0) else { continue };

        let t_int = player.troops.round() as i64;
        if cache.troops != t_int {
            cache.troops = t_int;
            let new_text = format!("{}\n[{}]", player.name, t_int);
            if text.0 != new_text {
                text.0 = new_text;
            }
        }

        if player.tile_count > 0 {
            let cx = (player.sum_x as f32 / player.tile_count as f32) * TILE_SIZE;
            let cy = (game.map.height as f32 - 1.0 - (player.sum_y as f32 / player.tile_count as f32)) * TILE_SIZE;
            
            let target = Vec3::new(cx, cy + ui.config.ui.label_offset_y, 150.0);
            
            // Just jump directly to target if too far, float smoothly, and snap when close to halt Transform recalculations
            let dist = transform.translation.distance(target);
            if dist > 100.0 {
                transform.translation = target;
            } else if dist > 0.5 {
                transform.translation = transform.translation.lerp(target, 0.2);
            } else if transform.translation != target {
                transform.translation = target;
            }

            let base_font = ui.config.ui.font_size;
            let mul = player_label_scale(
                player.tile_count,
                ui.config.ui.label_size_ref_tiles,
                ui.config.ui.label_max_scale,
            );
            let font_px = (base_font * mul).round().max(1.0);
            if cache.font_px != font_px {
                cache.font_px = font_px;
                set_text_font_size_if_changed(&mut p_font, font_px);
            }
        }
    }
}
