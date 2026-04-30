use bevy::prelude::*;

use crate::ui::layers::HUD_SHELL;
use crate::ui::theme::{BUTTON_RADIUS, PANEL_RADIUS, SPACE_2, SPACE_3, SPACE_4};
use crate::ui::write_guard::assign_if_ne;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiBreakpoint {
    Compact,
    Medium,
    Wide,
}

/// HUD scale used for the bottom-right build grid + HOME dock (matches `combat_hud_scale` at game start).
#[derive(Resource, Clone, Copy, Debug)]
pub struct CombatDockUiScale(pub f32);

impl Default for CombatDockUiScale {
    fn default() -> Self {
        Self(1.0)
    }
}

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct UiShellState {
    pub breakpoint: UiBreakpoint,
    pub inset_top: f32,
    pub inset_right: f32,
    pub inset_bottom: f32,
    pub inset_left: f32,
}

/// Stable breakpoint across frames: desktop window metrics jitter sub‑pixels; WASM canvas is steadier.
/// Without hysteresis, `aspect < 0.80` / `width < 600` flip flop → whole HUD relayout every frame.
fn breakpoint_from_window_stabilized(width: f32, height: f32, prev: UiBreakpoint) -> UiBreakpoint {
    let h = height.max(1.0);
    let aspect = width / h;
    // Portrait, near-square (tablet portrait / iPad-ish), or very narrow: use compact bottom HUD.
    // Clear landscape: noticeably wider than tall — desktop two-zone bottom layout.
    let compact_shape = aspect <= 1.06 || width < 560.0;
    let clear_landscape = width >= 700.0 && aspect >= 1.16;

    match prev {
        UiBreakpoint::Compact => {
            if clear_landscape {
                if width < 1100.0 {
                    UiBreakpoint::Medium
                } else {
                    UiBreakpoint::Wide
                }
            } else {
                UiBreakpoint::Compact
            }
        }
        UiBreakpoint::Medium => {
            if compact_shape && (width < 600.0 || aspect <= 1.04) {
                UiBreakpoint::Compact
            } else if width >= 1120.0 && aspect >= 1.08 {
                UiBreakpoint::Wide
            } else {
                UiBreakpoint::Medium
            }
        }
        UiBreakpoint::Wide => {
            if compact_shape && (width < 620.0 || aspect <= 1.04) {
                UiBreakpoint::Compact
            } else if width < 1080.0 {
                UiBreakpoint::Medium
            } else {
                UiBreakpoint::Wide
            }
        }
    }
}

impl Default for UiShellState {
    fn default() -> Self {
        Self {
            breakpoint: UiBreakpoint::Wide,
            inset_top: SPACE_3,
            inset_right: SPACE_3,
            inset_bottom: SPACE_3,
            inset_left: SPACE_3,
        }
    }
}

#[derive(Resource)]
pub struct HudShellRegions {
    pub top_bar: Entity,
    pub bottom_sheet: Entity,
    pub action_dock: Entity,
}

#[derive(Component)]
pub struct HudShellRoot;

#[derive(Component)]
pub struct HudTopBarRegion;

#[derive(Component)]
pub struct HudBottomSheetRegion;

#[derive(Component)]
pub struct HudActionDockRegion;

/// Spawns [`HudShellRoot`] at [`crate::ui_layers::HUD_SHELL`]. Child order under each region is
/// paint order for that region only; cross-menu stacking uses `GlobalZIndex` in `ui_layers.rs`.
pub fn setup_hud_shell(mut commands: Commands) {
    let root = commands
        .spawn((
            HudShellRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                ..default()
            },
            GlobalZIndex(HUD_SHELL),
        ))
        .id();

    let top_bar = commands
        .spawn((
            ChildOf(root),
            HudTopBarRegion,
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
        ))
        .id();
    let bottom_sheet = commands
        .spawn((
            ChildOf(root),
            HudBottomSheetRegion,
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            ZIndex(50),
        ))
        .id();
    let action_dock = commands
        .spawn((
            ChildOf(root),
            HudActionDockRegion,
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            ZIndex(100),
        ))
        .id();

    commands.insert_resource(HudShellRegions {
        top_bar,
        bottom_sheet,
        action_dock,
    });
}

