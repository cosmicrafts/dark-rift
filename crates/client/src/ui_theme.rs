use bevy::prelude::*;

#[inline]
pub fn panel_bg() -> Color {
    Color::srgba(0.1, 0.11, 0.14, 0.9)
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
