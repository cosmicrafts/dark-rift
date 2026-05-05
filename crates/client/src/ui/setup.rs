use crate::Cli;
use crate::ui::write_guard::assign_if_ne;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use instant::Instant;
use crate::ClientSettings;
use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use dark_rift_shared::game::*;
use crate::gameplay::buildings::building_kind_icon_path;
use crate::state::*;
use crate::ui::*;

#[derive(Component)]
pub struct BuildPillIcon;

#[derive(Component)]
pub struct BuildPillHotkey;

pub fn spawn_action_dock(
    commands: &mut Commands,
    shell: &HudShellRegions,
    observer: bool,
    hud_scale: f32,
    asset_server: &AssetServer,
    font: Handle<Font>,
) {
    let s = clamp_ui_scale(hud_scale);
    let vis = if observer {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    let (cell, gap, pad) = action_dock_grid_layout_sizes(s);
    let frame = 2.0 * s;
    let outer_w = action_dock_grid_outer_width_px(s);
    let icon = ((cell - 12.0 * s).max(20.0)).min(cell - 2.0);

    commands
        .spawn((
            ChildOf(shell.action_dock),
            ActionDockPointerSink,
            InGameEntity,
            RelativeCursorPosition::default(),
            vis,
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::FlexEnd,
                column_gap: Val::Px(SPACE_2),
                row_gap: Val::Px(SPACE_2),
                ..default()
            },
            TextFont {
                font: font.clone(),
                font_size: 16.0,
                ..default()
            },
        ))
        .with_children(|dock| {
            dock.spawn((
                BuildPillRow,
                Node {
                    max_width: Val::Px(outer_w),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(gap),
                    column_gap: Val::Px(gap),
                    padding: UiRect::all(Val::Px(pad)),
                    border: UiRect::all(Val::Px(frame)),
                    border_radius: BorderRadius::all(Val::Px(6.0 * s)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.04, 0.05, 0.08, 0.92)),
                BorderColor::all(panel_border()),
            ))
            .with_children(|grid| {
                for idx in 0..6 {
                    let kind = BuildingKind::ALL[idx];
                    let hk = (idx + 1).to_string();
                    grid.spawn((
                        Button,
                        BuildPillButton(kind),
                        Node {
                            width: Val::Px(cell),
                            height: Val::Px(cell),
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::all(Val::Px(3.0 * s)),
                            justify_content: JustifyContent::FlexStart,
                            row_gap: Val::Px(1.0 * s),
                            border_radius: BorderRadius::all(Val::Px(4.0 * s)),
                            border: UiRect::all(Val::Px(1.0 * s)),
                            ..default()
                        },
                        BackgroundColor(panel_bg()),
                        BorderColor::all(panel_border()),
                    ))
                    .with_children(|cell_ui| {
                        cell_ui.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_direction: FlexDirection::Row,
                                justify_content: JustifyContent::FlexEnd,
                                min_height: Val::Px(11.0 * s),
                                ..default()
                            },
                        ))
                        .with_children(|hk_row| {
                            hk_row.spawn((
                                BuildPillHotkey,
                                Text::new(hk),
                                BuildPillLabel { slot: 3, kind },
                                TextFont {
                                    font: font.clone(),
                                    font_size: 10.0 * s,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.55, 0.62, 0.72)),
                            ));
                        });
                        cell_ui.spawn((
                            BuildPillIcon,
                            ImageNode::new(asset_server.load(building_kind_icon_path(kind))),
                            Node {
                                width: Val::Px(icon),
                                height: Val::Px(icon),
                                align_self: AlignSelf::Center,
                                ..default()
                            },
                        ));
                        cell_ui.spawn((
                            Text::new("0"),
                            BuildPillLabel { slot: 1, kind },
                            TextFont {
                                font: font.clone(),
                                font_size: 10.0 * s,
                                ..default()
                            },
                            TextColor(Color::srgb(0.85, 0.9, 1.0)),
                        ));
                        cell_ui.spawn((
                            Text::new("0"),
                            BuildPillLabel { slot: 2, kind },
                            TextFont {
                                font: font.clone(),
                                font_size: 9.5 * s,
                                ..default()
                            },
                            TextColor(Color::srgb(0.75, 0.78, 0.82)),
                        ));
                    });
                }
            });

            dock.spawn((
                Node {
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                Button,
                HomeButton,
                BackgroundColor(accent_home()),
                BorderColor::all(panel_border()),
            ))
            .with_children(|b| {
                b.spawn((
                    ImageNode::new(asset_server.load("icons/home.png")),
                    Node {
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        ..default()
                    },
                ));
            });
        });
}

