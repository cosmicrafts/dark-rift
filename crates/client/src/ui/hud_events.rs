use bevy::prelude::*;
use bevy::ui::{UiTransform, Val2};
use std::collections::HashMap;
use dark_rift_shared::engine::DarkRiftEngine;
use crate::{ClientSettings, MyPlayerId, clamp_ui_scale, panel_radius_for_breakpoint};
use crate::ui::anim::{ease_in_out_cubic, ease_out_cubic, lerp_f};
use crate::ui::write_guard::{approx_eq_f32, set_display_if_changed, set_text_font_size_if_changed};
use crate::ui::theme::*;
use crate::hud_combat::{CancelFleetButton, CancelAttackButton};
use crate::hud_trunc_name;

use dark_rift_shared::execution::AttackExecution;
use dark_rift_shared::warp_fleet::WarpFleet;
use dark_rift_shared::game::GameEvent;
use crate::EventFeedLine;
use crate::ui::shell::{touch_target_for_breakpoint, UiShellState};

const ROW_ENTER_DURATION: f32 = 0.22;
const ROW_EXIT_DURATION: f32 = 0.18;

#[derive(Component)]
pub struct EventFeedContainer;

#[derive(Resource, Default)]
pub struct EventFeedBuffer {
    pub lines: Vec<Entity>,
}

#[derive(Component)]
pub struct OutgoingRowsContainer;

#[derive(Component)]
pub struct OutgoingAttackRowRoot;

#[derive(Component)]
pub struct OutgoingAttackLineText;

#[derive(Resource, Default)]
pub struct OutgoingAttackRowMap(pub std::collections::HashMap<u64, (Entity, Entity, Entity)>);

#[derive(Component)]
pub struct OutgoingFleetRowRoot;

#[derive(Component)]
pub struct OutgoingFleetLineText;

#[derive(Resource, Default)]
pub struct OutgoingFleetRowMap(pub HashMap<u64, (Entity, Entity, Entity)>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowAnimPhase {
    Entering,
    Exiting,
}

/// Intro / outro motion for attack and fleet rows. Removed after enter completes (steady state).
#[derive(Component)]
pub struct RowAnim {
    pub phase: RowAnimPhase,
    pub t: f32,
    pub exit_bg: Option<Color>,
    pub exit_border: Option<Color>,
}

pub fn system_update_event_feed(
    mut commands: Commands,
    time: Res<Time>,
    engine_opt: Option<Res<DarkRiftEngine>>,
    mut last_event_idx: Local<usize>,
    settings: Res<ClientSettings>,
    q_container: Query<Entity, With<EventFeedContainer>>,
    mut q_lines: Query<(Entity, &mut EventFeedLine, &mut TextColor, &mut Text)>,
    ui_shell: Res<UiShellState>,
    mut buffer: Local<EventFeedBuffer>,
) {
    let Some(container_ent) = q_container.iter().next() else { return };

    // ... EventFeedBuffer logic moved to after event parsing ...

    let Some(engine) = engine_opt.as_deref() else { return };
    let game = &engine.state;
    // Skip event feed log on mobile (attack rows in OutgoingRowsContainer are still shown)
    if matches!(ui_shell.breakpoint, crate::ui::shell::UiBreakpoint::Compact) {
        return; // No scrolling event feed on mobile, but attack rows still show
    }

    // New events
    let current_len = game.events.len();
    if *last_event_idx > current_len {
        *last_event_idx = 0;
    }
    
    // We fetch any new messages
    let mut new_msgs = Vec::new();
    if *last_event_idx < current_len {
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
                GameEvent::TileCaptured { .. } => continue, // Ignore tile captures, too noisy
                _ => continue,
            };
            new_msgs.push(msg);
        }
        *last_event_idx = current_len;
    }

    let s = clamp_ui_scale(settings.config.ui.combat_hud_scale);

    // If buffer isn't initialized, spawn EXACTLY 10 lines
    if buffer.lines.is_empty() {
        commands.entity(container_ent).with_children(|p| {
            for _ in 0..10 {
                let e = p.spawn((
                    EventFeedLine { timer: Timer::from_seconds(0.0, TimerMode::Once) }, // expired
                    Text::new(""),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 14.0 * s,
                        ..default()
                    },
                    TextColor(Color::NONE),
                    Node {
                        margin: UiRect::bottom(Val::Px(4.0 * s)),
                        ..default()
                    },
                )).id();
                buffer.lines.push(e);
            }
        });
        return; // Wait 1 frame for entities to register
    }

    // Tick timers and gather states
    let mut states = Vec::new();
    for &entity in &buffer.lines {
        let Ok((_, mut line, mut color, text)) = q_lines.get_mut(entity) else { continue };
        line.timer.tick(time.delta());
        let alpha = line.timer.fraction_remaining();
        if !approx_eq_f32(color.0.alpha(), alpha) {
            color.0 = color.0.with_alpha(alpha);
        }
        let txt = text.0.clone();
        states.push((txt, alpha, line.timer.clone()));
    }

    // Append new messages, shifting out the oldest
    for msg in new_msgs {
        states.remove(0); // shift
        let mut t = Timer::from_seconds(5.0, TimerMode::Once);
        t.set_elapsed(std::time::Duration::ZERO);
        states.push((msg, 1.0, t));
    }

    // Write back to the UI nodes (zero archetype changes!)
    for (i, &entity) in buffer.lines.iter().enumerate() {
        if let Some(state) = states.get(i) {
            let Ok((_, mut line, mut color, mut t)) = q_lines.get_mut(entity) else { continue };
            if t.0 != state.0 {
                t.0 = state.0.clone();
            }
            line.timer = state.2.clone();
            let mut c = Color::srgba(0.9, 0.9, 0.9, state.1);
            if state.1 <= 0.0 || state.0.is_empty() {
                c = Color::NONE;
            }
            if color.0 != c {
                color.0 = c;
            }
        }
    }
}

