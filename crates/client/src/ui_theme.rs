use bevy::prelude::*;

pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;

pub const PANEL_RADIUS: f32 = 8.0;
pub const BUTTON_RADIUS: f32 = 12.0;

#[inline]
pub fn panel_bg() -> Color {
    Color::srgba(0.1, 0.11, 0.14, 0.9)
}

#[inline]
pub fn panel_border() -> Color {
    Color::srgba(0.32, 0.34, 0.4, 0.95)
}

#[inline]
pub fn text_primary() -> Color {
    Color::srgba(0.92, 0.94, 0.98, 1.0)
}

#[inline]
pub fn text_secondary() -> Color {
    Color::srgba(0.72, 0.82, 0.94, 0.95)
}

#[inline]
pub fn accent_primary() -> Color {
    Color::srgb(0.35, 0.65, 0.95)
}

#[inline]
pub fn accent_danger() -> Color {
    Color::srgba(0.52, 0.2, 0.18, 0.88)
}

#[inline]
pub fn accent_danger_border() -> Color {
    Color::srgba(0.95, 0.5, 0.45, 0.85)
}

#[inline]
pub fn accent_home() -> Color {
    Color::srgb(0.9, 0.4, 0.0)
}

#[inline]
pub fn accent_home_hover() -> Color {
    Color::srgb(1.0, 0.5, 0.0)
}

#[inline]
pub fn accent_home_pressed() -> Color {
    Color::srgb(0.7, 0.3, 0.0)
}