pub fn spawn_bottom_combat_hud(
    commands: &mut Commands,
    shell: &HudShellRegions,
    observer: bool,
    hud_scale: f32,
    asset_server: &AssetServer,
    font: Handle<Font>,
) {
    let s = clamp_ui_scale(hud_scale);
    let vis = if observer {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    spawn_action_dock(commands, shell, observer, hud_scale, asset_server, font.clone());
    commands
        .spawn((
            ChildOf(shell.bottom_sheet),
            EventFeedContainer,
            InGameEntity,
            vis,
            Node {
                position_type: PositionType::Relative,
                width: Val::Px(300.0 * s),
                height: Val::Px(200.0 * s),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                margin: UiRect::bottom(Val::Px(4.0 * s)),
                row_gap: Val::Px(4.0 * s),
                ..default()
            },
            TextFont {
                font: font.clone(),
                font_size: 16.0,
                ..default()
            },
        ));

    commands
        .spawn((
            ChildOf(shell.bottom_sheet),
            BottomHudRoot,
            InGameEntity,
            RelativeCursorPosition::default(),
            vis,
            Node {
                position_type: PositionType::Relative,
                width: Val::Auto,
                max_width: Val::Percent(100.0),
                align_items: AlignItems::FlexStart,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(12.0 * s),
                padding: UiRect::bottom(Val::Px(16.0 * s)),
                ..default()
            },
            BackgroundColor(Color::NONE), // dynamically set to black-transparent on compact
            TextFont {
                font: font.clone(),
                font_size: 16.0,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                crate::ui::hud_combat::BottomHudPanel,
                Node {
                    flex_shrink: 0.0,
                    width: Val::Auto,
                    min_width: Val::Px(300.0 * s),
                    max_width: Val::Px(560.0 * s),
                    align_self: AlignSelf::FlexStart,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    row_gap: Val::Px(8.0 * s),
                    padding: UiRect::all(Val::Px(12.0 * s)),
                    border_radius: BorderRadius::all(Val::Px(12.0 * s)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.15, 0.85)),
                BorderColor::all(Color::NONE), // placeholder for uniform tuple size
            ))
            .with_children(|panel| {
                // Top row: Simplified Economy + Readout
                panel.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                )).with_children(|eco_row| {
                    // Left: Troops / Cap Loading Bar
                    eco_row.spawn((
                        crate::ui::hud_combat::TroopsBarShell,
                        Node {
                            width: Val::Px(180.0 * s),
                            height: Val::Px(28.0 * s),
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            overflow: Overflow::clip(),
                            border_radius: BorderRadius::all(Val::Px(6.0 * s)),
                            border: UiRect::all(Val::Px(1.5 * s)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.1, 0.1, 0.15, 0.9)),
                        BorderColor::all(Color::BLACK),
                    )).with_children(|bar| {
                        // Damage/Heal animation layer (lerps slowly)
                        bar.spawn((
                            crate::ui::hud_combat::TroopsBarDamage,
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(0.0),
                                width: Val::Percent(0.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(Color::WHITE),
                            ZIndex(0),
                        ));

                        // Actual fill layer (visual interpolation follows current troops state)
                        bar.spawn((
                            crate::ui::hud_combat::TroopsBarFill,
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(0.0),
                                width: Val::Percent(0.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.4, 0.9, 0.4)), // will be dynamically colored
                            ZIndex(1),
                        ));

                        // Text and Icon container (above fills)
                        bar.spawn((
                            Node {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(6.0 * s),
                                ..default()
                            },
                            ZIndex(2),
                        )).with_children(|group| {
                            group.spawn((
                                ImageNode::new(asset_server.load("icons/troops.png")),
                                Node { width: Val::Px(18.0 * s), height: Val::Px(18.0 * s), ..default() }
                            ));
                            group.spawn((
                                BottomHudTroops,
                                Text::new("0 / 0"),
                                TextFont { font_size: 16.0 * s, ..default() },
                                TextColor(Color::WHITE),
                            ));
                        });
                    });

                    // Center: Gold
                    eco_row.spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(6.0 * s),
                            ..default()
                        },
                    )).with_children(|group| {
                        group.spawn((
                            ImageNode::new(asset_server.load("icons/gold.png")),
                            Node { width: Val::Px(20.0 * s), height: Val::Px(20.0 * s), ..default() }
                        ));
                        group.spawn((
                            BottomHudGold,
                            Text::new("0"),
                            TextFont { font_size: 16.0 * s, ..default() },
                            TextColor(Color::WHITE),
                        ));
                    });

                    // Right: Slider readout
                    eco_row.spawn((
                        AttackRatioReadout,
                        Text::new(""),
                        TextFont {
                            font_size: 16.0 * s,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });

                // Bottom row: Attack Slider (Full Width)
                panel.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                )).with_children(|slider_row| {

                    slider_row.spawn((
                        Button,
                        AttackRatioTrack,
                        RelativeCursorPosition::default(),
                        Node {
                            flex_grow: 1.0,
                            height: Val::Px(32.0 * s),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Stretch,
                            ..default()
                        },
                    ))
                    .with_children(|track| {
                        track
                            .spawn((
                                AttackRatioBarShell,
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(16.0 * s),
                                    flex_direction: FlexDirection::Row,
                                    overflow: Overflow::clip(),
                                    border_radius: BorderRadius::all(Val::Px(8.0 * s)),
                                    ..default()
                                },
                            ))
                            .with_children(|bar| {
                                bar.spawn((
                                    AttackRatioFill,
                                    Node {
                                        width: Val::Percent(20.0),
                                        height: Val::Percent(100.0),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.2, 0.8, 0.2, 1.0)),
                                ));
                                bar.spawn((
                                    Node {
                                        flex_grow: 1.0,
                                        height: Val::Percent(100.0),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.2, 0.2, 0.25)),
                                ));
                            });
                            
                        track.spawn((
                            crate::ui::hud_combat::AttackRatioHandle,
                            Node {
                                position_type: PositionType::Absolute,
                                width: Val::Px(24.0 * s),
                                height: Val::Px(24.0 * s),
                                border_radius: BorderRadius::all(Val::Px(12.0 * s)),
                                left: Val::Percent(20.0),
                                margin: UiRect::left(Val::Px(-12.0 * s)), // center the handle
                                ..default()
                            },
                            BackgroundColor(Color::WHITE),
                            BorderColor::all(Color::BLACK),
                        ));
                    });
                });
            });

            root.spawn((
                OutgoingRowsContainer,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Percent(100.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    margin: UiRect::bottom(Val::Px(8.0 * s)),
                    // DOM order is oldest→newest; reverse column so the latest outgoing row is on top.
                    flex_direction: FlexDirection::ColumnReverse,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0 * s),
                    max_height: Val::Px(220.0 * s),
                    overflow: Overflow::scroll_y(),
                    display: Display::None,
                    ..default()
                },
                bevy::ui::ScrollPosition::default(),
            ));
        });
}

