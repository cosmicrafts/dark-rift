use bevy::prelude::*;

use crate::ui::layers::HUD_SHELL;
use crate::ui::theme::{BUTTON_RADIUS, PANEL_RADIUS, SPACE_2, SPACE_3, SPACE_4};

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

#[derive(Resource, Clone, Copy, Debug)]
pub struct UiShellState {
    pub breakpoint: UiBreakpoint,
    pub inset_top: f32,
    pub inset_right: f32,
    pub inset_bottom: f32,
    pub inset_left: f32,
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
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    let Ok(window) = windows.single() else { return };
    let width = window.width();
    let height = window.height();
    let aspect = if height > 0.0 { width / height } else { 1.0 };

    let breakpoint = if width < 600.0 || aspect < 0.80 {
        UiBreakpoint::Compact
    } else if width < 1100.0 {
        UiBreakpoint::Medium
    } else {
        UiBreakpoint::Wide
    };

    let inset = match breakpoint {
        UiBreakpoint::Compact => SPACE_2,
        UiBreakpoint::Medium => SPACE_3,
        UiBreakpoint::Wide => SPACE_4,
    };

    *state = UiShellState {
        breakpoint,
        inset_top: inset,
        inset_right: inset,
        inset_bottom: inset,
        inset_left: inset,
    };
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
        top.align_items = AlignItems::Center;
        top.column_gap = Val::Px(SPACE_2);
    }

    {
        let mut bottom_q = regions.p1();
        let Ok(mut bottom) = bottom_q.single_mut() else { return };
        // Same bottom inset as the action dock so combat HUD sits flush with HOME vertically.
        // Reserve the bottom-right corner so the centered column does not sit under the build grid + HOME.
        let dock_reserve = action_dock_content_reserve_width(state.breakpoint, dock_scale.0);
        bottom.left = Val::Px(state.inset_left);
        bottom.right = Val::Px(state.inset_right + dock_reserve);
        bottom.bottom = Val::Px(state.inset_bottom);
        bottom.flex_direction = FlexDirection::Column;
        bottom.justify_content = JustifyContent::FlexEnd;
        bottom.align_items = AlignItems::Center;
    }

    {
        let mut dock_q = regions.p2();
        let Ok(mut dock) = dock_q.single_mut() else { return };
        dock.right = Val::Px(state.inset_right);
        dock.bottom = Val::Px(state.inset_bottom);
        dock.flex_direction = FlexDirection::Row;
        dock.justify_content = JustifyContent::FlexEnd;
        dock.align_items = AlignItems::FlexEnd;
        dock.column_gap = Val::Px(SPACE_2);
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
