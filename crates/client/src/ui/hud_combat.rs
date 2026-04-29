use bevy::prelude::*;
use dark_rift_shared::game::GameState;
use dark_rift_shared::protocol::*;
use crate::{ClientSettings, MyPlayerId, clamp_ui_scale};
use lightyear::prelude::*;
use crate::ui::theme::*;
use crate::ui::shell::{UiBreakpoint, UiShellState};
use crate::{ATTACK_RATIO_KEYSTEP, AppState, JoinedLobby, PlayerLabel, OutgoingRowsContainer, home_button_size_for_breakpoint, home_button_width_for_breakpoint};
use bevy::ui::RelativeCursorPosition;

#[derive(Component)]
pub struct BottomHudRoot;

#[derive(Component)]
pub struct BottomHudTroops;

#[derive(Component)]
pub struct TroopsBarShell;

#[derive(Component)]
pub struct TroopsBarFill;

#[derive(Component)]
pub struct TroopsBarDamage;

#[derive(Component)]
pub struct BottomHudGold;

#[derive(Component)]
pub struct BottomHudPanel;

#[derive(Component)]
pub struct AttackRatioTrack;

#[derive(Component)]
pub struct AttackRatioFill;

#[derive(Component)]
pub struct AttackRatioHandle;

#[derive(Component)]
pub struct AttackRatioReadout;

#[derive(Component)]
pub struct AttackRatioBarShell;

#[derive(Resource, Default)]
pub struct AttackRatio(pub f32);

#[derive(Component)]
pub struct HomeButton;

#[derive(Component)]
pub struct ExitButton;

#[derive(Component)]
pub struct SettingsButton;

#[derive(Component)]
pub struct DevStatsPill;

#[derive(Component)]
pub struct CancelAttackButton(pub u64);

#[derive(Component)]
pub struct CancelFleetButton(pub u64);