pub fn update_hud(
    overlay: Res<OverlayDiagnostics>,
    build_mode: Res<BuildStructureMode>,
    highest: Res<HighestReceivedTurn>,
    mut fps_q: Query<&mut Text, (With<FpsText>, Without<GameTimerText>)>,
    mut timer_q: Query<&mut Text, With<GameTimerText>>,
) {
    if let Some(mut fps_text) = fps_q.iter_mut().next() {
        let show_build_line =
            build_mode.0.is_some() && !overlay.build_hint.is_empty();
        let tick = highest.0.unwrap_or(0);
        let mut fps_str = if show_build_line {
            format!(
                "{}  |  Ping: {}  |  {} FPS  |  Tick: {}  |  {}",
                overlay.status_line,
                overlay.ping_value,
                overlay.fps_value,
                tick,
                overlay.build_hint
            )
        } else {
            format!(
                "{}  |  Ping: {}  |  {} FPS  |  Tick: {}",
                overlay.status_line, overlay.ping_value, overlay.fps_value, tick
            )
        };
        if !overlay.sim_perf_line.is_empty() {
            fps_str.push_str(" | ");
            fps_str.push_str(&overlay.sim_perf_line);
        }
        if **fps_text != fps_str {
            **fps_text = fps_str;
        }
    }

    if let Some(mut timer_text) = timer_q.iter_mut().next() {
        let seconds = (highest.0.unwrap_or(0) as f64 * 0.040).floor() as u64;
        let mins = seconds / 60;
        let secs = seconds % 60;
        let time_str = format!("{:02}:{:02}", mins, secs);
        if **timer_text != time_str {
            **timer_text = time_str;
        }
    }
}