pub fn system_update_ui_shell_state(
    mut state: ResMut<UiShellState>,
    mut dock_scale: ResMut<CombatDockUiScale>,
    settings: Option<Res<crate::ClientSettings>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    let Ok(window) = windows.single() else { return };
    // Whole-pixel quantization kills winit sub-pixel oscillation on resize/DPI that triggers breakpoint churn.
    let width = window.width().round();
    let height = window.height().round().max(1.0);

    let breakpoint = breakpoint_from_window_stabilized(width, height, state.breakpoint);

    let inset = match breakpoint {
        UiBreakpoint::Compact => SPACE_2,
        UiBreakpoint::Medium => SPACE_3,
        UiBreakpoint::Wide => SPACE_4,
    };

    let new_state = UiShellState {
        breakpoint,
        inset_top: inset,
        inset_right: inset,
        inset_bottom: inset,
        inset_left: inset,
    };

    // Critical: assigning `ResMut` every frame marks the resource **changed** even when values match,
    // which forces layout systems to dirty the entire HUD every tick on native.
    if *state != new_state {
        *state = new_state;
    }

    if let Some(s) = settings {
        let new_scale = crate::clamp_ui_scale(match breakpoint {
            UiBreakpoint::Compact => s.config.ui.mobile_hud_scale,
            _ => s.config.ui.combat_hud_scale,
        });
        if (dock_scale.0 - new_scale).abs() > 1e-4 {
            dock_scale.0 = new_scale;
        }
    }
}

pub fn system_apply_ui_shell_layout(
    state: Res<UiShellState>,
    dock_scale: Res<CombatDockUiScale>,
    mut regions: ParamSet<(
        Query<&mut Node, With<HudTopBarRegion>>,
        Query<&mut Node, With<HudBottomSheetRegion>>,
        Query<&mut Node, With<HudActionDockRegion>>,
    )>,
) {
    {
        let mut top_q = regions.p0();
        let Ok(mut top) = top_q.single_mut() else { return };
        // Full-width top strip: diagnostics left, EXIT right (children use SpaceBetween).
        top.left = Val::Px(state.inset_left);
        top.right = Val::Px(state.inset_right);
        top.top = Val::Px(state.inset_top);
        top.flex_direction = FlexDirection::Row;
        top.justify_content = JustifyContent::SpaceBetween;
        top.align_items = AlignItems::FlexStart;
        top.column_gap = Val::Px(SPACE_2);
    }

    {
        let mut bottom_q = regions.p1();
        let Ok(mut bottom) = bottom_q.single_mut() else { return };
        let compact = matches!(state.breakpoint, UiBreakpoint::Compact);

        assign_if_ne(&mut bottom.left, Val::Px(state.inset_left));
        if compact {
            // Full width: combat HUD stacks above the bottom action dock.
            assign_if_ne(&mut bottom.right, Val::Px(state.inset_right));
            let dock_reserve_h = action_dock_grid_layout_sizes(dock_scale.0).0 + 8.0 * dock_scale.0;
            assign_if_ne(
                &mut bottom.bottom,
                Val::Px(state.inset_bottom + dock_reserve_h),
            );
            if bottom.align_items != AlignItems::Center {
                bottom.align_items = AlignItems::Center;
            }
        } else {
            // Leave a horizontal lane for the bottom-right action dock so HUD does not overlap it.
            let dock_lane = action_dock_content_reserve_width(state.breakpoint, dock_scale.0);
            assign_if_ne(
                &mut bottom.right,
                Val::Px(state.inset_right + dock_lane),
            );
            assign_if_ne(&mut bottom.bottom, Val::Px(state.inset_bottom));
            if bottom.align_items != AlignItems::FlexStart {
                bottom.align_items = AlignItems::FlexStart;
            }
        }

        bottom.flex_direction = FlexDirection::Column;
        bottom.justify_content = JustifyContent::FlexEnd;
    }

    {
        let mut dock_q = regions.p2();
        let Ok(mut dock) = dock_q.single_mut() else { return };
        let compact = matches!(state.breakpoint, UiBreakpoint::Compact);
        if compact {
            assign_if_ne(&mut dock.left, Val::Px(state.inset_left));
            assign_if_ne(&mut dock.right, Val::Px(state.inset_right));
            assign_if_ne(&mut dock.bottom, Val::Px(state.inset_bottom));
            dock.flex_direction = FlexDirection::Row;
            if dock.justify_content != JustifyContent::Center {
                dock.justify_content = JustifyContent::Center;
            }
            if dock.align_items != AlignItems::FlexEnd {
                dock.align_items = AlignItems::FlexEnd;
            }
            assign_if_ne(&mut dock.column_gap, Val::Px(SPACE_2));
            assign_if_ne(&mut dock.row_gap, Val::Px(0.0));
        } else {
            assign_if_ne(&mut dock.left, Val::Auto);
            assign_if_ne(&mut dock.right, Val::Px(state.inset_right));
            assign_if_ne(&mut dock.bottom, Val::Px(state.inset_bottom));
            dock.flex_direction = FlexDirection::Row;
            if dock.justify_content != JustifyContent::FlexEnd {
                dock.justify_content = JustifyContent::FlexEnd;
            }
            if dock.align_items != AlignItems::FlexEnd {
                dock.align_items = AlignItems::FlexEnd;
            }
            assign_if_ne(&mut dock.column_gap, Val::Px(SPACE_2));
            assign_if_ne(&mut dock.row_gap, Val::Px(SPACE_2));
        }
    }
}

