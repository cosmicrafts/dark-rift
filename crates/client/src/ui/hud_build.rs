use bevy::prelude::*;
use dark_rift_shared::game::GameState;
use dark_rift_shared::game::BuildingKind;
use dark_rift_shared::building::{BuildableEntry, compute_buildables_for_player, patch_buildable_entries_gold_and_counts};
use crate::{MyPlayerId, OverlayDiagnostics};
use crate::core::BuildStructureMode;
use crate::ui::theme::*;

#[derive(Resource, Default)]
pub struct BuildableHudState {
    entries: Vec<BuildableEntry>,
}

/// Set by [`system_refresh_buildable_hud`] when buildable entries change; consumed by apply.
#[derive(Resource, Default)]
pub struct BuildableHudDirty(pub bool);

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
    let next_hint = match mode.0 {
        None => String::new(),
        Some(k) => format!(
            "{} · 1–6 · Esc · click own tile (pays gold)",
            k.as_str()
        ),
    };
    if overlay.build_hint != next_hint {
        overlay.build_hint = next_hint;
    }
}

pub fn system_refresh_buildable_hud(
    game_opt: Option<Res<GameState>>,
    my_id: Res<MyPlayerId>,
    mut state: ResMut<BuildableHudState>,
    mut dirty: ResMut<BuildableHudDirty>,
    mut last_gold: Local<Option<f64>>,
    mut last_buildings_len: Local<Option<usize>>,
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
    
    let current_len = game.buildings.len();
    let buildings_changed = last_buildings_len.map_or(true, |l| l != current_len);
    let gold_changed = last_gold.map(|g| (p.gold - g).abs() > 0.5).unwrap_or(true);

    if buildings_changed || state.entries.len() != BuildingKind::ALL.len() {
        state.entries = compute_buildables_for_player(&game.map, pid, p.gold, &game.buildings)
            .into_iter()
            .collect();
        dirty.0 = true;
        *last_gold = Some(p.gold);
        *last_buildings_len = Some(current_len);
        return;
    }

    if gold_changed && state.entries.len() == BuildingKind::ALL.len() {
        patch_buildable_entries_gold_and_counts(&mut state.entries, pid, p.gold, &game.buildings);
        dirty.0 = true;
        *last_gold = Some(p.gold);
    }
}

pub fn system_apply_buildable_hud(
    state: Res<BuildableHudState>,
    mode: Res<BuildStructureMode>,
    mut dirty: ResMut<BuildableHudDirty>,
    q_ix: Query<(), (With<BuildPillButton>, Changed<Interaction>, With<Button>)>,
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
    let labels_need = dirty.0;
    let pills_need = dirty.0 || mode.is_changed() || q_ix.iter().next().is_some();
    if !labels_need && !pills_need {
        return;
    }
    if labels_need {
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
    }
    if pills_need {
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
    if dirty.0 {
        dirty.0 = false;
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
            let h = format!("Place {} — click own tile (gold) · Esc", btn.0.as_str());
            if overlay.build_hint != h {
                overlay.build_hint = h;
            }
        }
    }
}