fn sync_outgoing_slot_visibility(slot: Entity, children_q: &Query<&Children>, slot_node: &mut Node) {
    let n = children_q.get(slot).map(|c| c.len()).unwrap_or(0);
    let next = if n > 0 {
        Display::Flex
    } else {
        Display::None
    };
    set_display_if_changed(slot_node, next);
}

fn start_row_exit(commands: &mut Commands, row_e: Entity) {
    commands.entity(row_e).insert(RowAnim {
        phase: RowAnimPhase::Exiting,
        t: 0.0,
        exit_bg: None,
        exit_border: None,
    });
}

/// Lobby-style motion for outgoing attack/fleet rows (translate + scale + fade on exit).
pub fn system_animate_outgoing_rows(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<
        (
            Entity,
            &mut UiTransform,
            &mut RowAnim,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Or<(With<OutgoingAttackRowRoot>, With<OutgoingFleetRowRoot>)>,
    >,
) {
    if q.is_empty() {
        return;
    }
    let dt = time.delta_secs();

    for (entity, mut tr, mut anim, mut bg, mut border) in q.iter_mut() {
        match anim.phase {
            RowAnimPhase::Entering => {
                anim.t += dt;
                let u = (anim.t / ROW_ENTER_DURATION).min(1.0);
                let e = ease_out_cubic(u);
                tr.translation = Val2::px(0.0, lerp_f(24.0, 0.0, e));
                tr.scale = Vec2::splat(lerp_f(0.85, 1.0, e));
                if u >= 1.0 {
                    *tr = UiTransform::default();
                    commands.entity(entity).remove::<RowAnim>();
                }
            }
            RowAnimPhase::Exiting => {
                if anim.exit_bg.is_none() {
                    anim.exit_bg = Some(bg.0);
                    anim.exit_border = Some(border.top);
                }
                anim.t += dt;
                let u = (anim.t / ROW_EXIT_DURATION).min(1.0);
                let e = ease_in_out_cubic(u);
                let alpha_mul = lerp_f(1.0, 0.0, e);
                tr.translation = Val2::px(0.0, lerp_f(0.0, 14.0, e));
                tr.scale = Vec2::splat(lerp_f(1.0, 0.9, e));
                if let Some(base) = anim.exit_bg {
                    bg.0 = base.with_alpha(base.alpha() * alpha_mul);
                }
                if let Some(base_b) = anim.exit_border {
                    let c = base_b.with_alpha(base_b.alpha() * alpha_mul);
                    *border = BorderColor::all(c);
                }
                if u >= 1.0 {
                    commands.entity(entity).try_despawn();
                }
            }
        }
    }
}

pub fn system_sync_outgoing_attack_rows(
    mut commands: Commands,
    engine_opt: Option<Res<DarkRiftEngine>>,
    mut slot_q: Query<(Entity, &mut Node), With<OutgoingRowsContainer>>,
    children_q: Query<&Children>,
    my_id: Res<MyPlayerId>,
    settings: Res<ClientSettings>,
    ui_shell: Res<UiShellState>,
    mut row_map: ResMut<OutgoingAttackRowMap>,
    mut fleet_row_map: ResMut<OutgoingFleetRowMap>,
    mut q_attack_line: Query<(&mut Text, &mut TextFont), (With<OutgoingAttackLineText>, Without<OutgoingFleetLineText>)>,
    mut q_fleet_line: Query<(&mut Text, &mut TextFont), (With<OutgoingFleetLineText>, Without<OutgoingAttackLineText>)>,
) {
    let Ok((slot, mut slot_node)) = slot_q.single_mut() else {
        return;
    };

    if my_id.0 == 0 {
        for (_, (row_e, _, _)) in row_map.0.drain() {
            commands.entity(row_e).try_despawn();
        }
        for (_, (row_e, _, _)) in fleet_row_map.0.drain() {
            commands.entity(row_e).try_despawn();
        }
        sync_outgoing_slot_visibility(slot, &children_q, &mut slot_node);
        return;
    }
    let Some(engine) = engine_opt.as_deref() else {
        return;
    };
    let game = &engine.state;

    let s = clamp_ui_scale(settings.config.ui.combat_hud_scale);
    let row_font = 11.5 * s;
    let cancel_icon_font = 14.0 * s;
    let (tw, th) = touch_target_for_breakpoint(ui_shell.breakpoint);
    let cancel_w = tw * s;
    let cancel_h = th * s;
    let row_radius = panel_radius_for_breakpoint(ui_shell.breakpoint) * s;

    let outgoing: Vec<&AttackExecution> = engine
        .attacks
        .iter()
        .rev() // Newest attacks first (engine array is sorted ascending by ID)
        .filter(|e| e.owner_id == my_id.0 && !e.retreating)
        .collect();

    let stale: Vec<u64> = row_map
        .0
        .keys()
        .copied()
        .filter(|id| outgoing.binary_search_by(|e| e.id.cmp(id).reverse()).is_err())
        .collect();
    for id in stale {
        if let Some((row_e, _, _)) = row_map.0.remove(&id) {
            start_row_exit(&mut commands, row_e);
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

        let to_text = format!("To: {}", who);
        let count_text = fmt_num(exec.troops.max(0.0));

        if let Some((_, text_e1, text_e2)) = row_map.0.get(&exec.id).copied() {
            if let Ok((mut t1, mut tf1)) = q_attack_line.get_mut(text_e1) {
                if **t1 != to_text {
                    **t1 = to_text;
                }
                set_text_font_size_if_changed(&mut tf1, row_font);
            }
            if let Ok((mut t2, mut tf2)) = q_attack_line.get_mut(text_e2) {
                if **t2 != count_text {
                    **t2 = count_text;
                }
                set_text_font_size_if_changed(&mut tf2, row_font);
            }
        } else {
            let font = settings.font.clone();
            let exec_id = exec.id;
            let mut tr = UiTransform::from_translation(Val2::px(0.0, 24.0));
            tr.scale = Vec2::splat(0.85);
            let row_e = commands
                .spawn((
                    ChildOf(slot),
                    OutgoingAttackRowRoot,
                    tr,
                    RowAnim {
                        phase: RowAnimPhase::Entering,
                        t: 0.0,
                        exit_bg: None,
                        exit_border: None,
                    },
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(if matches!(ui_shell.breakpoint, crate::ui::shell::UiBreakpoint::Compact) { 34.0 * s } else { 44.0 * s }),
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(8.0 * s), Val::Px(4.0 * s)),
                        border: UiRect::all(Val::Px(1.0 * s)),
                        border_radius: BorderRadius::all(Val::Px(row_radius)),
                        column_gap: Val::Px(6.0 * s),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    BackgroundColor(panel_bg()),
                    BorderColor::all(panel_border()),
                ))
                .id();

            let text_e1 = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingAttackLineText,
                    Text::new(to_text),
                    TextFont {
                        font: font.clone(),
                        font_size: row_font,
                        ..default()
                    },
                    TextColor(text_primary()),
                    TextLayout::new_with_justify(Justify::Left),
                    Node {
                        flex_grow: 1.0,
                        align_self: AlignSelf::Center,
                        ..default()
                    },
                ))
                .id();

            let text_e2 = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingAttackLineText,
                    Text::new(count_text),
                    TextFont {
                        font: font.clone(),
                        font_size: row_font,
                        ..default()
                    },
                    TextColor(Color::srgba(0.9, 0.7, 0.1, 1.0)), // Golden count
                    TextLayout::new_with_justify(Justify::Right),
                    Node {
                        align_self: AlignSelf::Center,
                        margin: UiRect::right(Val::Px(8.0 * s)),
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

            row_map.0.insert(exec_id, (row_e, text_e1, text_e2));
        }
    }

    // Outgoing boats (`WarpFleet`) — OpenFront `AttacksDisplay` transport rows + recall.
    if !settings.config.ui.combat_hud_fleet_rows {
        for (_, (row_e, _, _)) in fleet_row_map.0.drain() {
            commands.entity(row_e).try_despawn();
        }
        sync_outgoing_slot_visibility(slot, &children_q, &mut slot_node);
        return;
    }

    let fleets: Vec<&WarpFleet> = engine
        .fleets
        .iter()
        .rev() // Newest fleets first
        .filter(|f| f.owner_id == my_id.0 && !f.retreating)
        .collect();

    let stale_f: Vec<u64> = fleet_row_map
        .0
        .keys()
        .copied()
        .filter(|id| fleets.binary_search_by(|e| e.id.cmp(id).reverse()).is_err())
        .collect();
    for id in stale_f {
        if let Some((row_e, _, _)) = fleet_row_map.0.remove(&id) {
            start_row_exit(&mut commands, row_e);
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

        let to_text = format!("Fleet To: {}", who);
        let count_text = fmt_num(fleet.troops.max(0.0));

        if let Some((_, text_e1, text_e2)) = fleet_row_map.0.get(&fleet.id).copied() {
            if let Ok((mut t1, mut tf1)) = q_fleet_line.get_mut(text_e1) {
                if **t1 != to_text {
                    **t1 = to_text;
                }
                set_text_font_size_if_changed(&mut tf1, row_font);
            }
            if let Ok((mut t2, mut tf2)) = q_fleet_line.get_mut(text_e2) {
                if **t2 != count_text {
                    **t2 = count_text;
                }
                set_text_font_size_if_changed(&mut tf2, row_font);
            }
        } else {
            let font = settings.font.clone();
            let fleet_id = fleet.id;
            let mut tr = UiTransform::from_translation(Val2::px(0.0, 24.0));
            tr.scale = Vec2::splat(0.85);
            let row_e = commands
                .spawn((
                    ChildOf(slot),
                    OutgoingFleetRowRoot,
                    tr,
                    RowAnim {
                        phase: RowAnimPhase::Entering,
                        t: 0.0,
                        exit_bg: None,
                        exit_border: None,
                    },
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(44.0 * s),
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(12.0 * s), Val::Px(6.0 * s)),
                        border: UiRect::all(Val::Px(1.0 * s)),
                        border_radius: BorderRadius::all(Val::Px(row_radius)),
                        column_gap: Val::Px(8.0 * s),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.06, 0.1, 0.16, 0.85)),
                    BorderColor::all(Color::srgba(0.2, 0.4, 0.8, 0.6)),
                ))
                .id();

            let text_e1 = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingFleetLineText,
                    Text::new(to_text),
                    TextFont {
                        font: font.clone(),
                        font_size: row_font,
                        ..default()
                    },
                    TextColor(Color::srgba(0.8, 0.9, 1.0, 1.0)),
                    TextLayout::new_with_justify(Justify::Left),
                    Node {
                        flex_grow: 1.0,
                        align_self: AlignSelf::Center,
                        ..default()
                    },
                ))
                .id();

            let text_e2 = commands
                .spawn((
                    ChildOf(row_e),
                    OutgoingFleetLineText,
                    Text::new(count_text),
                    TextFont {
                        font: font.clone(),
                        font_size: row_font,
                        ..default()
                    },
                    TextColor(Color::srgba(0.9, 0.7, 0.1, 1.0)),
                    TextLayout::new_with_justify(Justify::Right),
                    Node {
                        align_self: AlignSelf::Center,
                        margin: UiRect::right(Val::Px(8.0 * s)),
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

            fleet_row_map.0.insert(fleet_id, (row_e, text_e1, text_e2));
        }
    }

    sync_outgoing_slot_visibility(slot, &children_q, &mut slot_node);
}