pub fn system_apply_responsive_hud_layout(
    ui_shell: Res<UiShellState>,
    settings: Res<crate::ClientSettings>,
    mut nodes: ParamSet<(
        Query<(&mut Node, &mut BackgroundColor), With<BottomHudRoot>>,   // p0
        Query<&mut Node, With<AttackRatioTrack>>,                         // p1
        Query<&mut Node, With<OutgoingRowsContainer>>,                    // p2
        Query<&mut Node, (With<HomeButton>, With<Button>)>,               // p3
        Query<&mut Node, (With<ExitButton>, With<Button>)>,               // p4
        Query<(&mut BackgroundColor, &mut BorderColor, &mut Node), With<crate::ui::hud_build::BuildPillRow>>,  // p5
        Query<(&mut BackgroundColor, &mut BorderColor, &mut Node), With<BottomHudPanel>>, // p6
        Query<&mut Node, With<crate::ui::hud_build::BuildPillButton>>,    // p7
    )>,
) {
    let compact = matches!(ui_shell.breakpoint, UiBreakpoint::Compact);

    {
        let mut root_q = nodes.p0();
        let Ok((mut root_node, mut root_bg)) = root_q.single_mut() else {
            return;
        };
        if compact {
            root_node.row_gap = Val::Px(0.0);
            root_node.padding = UiRect::ZERO;
            root_bg.0 = Color::srgba(0.0, 0.0, 0.0, 0.55);
        } else {
            root_node.row_gap = Val::Px(6.0);
            root_node.padding = UiRect::bottom(Val::Px(16.0 * crate::clamp_ui_scale(settings.config.ui.combat_hud_scale)));
            root_bg.0 = Color::NONE;
        }
    }

    {
        let mut track_q = nodes.p1();
        let Ok(mut track_node) = track_q.single_mut() else {
            return;
        };
        track_node.height = Val::Px(if compact { 18.0 } else { 12.0 });
    }

    {
        let mut outgoing_q = nodes.p2();
        let Ok(mut outgoing_node) = outgoing_q.single_mut() else {
            return;
        };
        if compact {
            outgoing_node.max_height = Val::Px(140.0); // limit height, but show
            outgoing_node.display = Display::Flex;
            outgoing_node.width = Val::Percent(100.0);
            outgoing_node.max_width = Val::Percent(100.0);
            outgoing_node.min_width = Val::Auto;
            outgoing_node.padding = UiRect::axes(Val::Px(4.0), Val::Px(4.0));
        } else {
            outgoing_node.max_height = Val::Px(220.0);
            outgoing_node.display = Display::Flex;
            outgoing_node.width = Val::Percent(96.0);
            outgoing_node.max_width = Val::Px(800.0 * crate::clamp_ui_scale(settings.config.ui.combat_hud_scale));
        }
    }

    {
        let mut home_q = nodes.p3();
        if let Ok(mut home_node) = home_q.single_mut() {
            if compact {
                // Match the smaller compact cell size
                let cell = 38.0;
                home_node.width = Val::Px(cell);
                home_node.height = Val::Px(cell);
                home_node.border_radius = BorderRadius::all(Val::Px(4.0));
            } else {
                let size = home_button_size_for_breakpoint(ui_shell.breakpoint);
                home_node.width = Val::Px(home_button_width_for_breakpoint(ui_shell.breakpoint));
                home_node.height = Val::Px(size);
                home_node.border_radius = BorderRadius::all(Val::Px(12.0));
            }
        }
    }
    {
        let mut exit_q = nodes.p4();
        if let Ok(mut exit_node) = exit_q.single_mut() {
            let size = home_button_size_for_breakpoint(ui_shell.breakpoint);
            exit_node.width = Val::Px(home_button_width_for_breakpoint(ui_shell.breakpoint));
            exit_node.height = Val::Px(size);
            exit_node.border_radius = BorderRadius::all(Val::Px(if compact { 10.0 } else { 12.0 }));
        }
    }
    
    {
        let mut build_q = nodes.p5();
        if let Ok((mut bg, mut border, mut node)) = build_q.single_mut() {
            if compact {
                // Transparent — unified background on BottomHudRoot
                bg.0 = Color::NONE;
                *border = BorderColor::all(Color::NONE);
                node.flex_wrap = FlexWrap::NoWrap;
                node.max_width = Val::Percent(100.0);
                node.width = Val::Percent(100.0);
                node.border_radius = BorderRadius::ZERO;
                node.padding = UiRect::axes(Val::Px(4.0), Val::Px(6.0));
                node.column_gap = Val::Px(2.0);
                node.justify_content = JustifyContent::SpaceEvenly;
            } else {
                bg.0 = Color::srgba(0.04, 0.05, 0.08, 0.92);
                *border = BorderColor::all(panel_border());
                node.flex_wrap = FlexWrap::Wrap;
                node.border_radius = BorderRadius::all(Val::Px(6.0 * settings.config.ui.combat_hud_scale));
                let s = crate::clamp_ui_scale(settings.config.ui.combat_hud_scale);
                node.padding = UiRect::all(Val::Px(8.0 * s));
                node.column_gap = Val::Px(4.0 * s);
                node.justify_content = JustifyContent::Center;
                let (cell, gap, pad) = crate::ui::shell::action_dock_grid_layout_sizes(s);
                node.max_width = Val::Px(3.0_f32.mul_add(cell, 2.0 * gap) + pad * 2.0 + 4.0); // 3 cols
            }
        }
    }
    
    {
        let mut panel_q = nodes.p6();
        if let Ok((mut bg, mut border, mut node)) = panel_q.single_mut() {
            if compact {
                // Transparent — the shared BottomHudRoot bg covers everything
                bg.0 = Color::NONE;
                *border = BorderColor::all(Color::NONE);
                node.border_radius = BorderRadius::ZERO;
                node.max_width = Val::Percent(100.0);
                node.width = Val::Percent(100.0);
                node.padding = UiRect::axes(Val::Px(4.0), Val::Px(6.0));
            } else {
                bg.0 = Color::srgba(0.1, 0.1, 0.15, 0.85);
                node.border_radius = BorderRadius::all(Val::Px(12.0 * settings.config.ui.combat_hud_scale));
                node.max_width = Val::Px(800.0 * settings.config.ui.combat_hud_scale);
                node.width = Val::Percent(96.0);
                node.padding = UiRect::all(Val::Px(12.0 * settings.config.ui.combat_hud_scale));
            }
        }
    }

    {
        let mut build_btns = nodes.p7();
        let s = crate::clamp_ui_scale(settings.config.ui.combat_hud_scale);
        let normal_cell = crate::ui::shell::action_dock_grid_layout_sizes(s).0;
        let compact_cell = 38.0; // smaller buttons for mobile to fit 7 elements
        
        for mut btn in build_btns.iter_mut() {
            if compact {
                btn.width = Val::Px(compact_cell);
                btn.height = Val::Px(compact_cell);
                btn.justify_content = JustifyContent::Center;
                btn.align_items = AlignItems::Center;
                btn.padding = UiRect::ZERO;
                btn.row_gap = Val::Px(0.0);
            } else {
                btn.width = Val::Px(normal_cell);
                btn.height = Val::Px(normal_cell);
                btn.justify_content = JustifyContent::FlexStart;
                btn.align_items = AlignItems::Stretch;
            }
        }
    }
    
}

