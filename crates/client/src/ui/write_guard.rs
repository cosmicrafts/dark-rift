//! Compare-before-write helpers so UI components are not marked dirty every frame.

use bevy::prelude::*;

pub const UI_F32_EPS: f32 = 1e-4;

#[inline]
pub fn approx_eq_f32(a: f32, b: f32) -> bool {
    (a - b).abs() < UI_F32_EPS
}

#[inline]
pub fn assign_if_ne<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot != value {
        *slot = value;
        true
    } else {
        false
    }
}

#[inline]
pub fn set_text_font_size_if_changed(tf: &mut TextFont, next: f32) {
    if !approx_eq_f32(tf.font_size, next) {
        tf.font_size = next;
    }
}

#[inline]
pub fn set_display_if_changed(node: &mut Node, next: Display) {
    if node.display != next {
        node.display = next;
    }
}

#[inline]
pub fn set_visibility_if_changed(vis: &mut Visibility, next: Visibility) {
    if *vis != next {
        *vis = next;
    }
}
