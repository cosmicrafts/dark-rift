use bevy::prelude::*;

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};

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
            ortho.scale = (ortho.scale * (1.0 + zoom)).clamp(0.1, 20.0);
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
                // Prefer touch on mobile (avoids DPI-scaled emulated MouseMotion spikes)
                raw_drag = touch_delta;
                let _ = motion_evr.read().count(); // Drain mouse events
                raw_drag * 0.5 // Dampen mobile sensitivity significantly
            } else {
                for ev in motion_evr.read() {
                    raw_drag += ev.delta;
                }
                raw_drag * 0.85 // Slightly dampen desktop mouse
            };

            drag_state.accumulated_drag += raw_drag.length();

            if drag_delta != Vec2::ZERO {
                // Screen coordinates: +X is right, +Y is down
                // World coordinates: +X is right, +Y is up
                // So dragging mouse Right: delta.x > 0 -> Move camera Left (subtract)
                // Dragging mouse Down: delta.y > 0 -> Move camera Up (add)
                pan.x -= drag_delta.x * ortho.scale;
                pan.y += drag_delta.y * ortho.scale;
            }
        }
        
        transform.translation.x += pan.x;
        transform.translation.y += pan.y;
    }
}