pub fn hud_trunc_name(s: &str, max_ch: usize) -> String {
    let count = s.chars().count();
    if count <= max_ch {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_ch).collect::<String>())
    }
}

pub fn player_label_scale(tiles_owned: u32, ref_tiles: f32, max_scale: f32) -> f32 {
    let t = tiles_owned.max(1) as f32;
    let r = ref_tiles.max(1.0);
    (t / r).sqrt().max(1.0).min(max_scale.max(1.0))
}

pub fn init_attack_ratio_from_ui_settings(ui: Res<ClientSettings>, mut ratio: ResMut<AttackRatio>) {
    ratio.0 = ui.config.ui.attack_ratio.clamp(0.01, 1.0);
}

pub fn system_update_perf_overlay(
    diagnostics: Res<DiagnosticsStore>,
    net: Res<NetTiming>,
    cli: Res<Cli>,
    mut overlay: ResMut<OverlayDiagnostics>,
    mut throttle: Local<PerfOverlayUiState>,
) {
    let now = Instant::now();

    let refresh_fps = throttle
        .last_fps_refresh
        .map(|t| now.duration_since(t) >= PERF_OVERLAY_FPS_INTERVAL)
        .unwrap_or(true);
    let refresh_net = throttle
        .last_net_refresh
        .map(|t| now.duration_since(t) >= PERF_OVERLAY_NET_INTERVAL)
        .unwrap_or(true);

    if refresh_fps {
        throttle.last_fps_refresh = Some(now);
        let next = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|v| v.value())
            .map(|f| format!("{f:.1}"))
            .unwrap_or_else(|| "—".to_string());
        assign_if_ne(&mut overlay.fps_value, next);
    }

    if refresh_net {
        throttle.last_net_refresh = Some(now);
        let next = if cli.replay.is_some() {
            "Offline".to_string()
        } else if net.handshake_ms.is_none() && net.connect_started.is_some() {
            "Connecting…".to_string()
        } else if net.handshake_ms.is_some() {
            let ping = net.turn_spacing_ema_ms.max(1.0);
            format!("{ping:.0} ms")
        } else {
            "…".to_string()
        };
        assign_if_ne(&mut overlay.ping_value, next);
    }

    if !(refresh_fps || refresh_net) {
        return;
    }
}

