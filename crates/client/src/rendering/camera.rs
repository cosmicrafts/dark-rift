use bevy::prelude::*;

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};

/// Matches wheel / pinch zoom clamp in [`camera_movement_and_zoom`].
pub const CAMERA_MIN_SCALE: f32 = 0.1;
pub const CAMERA_MAX_SCALE: f32 = 20.0;
/// Five line-scroll zoom-out steps from max zoom-in: `CAMERA_MIN_SCALE * 1.1^5`.
pub const CAMERA_START_SCALE: f32 = 0.161051;

/// Focus at world XY, fixed Z; orthographic scale set for a close default view when a match starts.
pub fn reset_game_camera(transform: &mut Transform, projection: &mut Projection, focus: Vec2) {
    transform.translation.x = focus.x;
    transform.translation.y = focus.y;
    transform.translation.z = 999.0;
    if let Projection::Orthographic(ref mut ortho) = *projection {
        ortho.scale = CAMERA_START_SCALE.clamp(CAMERA_MIN_SCALE, CAMERA_MAX_SCALE);
    }
}

/// Neutral lobby camera (matches [`crate::setup_live`] spawn pose).
pub fn reset_lobby_camera(transform: &mut Transform, projection: &mut Projection) {
    transform.translation = Vec3::new(0.0, 0.0, 999.0);
    if let Projection::Orthographic(ref mut ortho) = *projection {
        ortho.scale = 1.0;
    }
}

#[derive(Resource, Default)]
pub struct CameraDragState {
    pub accumulated_drag: f32,
    pub is_dragging: bool,
}

#[derive(Resource, Default)]
pub struct PinchState {
    pub last_distance: Option<f32>,
}

#[derive(Resource, Default)]
pub struct TouchGestureState {
    pub max_touches_in_gesture: u8,
    pub last_touch_pos: Option<Vec2>,
}

/// Low-pass filter for mouse-drag pan so desktop feels closer to touch (less jitter), without touching touch/pinch paths.
#[derive(Resource, Default)]
pub struct DesktopCameraPanSmooth {
    pub smoothed_delta: Vec2,
}

/// One-finger pan multiplier for touch (bak baseline used 1.0 vs older 0.5 damp).
const MOBILE_PAN_MULTIPLIER: f32 = 1.0;
/// Raw mouse motion scale before smoothing.
const DESKTOP_PAN_RAW_MULTIPLIER: f32 = 0.88;
/// Higher = snappier tracking; lower = smoother ease (per frame lerp toward raw drag).
const DESKTOP_PAN_SMOOTH: f32 = 0.52;

