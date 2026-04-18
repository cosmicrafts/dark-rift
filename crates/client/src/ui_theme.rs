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

/// Outer lobby backdrop (mockup: deep space blue).
#[inline]
pub fn menu_backdrop() -> Color {
    Color::srgba(0.039, 0.078, 0.114, 0.97)
}

/// Main framed panel fill (~#0A141D).
#[inline]
pub fn menu_panel_bg() -> Color {
    Color::srgba(0.039, 0.078, 0.114, 0.92)
}

#[inline]
pub fn menu_panel_border_glow() -> Color {
    Color::srgba(0.0, 0.83, 1.0, 0.55)
}

/// Primary solo CTA (mockup neon cyan).
#[inline]
pub fn accent_solo_cyan() -> Color {
    Color::srgb(0.0, 0.83, 1.0)
}

#[inline]
pub fn accent_solo_cyan_hover() -> Color {
    Color::srgb(0.2, 0.92, 1.0)
}

#[inline]
pub fn accent_solo_cyan_pressed() -> Color {
    Color::srgb(0.0, 0.55, 0.78)
}

/// Ranked CTA (mockup gold).
#[inline]
pub fn accent_ranked_gold() -> Color {
    Color::srgb(0.75, 0.58, 0.25)
}

#[inline]
pub fn accent_ranked_gold_hover() -> Color {
    Color::srgb(0.88, 0.72, 0.35)
}

#[inline]
pub fn accent_ranked_gold_pressed() -> Color {
    Color::srgb(0.55, 0.42, 0.15)
}

#[inline]
pub fn menu_secondary_button() -> Color {
    Color::srgba(0.14, 0.16, 0.2, 0.95)
}

#[inline]
pub fn menu_secondary_button_hover() -> Color {
    Color::srgba(0.22, 0.26, 0.32, 0.98)
}

#[inline]
pub fn nickname_field_bg() -> Color {
    Color::srgba(0.06, 0.09, 0.12, 0.95)
}

#[inline]
pub fn nickname_field_border() -> Color {
    Color::srgba(0.0, 0.65, 0.9, 0.45)
}

#[inline]
pub fn nickname_field_border_focused() -> Color {
    Color::srgba(0.0, 0.85, 1.0, 0.85)
}

#[inline]
pub fn menu_footer_bg() -> Color {
    Color::srgba(0.02, 0.04, 0.06, 0.88)
}