pub fn system_apply_responsive_hud_icons_and_hotkeys(
    ui_shell: Res<UiShellState>,
    settings: Res<crate::ClientSettings>,
    mut q_icons: Query<(&mut Node, &mut Visibility), With<crate::ui::setup::BuildPillIcon>>,
    mut q_hotkeys: Query<&mut Visibility, (With<crate::ui::setup::BuildPillHotkey>, Without<crate::ui::setup::BuildPillIcon>)>,
) {
    let compact = matches!(ui_shell.breakpoint, UiBreakpoint::Compact);
    let icon_size = if compact { 22.0 } else {
        let s = crate::clamp_ui_scale(settings.config.ui.combat_hud_scale);
        crate::ui::shell::action_dock_grid_layout_sizes(s).0 * 0.55
    };
    for (mut node, mut vis) in q_icons.iter_mut() {
        node.width = Val::Px(icon_size);
        node.height = Val::Px(icon_size);
        *vis = Visibility::Visible;
    }
    for mut vis in q_hotkeys.iter_mut() {
        *vis = if compact { Visibility::Hidden } else { Visibility::Visible };
    }
}

pub fn system_sync_bottom_combat_hud(
    game_opt: Option<Res<GameState>>,
    my_id: Res<MyPlayerId>,
    attack_ratio: Res<AttackRatio>,
    settings: Res<ClientSettings>,
    mut q_readout: Query<
        (&mut Text, &mut TextFont, &mut Node),
        (
            With<AttackRatioReadout>,
            Without<BottomHudTroops>,
            Without<BottomHudGold>,
            Without<AttackRatioTrack>,
            Without<AttackRatioBarShell>,
            Without<AttackRatioFill>,
        ),
    >,
    mut q_gold: Query<
        &mut Text,
        (
            With<BottomHudGold>,
            Without<AttackRatioReadout>,
            Without<BottomHudTroops>,
        ),
    >,
    mut q_troops: Query<
        (&mut Text, &mut TextColor),
        (
            With<BottomHudTroops>,
            Without<AttackRatioReadout>,
            Without<BottomHudGold>,
        ),
    >,
    mut q_track: Query<
        &mut Node,
        (
            With<AttackRatioTrack>,
            Without<AttackRatioReadout>,
            Without<AttackRatioBarShell>,
            Without<AttackRatioFill>,
        ),
    >,
    mut q_bar_shell: Query<
        &mut Node,
        (
            With<AttackRatioBarShell>,
            Without<AttackRatioReadout>,
            Without<AttackRatioTrack>,
            Without<AttackRatioFill>,
        ),
    >,
    mut q_fill: Query<
        &mut Node,
        (
            With<AttackRatioFill>,
            Without<AttackRatioReadout>,
            Without<AttackRatioTrack>,
            Without<AttackRatioBarShell>,
            Without<AttackRatioHandle>,
        ),
    >,
    mut q_handle: Query<
        &mut Node,
        (
            With<AttackRatioHandle>,
            Without<AttackRatioReadout>,
            Without<AttackRatioTrack>,
            Without<AttackRatioBarShell>,
            Without<AttackRatioFill>,
        ),
    >,
) {
    let Some(game) = game_opt.as_deref() else { return };

    let Ok((mut readout, mut readout_font, mut readout_node)) = q_readout.single_mut() else { return };
    
    if my_id.0 == 0 {
        **readout = String::new();
        if let Ok(mut gl) = q_gold.single_mut() { **gl = "0".to_string(); }
        if let Ok((mut t, _)) = q_troops.single_mut() { **t = "0 / 0".to_string(); }
        return;
    }

    let s = clamp_ui_scale(settings.config.ui.combat_hud_scale);
    let compact = settings.config.ui.combat_hud_scale < 1.0;
    readout_font.font_size = if compact { 13.0 * s } else { 18.0 * s };
    readout_node.width = Val::Px(if compact { 90.0 * s } else { 110.0 * s });
    if let Ok(mut n) = q_track.single_mut() {
        n.height = Val::Px(32.0 * s);
    }
    if let Ok(mut n) = q_bar_shell.single_mut() {
        n.height = Val::Px(16.0 * s);
        n.border_radius = BorderRadius::all(Val::Px(8.0 * s));
    }

    let r = attack_ratio.0.clamp(0.01, 1.0);
    if let Ok(mut fill) = q_fill.single_mut() {
        fill.width = Val::Percent(r * 100.0);
    }
    if let Ok(mut handle) = q_handle.single_mut() {
        handle.left = Val::Percent(r * 100.0);
    }

    // Economy Sync
    if let Some(player) = game.player(my_id.0) {
        let fmt = |v: f64| -> String {
            if v >= 1_000_000.0 {
                format!("{:.1}M", v / 1_000_000.0)
            } else if v >= 1_000.0 {
                format!("{:.1}K", v / 1_000.0)
            } else {
                format!("{:.0}", v)
            }
        };

        if let Ok(mut gl) = q_gold.single_mut() {
            let s = fmt(player.gold);
            if **gl != s { **gl = s; }
        }
        if let Ok((mut t, mut t_color)) = q_troops.single_mut() {
            let s = format!("{} / {}", fmt(player.troops), fmt(player.max_troops));
            if **t != s { **t = s; }

            let ratio = player.troops / player.max_troops.max(1.0);
            let color = if ratio < 0.45 {
                Color::srgb(0.4, 0.9, 0.4) // Green
            } else if ratio < 0.75 {
                Color::srgb(0.9, 0.7, 0.1) // Yellow/Orange
            } else {
                Color::srgb(0.9, 0.3, 0.3) // Red
            };
            if t_color.0 != color {
                t_color.0 = color;
            }
        }
    }

    let preview_troops = game
        .player(my_id.0)
        .map(|p| (p.troops * r as f64).floor().max(0.0))
        .unwrap_or(0.0);
    let readout_str = format!("{}% ({:.0})", (r * 100.0).round(), preview_troops);
    if **readout != readout_str {
        **readout = readout_str;
    }

}

