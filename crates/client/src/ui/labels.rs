use bevy::prelude::*;
use dark_rift_shared::engine::DarkRiftEngine;
use crate::{
    BuildingMapBadgeRoot, ClientSettings, InGameEntity, TILE_SIZE, player_label_scale,
};
use crate::ui::write_guard::set_text_font_size_if_changed;

#[derive(Component)]
pub struct PlayerLabel(pub u16);

#[derive(Component)]
pub struct PlayerLabelRenderCache(pub f32);

pub fn system_sync_building_map_labels(
    engine_opt: Option<Res<DarkRiftEngine>>,
    mut q_badge: Query<(&BuildingMapBadgeRoot, &mut Visibility, &Children)>,
    mut q_text: Query<&mut Text2d>,
) {
    let Some(engine) = engine_opt.as_deref() else {
        return;
    };
    for (marker, mut vis, children) in q_badge.iter_mut() {
        let Some(b) = engine.buildings.iter().find(|b| b.id == marker.0) else {
            continue;
        };
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
            if let Ok(mut text) = q_text.get_mut(child) {
                if text.0 != line {
                    text.0 = line.clone();
                }
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
        &PlayerLabel,
        &mut PlayerLabelRenderCache,
    )>,
) {
    let Some(engine) = engine_opt.as_deref() else { return };
    let game = &engine.state;
    let Some(ui) = ui_opt else { return };

    // 1. Despawn labels for dead players
    for (entity, _, _, _, _, label, _) in &q_labels {
        let alive = game.players.iter().any(|p| p.id == label.0 && p.alive);
        if !alive {
            commands.entity(entity).try_despawn();
        }
    }

    // 2. Spawn labels for alive players if they don't exist
    for player in game.players.iter().filter(|p| p.alive) {
        let exists = q_labels
            .iter()
            .any(|(_, _, _, _, _, label, _)| label.0 == player.id);
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
            commands.spawn((
                PlayerLabel(player.id),
                InGameEntity,
                PlayerLabelRenderCache(font_px),
                Text2d::new(label_text),
                TextFont {
                    font: ui.font.clone(),
                    font_size: font_px,
                    ..default()
                },
                TextColor(Color::BLACK),
                TextLayout::new_with_justify(Justify::Center),
                Transform::from_xyz(0.0, 0.0, 150.0),
            ));
        }
    }

    // 3. Update positions and text based on territory center of mass
    for (
        _entity,
        mut text,
        mut p_font,
        mut transform,
        mut text_color,
        label,
        mut cache,
    ) in q_labels.iter_mut()
    {
        if text_color.0 != Color::BLACK {
            text_color.0 = Color::BLACK;
        }
        let Some(player) = game.player(label.0) else { continue };

        let new_text = format!("{}\n[{:.0}]", player.name, player.troops);
        if text.0 != new_text {
            text.0 = new_text.clone();
        }

        if player.tile_count > 0 {
            let cx = (player.sum_x as f32 / player.tile_count as f32) * TILE_SIZE;
            let cy = (game.map.height as f32 - 1.0 - (player.sum_y as f32 / player.tile_count as f32)) * TILE_SIZE;
            
            let target = Vec3::new(cx, cy + ui.config.ui.label_offset_y, 150.0);
            
            // Just jump directly to target if too far, otherwise float smoothly
            if transform.translation.distance(target) > 100.0 {
                transform.translation = target;
            } else {
                transform.translation = transform.translation.lerp(target, 0.2);
            }

            let base_font = ui.config.ui.font_size;
            let mul = player_label_scale(
                player.tile_count,
                ui.config.ui.label_size_ref_tiles,
                ui.config.ui.label_max_scale,
            );
            let font_px = (base_font * mul).round().max(1.0);
            if cache.0 != font_px {
                cache.0 = font_px;
                set_text_font_size_if_changed(&mut p_font, font_px);
            }
        }
    }
}