/// `(cell_side, gap, inner_padding)` in px for the 3×2 build command grid (matches `spawn_action_dock` in client).
#[inline]
pub fn action_dock_grid_layout_sizes(dock_s: f32) -> (f32, f32, f32) {
    let s = dock_s.clamp(0.35, 3.0);
    (52.0 * s, 4.0 * s, 8.0 * s)
}

/// Outer width of the bordered build grid panel (3 columns × 2 rows).
#[inline]
pub fn action_dock_grid_outer_width_px(dock_s: f32) -> f32 {
    let (cell, gap, pad) = action_dock_grid_layout_sizes(dock_s);
    let frame = 2.0 * dock_s.clamp(0.35, 3.0);
    pad.mul_add(2.0, 3.0_f32.mul_add(cell, 2.0 * gap)) + frame * 2.0
}

/// Total width to reserve on the bottom sheet so content clears the action dock (grid + gap + HOME).
#[inline]
pub fn action_dock_content_reserve_width(bp: UiBreakpoint, dock_s: f32) -> f32 {
    action_dock_grid_outer_width_px(dock_s) + SPACE_2 + home_button_width_for_breakpoint(bp) + 8.0
}

/// HOME hit box width (matches `system_apply_responsive_hud_layout` in `main.rs`).
#[inline]
pub fn home_button_width_for_breakpoint(bp: UiBreakpoint) -> f32 {
    home_button_size_for_breakpoint(bp) * 1.6
}

/// HOME hit box height (square side used for width = side × 1.6).
#[inline]
pub fn home_button_size_for_breakpoint(bp: UiBreakpoint) -> f32 {
    match bp {
        UiBreakpoint::Compact => 44.0,
        UiBreakpoint::Medium => 52.0,
        UiBreakpoint::Wide => 60.0,
    }
}

pub fn touch_target_for_breakpoint(bp: UiBreakpoint) -> (f32, f32) {
    match bp {
        UiBreakpoint::Compact => (44.0, 40.0),
        UiBreakpoint::Medium => (40.0, 34.0),
        UiBreakpoint::Wide => (32.0, 28.0),
    }
}

pub fn panel_radius_for_breakpoint(bp: UiBreakpoint) -> f32 {
    match bp {
        UiBreakpoint::Compact => BUTTON_RADIUS,
        UiBreakpoint::Medium | UiBreakpoint::Wide => PANEL_RADIUS,
    }
}