pub fn camera_movement_and_zoom(
    mut q_camera: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mouse_input: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    mut scroll_evr: MessageReader<MouseWheel>,
    mut motion_evr: MessageReader<bevy::input::mouse::MouseMotion>,
    mut drag_state: ResMut<CameraDragState>,
    mut pinch_state: ResMut<PinchState>,
    mut touch_gesture: ResMut<TouchGestureState>,
    mut desktop_pan_smooth: ResMut<DesktopCameraPanSmooth>,
) {
    if mouse_input.just_pressed(MouseButton::Left) || touches.any_just_pressed() {
        drag_state.is_dragging = true;
        drag_state.accumulated_drag = 0.0;
    }
    if mouse_input.just_released(MouseButton::Left) || touches.any_just_released() {
        drag_state.is_dragging = false;
    }

    // Touch dragging (if available) - calculate average delta
    let mut touch_delta = Vec2::ZERO;
    let mut touch_count = 0;
    for touch in touches.iter() {
        touch_delta += touch.delta();
        touch_count += 1;
    }
    if touch_count > 0 {
        touch_delta /= touch_count as f32;
    }
    if touch_count > 0 {
        if touch_gesture.max_touches_in_gesture == 0 || touches.any_just_pressed() {
            touch_gesture.max_touches_in_gesture = touch_count as u8;
        } else {
            touch_gesture.max_touches_in_gesture =
                touch_gesture.max_touches_in_gesture.max(touch_count as u8);
        }
    }
    if let Some(touch) = touches.iter().next() {
        touch_gesture.last_touch_pos = Some(touch.position());
    } else if !touches.any_just_released() {
        touch_gesture.last_touch_pos = None;
        touch_gesture.max_touches_in_gesture = 0;
    }

    // Two-finger pinch zoom (mobile/tablet). Keep this branch isolated so
    // desktop wheel zoom and one-finger pan semantics remain unchanged.
    let mut touch_a = None;
    let mut touch_b = None;
    let mut extra_touches = false;
    for touch in touches.iter() {
        if touch_a.is_none() {
            touch_a = Some(touch.position());
        } else if touch_b.is_none() {
            touch_b = Some(touch.position());
        } else {
            extra_touches = true;
            break;
        }
    }
    let is_pinching = touch_a.is_some() && touch_b.is_some() && !extra_touches;
    let mut pinch_zoom = 0.0_f32;
    if let (Some(a), Some(b)) = (touch_a, touch_b) {
        let distance = a.distance(b);
        if is_pinching {
            if let Some(prev_distance) = pinch_state.last_distance {
                let delta = distance - prev_distance;
                pinch_zoom = -delta * 0.004;
            }
            pinch_state.last_distance = Some(distance);
            // Treat pinch as non-tap interaction.
            drag_state.accumulated_drag = drag_state.accumulated_drag.max(11.0);
        } else {
            pinch_state.last_distance = None;
        }
    } else {
        pinch_state.last_distance = None;
    }

    for (mut transform, mut projection) in q_camera.iter_mut() {
        let bevy::prelude::Projection::Orthographic(ref mut ortho) = *projection else { continue };
        
        // Zoom
        let mut zoom = 0.0;
        for ev in scroll_evr.read() {
            match ev.unit {
                MouseScrollUnit::Line => zoom -= ev.y * 0.1,
                MouseScrollUnit::Pixel => zoom -= ev.y * 0.005,
            }
        }
        
        zoom += pinch_zoom;
        if zoom != 0.0 {
            ortho.scale = (ortho.scale * (1.0 + zoom)).clamp(CAMERA_MIN_SCALE, CAMERA_MAX_SCALE);
        }
        
        // Pan
        let mut pan = Vec2::ZERO;
        let speed = 20.0 * ortho.scale;
        
        if keyboard_input.pressed(KeyCode::KeyW) || keyboard_input.pressed(KeyCode::ArrowUp) {
            pan.y += speed;
        }
        if keyboard_input.pressed(KeyCode::KeyS) || keyboard_input.pressed(KeyCode::ArrowDown) {
            pan.y -= speed;
        }
        if keyboard_input.pressed(KeyCode::KeyA) || keyboard_input.pressed(KeyCode::ArrowLeft) {
            pan.x -= speed;
        }
        if keyboard_input.pressed(KeyCode::KeyD) || keyboard_input.pressed(KeyCode::ArrowRight) {
            pan.x += speed;
        }
        
        // Mouse/Touch Drag Pan
        if drag_state.is_dragging && !is_pinching {
            let mut raw_drag = Vec2::ZERO;
            let drag_delta = if touches.iter().count() > 0 {
                desktop_pan_smooth.smoothed_delta = Vec2::ZERO;
                raw_drag = touch_delta;
                let _ = motion_evr.read().count(); // Drain mouse events
                raw_drag * MOBILE_PAN_MULTIPLIER
            } else {
                for ev in motion_evr.read() {
                    raw_drag += ev.delta;
                }
                raw_drag *= DESKTOP_PAN_RAW_MULTIPLIER;
                desktop_pan_smooth.smoothed_delta = desktop_pan_smooth
                    .smoothed_delta
                    .lerp(raw_drag, DESKTOP_PAN_SMOOTH);
                desktop_pan_smooth.smoothed_delta
            };

            drag_state.accumulated_drag += drag_delta.length();

            if drag_delta != Vec2::ZERO {
                // Screen coordinates: +X is right, +Y is down
                // World coordinates: +X is right, +Y is up
                // So dragging mouse Right: delta.x > 0 -> Move camera Left (subtract)
                // Dragging mouse Down: delta.y > 0 -> Move camera Up (add)
                pan.x -= drag_delta.x * ortho.scale;
                pan.y += drag_delta.y * ortho.scale;
            }
        } else {
            desktop_pan_smooth.smoothed_delta = Vec2::ZERO;
        }
        
        transform.translation.x += pan.x;
        transform.translation.y += pan.y;
    }
}