pub fn system_attack_ratio_keyboard(
    kb: Res<ButtonInput<KeyCode>>,
    my_id: Res<MyPlayerId>,
    mut ratio: ResMut<AttackRatio>,
) {
    if my_id.0 == 0 {
        return;
    }
    if kb.just_pressed(KeyCode::KeyT) {
        ratio.0 = (ratio.0 - ATTACK_RATIO_KEYSTEP).max(0.01);
    }
    if kb.just_pressed(KeyCode::KeyY) {
        ratio.0 = (ratio.0 + ATTACK_RATIO_KEYSTEP).min(1.0);
    }
}

pub fn system_attack_ratio_slider_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut ratio: ResMut<AttackRatio>,
    my_id: Res<MyPlayerId>,
    q_track: Query<&RelativeCursorPosition, With<AttackRatioTrack>>,
) {
    if my_id.0 == 0 {
        return;
    }
    let dragging =
        mouse.pressed(MouseButton::Left) || touches.iter().next().is_some();
    let released = mouse.just_released(MouseButton::Left) || touches.any_just_released();
    if !dragging && !released {
        return;
    }

    let apply = |out: &mut f32| {
        for rcp in &q_track {
            if !rcp.cursor_over {
                continue;
            }
            let Some(n) = rcp.normalized else { continue };
            let t = (n.x + 0.5).clamp(0.0, 1.0);
            *out = (0.01_f32 + t * 0.99_f32).clamp(0.01_f32, 1.0_f32);
            break;
        }
    };

    if dragging {
        apply(&mut ratio.0);
    } else if released {
        // Release frame: `pressed` is false — still snap ratio if the cursor is on the rail.
        apply(&mut ratio.0);
    }
}

pub fn system_exit_button(
    mut interaction_q: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<ExitButton>),
    >,
    mut next_state: ResMut<NextState<AppState>>,
    mut senders: Query<&mut MessageSender<ClientLeaveMessage>>,
    mut joined_lobby: Option<ResMut<JoinedLobby>>,
) {
    for (interaction, mut color) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *color = Color::srgb(0.28, 0.1, 0.08).into();
                if let Some(jl) = joined_lobby.as_mut() {
                    jl.0 = None;
                }
                for mut sender in senders.iter_mut() {
                    let _ = sender.send::<ReliableChannel>(ClientLeaveMessage);
                }
                next_state.set(AppState::LobbyBrowser);
            }
            Interaction::Hovered => {
                *color = Color::srgb(0.72, 0.28, 0.24).into();
            }
            Interaction::None => {
                *color = accent_danger().into();
            }
        }
    }
}

