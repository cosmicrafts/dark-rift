use bevy::prelude::*;
use std::collections::HashMap;
use dark_rift_shared::game::GameState;
use crate::{ClientSettings, MyPlayerId, clamp_ui_scale, panel_radius_for_breakpoint};
use crate::ui::theme::*;
use crate::hud_combat::{CancelFleetButton, CancelAttackButton};
use crate::hud_trunc_name;
use crate::WarpFleet;
use std::collections::HashSet;
use dark_rift_shared::execution::AttackExecution;
use dark_rift_shared::game::GameEvent;
use crate::EventFeedLine;
use crate::ui::shell::{touch_target_for_breakpoint, UiShellState};

#[derive(Component)]
pub struct EventFeedContainer;

#[derive(Component)]
pub struct OutgoingRowsContainer;

#[derive(Component)]
pub struct OutgoingAttackRowRoot;

#[derive(Component)]
pub struct OutgoingAttackLineText;

#[derive(Resource, Default)]
pub struct OutgoingAttackRowMap(pub std::collections::HashMap<u64, (Entity, Entity)>);

#[derive(Component)]
pub struct OutgoingFleetRowRoot;

#[derive(Component)]
pub struct OutgoingFleetLineText;

#[derive(Resource, Default)]
pub struct OutgoingFleetRowMap(pub HashMap<u64, (Entity, Entity)>);

pub fn system_update_event_feed(
    mut commands: Commands,
    time: Res<Time>,
    game_opt: Option<Res<GameState>>,
    mut last_event_idx: Local<usize>,
    settings: Res<ClientSettings>,
    q_container: Query<Entity, With<EventFeedContainer>>,
    mut q_lines: Query<(Entity, &mut EventFeedLine, &mut TextColor)>,
) {
    let Some(container_ent) = q_container.iter().next() else { return };

    for (entity, mut line, mut color) in q_lines.iter_mut() {
        line.timer.tick(time.delta());
        let alpha = line.timer.fraction_remaining();
        color.0.set_alpha(alpha);
        if alpha <= 0.0 {
            commands.entity(entity).despawn();
        }
    }

    let Some(game) = game_opt else { return };

    // New events
    let current_len = game.events.len();
    if *last_event_idx < current_len {
        let s = clamp_ui_scale(settings.config.ui.combat_hud_scale);
        
        for i in *last_event_idx..current_len {
            let ev = &game.events[i];
            let msg = match ev {
                GameEvent::PlayerEliminated { player_id } => {
                    let name = game.player(*player_id).map(|p| p.name.as_str()).unwrap_or("Unknown");
                    format!("Eliminated: {}", name)
                }
                GameEvent::StructureSpawned { owner_id, kind, .. } => {
                    let name = game.player(*owner_id).map(|p| p.name.as_str()).unwrap_or("Unknown");
                    format!("Built: {} {}", name, kind.as_str())
                }
                GameEvent::StructureUpgraded { kind, level, .. } => {
                    format!("Upgraded: {} to Lvl {}", kind.as_str(), level)
                }
                GameEvent::TileCaptured { .. } => {
                    // Ignore tile captures, too noisy
                    continue;
                }
                _ => continue,
            };

            commands.entity(container_ent).with_children(|p| {
                p.spawn((
                    EventFeedLine { timer: Timer::from_seconds(5.0, TimerMode::Once) },
                    Text::new(msg),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 14.0 * s,
                        ..default()
                    },
                    TextColor(Color::srgba(0.9, 0.9, 0.9, 1.0)),
                    Node {
                        margin: UiRect::bottom(Val::Px(4.0 * s)),
                        ..default()
                    },
                ));
            });
        }
        *last_event_idx = current_len;
    }
}

