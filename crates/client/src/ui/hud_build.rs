use bevy::prelude::*;
use dark_rift_shared::game::GameState;
use dark_rift_shared::game::BuildingKind;
use dark_rift_shared::building::{BuildableEntry, compute_buildables_for_player, patch_buildable_entries_gold_and_counts};
use crate::{MyPlayerId, OverlayDiagnostics, Building};
use crate::core::BuildStructureMode;
use crate::ui::theme::*;

#[derive(Resource, Default)]
pub struct BuildableHudState {
    entries: Vec<BuildableEntry>,
}

#[derive(Component)]
pub struct BuildPillRow;

#[derive(Component)]
pub struct BuildPillButton(pub BuildingKind);

#[derive(Component, Clone, Copy)]
pub struct BuildPillLabel {
    /// 1 = level total, 2 = gold cost, 3 = hotkey digit (icon is separate `ImageNode`).
    pub slot: u8,
    pub kind: BuildingKind,
}


pub fn system_build_structure_hotkeys(
    kb: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<BuildStructureMode>,
    mut overlay: ResMut<OverlayDiagnostics>,
) {
    if kb.just_pressed(KeyCode::Escape) {
        mode.0 = None;
    }
    let keys = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
    ];
    for (i, key) in keys.iter().enumerate() {
        if kb.just_pressed(*key) {
            mode.0 = Some(BuildingKind::ALL[i]);
            break;
        }
    }
    overlay.build_hint = match mode.0 {
        None => String::new(),
        Some(k) => format!(
            "{} · 1–6 · Esc · click own tile (pays gold)",
            k.as_str()
        ),
    };
}

pub fn system_refresh_buildable_hud(
    game_opt: Option<Res<GameState>>,
    my_id: Res<MyPlayerId>,
    q_building: Query<&Building>,
    mut removed: RemovedComponents<Building>,
    added: Query<(), Added<Building>>,
    mut state: ResMut<BuildableHudState>,
    mut last_gold: Local<Option<f64>>,
) {
    let Some(game) = game_opt.as_deref() else {
        return;
    };
    let pid = my_id.0;
    if pid == 0 {
        return;
    }
    let Some(p) = game.player(pid) else {
        return;
    };
    let buildings_changed = removed.read().count() > 0 || added.iter().next().is_some();
    let gold_changed = last_gold.map(|g| (p.gold - g).abs() > 0.5).unwrap_or(true);
    let buildings: Vec<Building> = q_building.iter().copied().collect();

    if buildings_changed || state.entries.len() != BuildingKind::ALL.len() {
        state.entries = compute_buildables_for_player(&game.map, pid, p.gold, &buildings)
            .into_iter()
            .collect();
        *last_gold = Some(p.gold);
        return;
    }

    if gold_changed && state.entries.len() == BuildingKind::ALL.len() {
        patch_buildable_entries_gold_and_counts(&mut state.entries, pid, p.gold, &buildings);
        *last_gold = Some(p.gold);
    }
}

pub fn system_apply_buildable_hud(
    state: Res<BuildableHudState>,
    mode: Res<BuildStructureMode>,
    mut q_btn: Query<
        (
            &BuildPillButton,
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<Button>,
    >,
    mut q_label: Query<(&BuildPillLabel, &mut Text)>,
) {
    if state.entries.len() != BuildingKind::ALL.len() {
        return;
    }
    for (label, mut text) in q_label.iter_mut() {
        let Some(entry) = state.entries.iter().find(|e| e.kind == label.kind) else {
            continue;
        };
        let hk = BuildingKind::ALL
            .iter()
            .position(|&k| k == entry.kind)
            .map(|i: usize| (i + 1).to_string())
            .unwrap_or_else(|| "?".into());
        let s = match label.slot {
            1 => format!("{}", entry.level_total),
            2 => format!("{:.0}", entry.cost),
            3 => hk,
            _ => continue,
        };
        if **text != s {
            **text = s;
        }
    }
    for (btn, interaction, mut bg, mut border) in q_btn.iter_mut() {
        let Some(entry) = state.entries.iter().find(|e| e.kind == btn.0) else {
            continue;
        };
        let disabled = !entry.can_build && !entry.can_upgrade;
        let sel = mode.0 == Some(btn.0);
        let (target_bg, target_border) = if disabled {
            (
                Color::srgba(0.1, 0.11, 0.13, 0.38).into(),
                BorderColor::all(Color::srgba(0.3, 0.32, 0.36, 0.35)),
            )
        } else if sel {
            (
                accent_primary().with_alpha(0.42).into(),
                BorderColor::all(accent_primary()),
            )
        } else if *interaction == Interaction::Hovered {
            (
                panel_bg().with_alpha(0.92).into(),
                BorderColor::all(panel_border()),
            )
        } else {
            (panel_bg().into(), BorderColor::all(panel_border()))
        };
        // Avoid writing identical colors every frame — native UI relayouts on any Node/BackgroundColor touch.
        if bg.0 != target_bg {
            bg.0 = target_bg;
        }
        if *border != target_border {
            *border = target_border;
        }
    }
}

pub fn system_buildpill_interactions(
    q: Query<(&Interaction, &BuildPillButton), (Changed<Interaction>, With<Button>)>,
    mut mode: ResMut<BuildStructureMode>,
    mut overlay: ResMut<OverlayDiagnostics>,
) {
    for (ix, btn) in &q {
        if *ix == Interaction::Pressed {
            mode.0 = Some(btn.0);
            overlay.build_hint = format!("Place {} — click own tile (gold) · Esc", btn.0.as_str());
        }
    }
}