pub fn system_home_button(
    mut interaction_q: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<HomeButton>),
    >,
    mut cam_q: Query<&mut Transform, With<Camera2d>>,
    labels_q: Query<(&Transform, &PlayerLabel), Without<Camera2d>>,
    my_id: Res<MyPlayerId>,
) {
    for (interaction, mut color) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *color = accent_home_pressed().into();
                // Home the camera
                let mut target_pos = None;
                for (lbl_trans, lbl) in labels_q.iter() {
                    if lbl.0 == my_id.0 {
                        target_pos = Some(lbl_trans.translation.xy());
                        break;
                    }
                }
                if let Some(pos) = target_pos {
                    for mut cam_t in cam_q.iter_mut() {
                        cam_t.translation.x = pos.x;
                        cam_t.translation.y = pos.y;
                    }
                }
            }
            Interaction::Hovered => {
                *color = accent_home_hover().into();
            }
            Interaction::None => {
                *color = accent_home().into();
            }
        }
    }
}

pub fn system_handle_cancel_click(
    interaction_q: Query<
        (&Interaction, &CancelAttackButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
) {
    for (interaction, btn) in &interaction_q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let attack_id = btn.0;
        for mut sender in senders.iter_mut() {
            let _ = sender.send::<ReliableChannel>(ClientGameplayMessage {
                intent: GameplayIntent::CancelAttack { attack_id },
            });
        }
    }
}

pub fn system_handle_recall_fleet_click(
    interaction_q: Query<
        (&Interaction, &CancelFleetButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
) {
    for (interaction, btn) in &interaction_q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let fleet_id = btn.0;
        for mut sender in senders.iter_mut() {
            let _ = sender.send::<ReliableChannel>(ClientGameplayMessage {
                intent: GameplayIntent::RecallFleet { fleet_id },
            });
        }
    }
}

/// Toggle dev stats pill visibility when the Settings button is pressed.
pub fn system_settings_toggle_devstats(
    mut interaction_q: Query<
        &Interaction,
        (Changed<Interaction>, With<SettingsButton>),
    >,
    mut q_pill: Query<&mut Node, With<DevStatsPill>>,
) {
    for interaction in interaction_q.iter_mut() {
        if *interaction == Interaction::Pressed {
            for mut node in q_pill.iter_mut() {
                node.display = match node.display {
                    Display::None => Display::Flex,
                    _ => Display::None,
                };
            }
        }
    }
}

pub fn system_sync_bottom_troops_bar(
    game_opt: Option<Res<GameState>>,
    my_id: Res<MyPlayerId>,
    time: Res<Time>,
    mut display_troops: Local<f32>,
    mut q_shell: Query<&mut BorderColor, With<TroopsBarShell>>,
    mut q_fill: Query<(&mut Node, &mut BackgroundColor), With<TroopsBarFill>>,
    mut q_damage: Query<&mut Node, (With<TroopsBarDamage>, Without<TroopsBarFill>)>,
) {
    let Some(game) = game_opt.as_deref() else { return };
    if my_id.0 == 0 { return; }

    let Some(player) = game.player(my_id.0) else { return };
    let max_troops = player.max_troops.max(1.0);
    let target_percent = (player.troops / max_troops).clamp(0.0, 1.0) as f32;

    let dt = time.delta_secs();
    let lerp_speed = 3.33;
    
    if (target_percent - *display_troops).abs() < 0.001 {
        *display_troops = target_percent;
    } else {
        *display_troops += (target_percent - *display_troops) * (lerp_speed * dt).min(1.0);
    }

    let color = if target_percent < 0.45 {
        Color::srgb(0.4, 0.9, 0.4) // Green
    } else if target_percent < 0.75 {
        Color::srgb(0.9, 0.7, 0.1) // Yellow/Orange
    } else {
        Color::srgb(0.9, 0.3, 0.3) // Red
    };

    if let Ok(mut bc) = q_shell.single_mut() {
        if bc.top != color { *bc = BorderColor::all(color); }
    }

    if let Ok((mut fill_node, mut bg)) = q_fill.single_mut() {
        // Instant snap for the actual fill
        fill_node.width = Val::Percent(target_percent * 100.0);
        if bg.0 != color { bg.0 = color; }
    }

    if let Ok(mut dmg_node) = q_damage.single_mut() {
        // Slow lerp for the damage background
        dmg_node.width = Val::Percent(*display_troops * 100.0);
    }
}