pub fn system_sync_outgoing_attack_rows(
    mut commands: Commands,
    game_opt: Option<Res<GameState>>,
    slot_q: Query<Entity, With<OutgoingRowsContainer>>,
    q_exec: Query<&AttackExecution>,
    q_fleet: Query<&WarpFleet>,
    my_id: Res<MyPlayerId>,
    settings: Res<ClientSettings>,
    ui_shell: Res<UiShellState>,
    mut row_map: ResMut<OutgoingAttackRowMap>,
    mut fleet_row_map: ResMut<OutgoingFleetRowMap>,
    mut q_attack_line: Query<(&mut Text, &mut TextFont), (With<OutgoingAttackLineText>, Without<OutgoingFleetLineText>)>,
    mut q_fleet_line: Query<(&mut Text, &mut TextFont), (With<OutgoingFleetLineText>, Without<OutgoingAttackLineText>)>,
) {
    let Ok(slot) = slot_q.single() else {
        return;
    };

    if my_id.0 == 0 {
        for (_, (row_e, _)) in row_map.0.drain() {
            commands.entity(row_e).despawn();
        }
        for (_, (row_e, _)) in fleet_row_map.0.drain() {
            commands.entity(row_e).despawn();
        }
        return;
    }
    let Some(game) = game_opt else {
        return;
    };

    let s = clamp_ui_scale(settings.config.ui.combat_hud_scale);
    let row_font = 11.5 * s;
    let cancel_icon_font = 14.0 * s;
    let (tw, th) = touch_target_for_breakpoint(ui_shell.breakpoint);
    let cancel_w = tw * s;
    let cancel_h = th * s;
    let row_radius = panel_radius_for_breakpoint(ui_shell.breakpoint) * s;

    let mut outgoing: Vec<&AttackExecution> = q_exec
        .iter()
        .filter(|e| e.owner_id == my_id.0 && !e.retreating)
        .collect();
    outgoing.sort_by_key(|e| e.id);

    let desired: HashSet<u64> = outgoing.iter().map(|e| e.id).collect();

    let stale: Vec<u64> = row_map
        .0
        .keys()
        .copied()
        .filter(|id| !desired.contains(id))
        .collect();
    for id in stale {
        if let Some((row_e, _)) = row_map.0.remove(&id) {
            commands.entity(row_e).despawn();
        }
    }

    for exec in outgoing {
        let who = if exec.target_owner == 0 {
            "Neutral".to_string()
        } else {
            game
                .player(exec.target_owner)
                .map(|p| hud_trunc_name(&p.name, 14))
                .unwrap_or_else(|| format!("{}", exec.target_owner))
        };
        let fmt_num = |v: f64| -> String {
            if v >= 10_000.0 {
                format!("{:.1}k", v / 1000.0)
            } else {
                format!("{:.0}", v)
            }
        };
        
        let line = format!(
            "To: {}\n{} / {}",
            who,
            fmt_num(exec.troops.max(0.0)),
            fmt_num(exec.initial_troops.max(0.0)),
        );

        if let Some((_, text_e)) = row_map.0.get(&exec.id).copied() {
            if let Ok((mut t, mut tf)) = q_attack_line.get_mut(text_e) {
                if **t != line {
                    **t = line;
                }
                tf.font_size = row_font;
            }
        } else {
            let font = settings.font.clone();
            let exec_id = exec.id;
            let row_e = commands
                .spawn((
                    ChildOf(slot),
                    OutgoingAttackRowRoot,
                    Node {
                        width: Val::Px(120.0 * s),
                        height: Val::Px(80.0 * s),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(4.0 * s), Val::Px(6.0 * s)),
                        border: UiRect::all(Val::Px(1.0 * s)),
                        border_radius: BorderRadius::all(Val::Px(row_radius)),
                        ..default()
                    },
                    BackgroundColor(panel_bg()),
                    BorderColor::all(panel_border()),
                ))
                .id();

            let text_e = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingAttackLineText,
                    Text::new(line),
                    TextFont {
                        font: font.clone(),
                        font_size: row_font,
                        ..default()
                    },
                    TextColor(text_primary()),
                    TextLayout::new_with_justify(Justify::Center),
                    Node {
                        flex_grow: 1.0,
                        align_self: AlignSelf::Center,
                        ..default()
                    },
                ))
                .id();

            commands
                .spawn((
                    ChildOf(row_e),
                    Button,
                    CancelAttackButton(exec_id),
                    Node {
                        width: Val::Px(cancel_w),
                        height: Val::Px(cancel_h),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(Val::Px(8.0 * s)),
                        ..default()
                    },
                    BackgroundColor(accent_danger()),
                    BorderColor::all(accent_danger_border()),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("X"),
                        TextFont {
                            font,
                            font_size: cancel_icon_font,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });

            row_map.0.insert(exec_id, (row_e, text_e));
        }
    }

    // Outgoing boats (`WarpFleet`) — OpenFront `AttacksDisplay` transport rows + recall.
    if !settings.config.ui.combat_hud_fleet_rows {
        for (_, (row_e, _)) in fleet_row_map.0.drain() {
            commands.entity(row_e).despawn();
        }
        return;
    }

    let mut fleets: Vec<&WarpFleet> = q_fleet
        .iter()
        .filter(|f| f.owner_id == my_id.0 && !f.retreating)
        .collect();
    fleets.sort_by_key(|f| f.id);

    let desired_f: HashSet<u64> = fleets.iter().map(|f| f.id).collect();
    let stale_f: Vec<u64> = fleet_row_map
        .0
        .keys()
        .copied()
        .filter(|id| !desired_f.contains(id))
        .collect();
    for id in stale_f {
        if let Some((row_e, _)) = fleet_row_map.0.remove(&id) {
            commands.entity(row_e).despawn();
        }
    }

    for fleet in fleets {
        let who = if fleet.target_owner == 0 {
            "Neutral".to_string()
        } else {
            game
                .player(fleet.target_owner)
                .map(|p| hud_trunc_name(&p.name, 14))
                .unwrap_or_else(|| format!("{}", fleet.target_owner))
        };
        let fmt_num = |v: f64| -> String {
            if v >= 10_000.0 {
                format!("{:.1}k", v / 1000.0)
            } else {
                format!("{:.0}", v)
            }
        };

        let line = format!(
            "⛵ To: {}\n{}",
            who,
            fmt_num(fleet.troops.max(0.0)),
        );

        if let Some((_, text_e)) = fleet_row_map.0.get(&fleet.id).copied() {
            if let Ok((mut t, mut tf)) = q_fleet_line.get_mut(text_e) {
                if **t != line {
                    **t = line;
                }
                tf.font_size = row_font;
            }
        } else {
            let font = settings.font.clone();
            let fleet_id = fleet.id;
            let row_e = commands
                .spawn((
                    ChildOf(slot),
                    OutgoingFleetRowRoot,
                    Node {
                        width: Val::Px(120.0 * s),
                        height: Val::Px(80.0 * s),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(4.0 * s), Val::Px(6.0 * s)),
                        border: UiRect::all(Val::Px(1.0 * s)),
                        border_radius: BorderRadius::all(Val::Px(row_radius)),
                        ..default()
                    },
                    BackgroundColor(panel_bg()),
                    BorderColor::all(panel_border()),
                ))
                .id();

            let text_e = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingFleetLineText,
                    Text::new(line),
                    TextFont {
                        font: font.clone(),
                        font_size: row_font,
                        ..default()
                    },
                    TextColor(text_primary()),
                    TextLayout::new_with_justify(Justify::Center),
                    Node {
                        flex_grow: 1.0,
                        align_self: AlignSelf::Center,
                        ..default()
                    },
                ))
                .id();

            commands
                .spawn((
                    ChildOf(row_e),
                    Button,
                    CancelFleetButton(fleet_id),
                    Node {
                        width: Val::Px(cancel_w),
                        height: Val::Px(cancel_h),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(Val::Px(8.0 * s)),
                        ..default()
                    },
                    BackgroundColor(accent_danger()),
                    BorderColor::all(accent_danger_border()),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("X"),
                        TextFont {
                            font,
                            font_size: cancel_icon_font,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });

            fleet_row_map.0.insert(fleet_id, (row_e, text_e));
        }
    }
}