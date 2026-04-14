use bevy::prelude::*;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct UiSettingsConfig {
    /// Reserved for future AssetServer-based font override; labels use the embedded TTF.
    #[allow(dead_code)]
    pub font_path: String,
    pub font_size: f32,
    pub outline_thickness: f32,
    pub outline_color: (f32, f32, f32, f32),
    pub label_offset_y: f32,
}

impl Default for UiSettingsConfig {
    fn default() -> Self {
        Self {
            font_path: "".to_string(),
            font_size: 16.0,
            outline_thickness: 1.0,
            outline_color: (0.0, 0.0, 0.0, 1.0),
            label_offset_y: 12.0,
        }
    }
}

/// Helper method to convert the struct's tuple color to Bevy Color
impl UiSettingsConfig {
    pub fn get_outline_color(&self) -> Color {
        Color::srgba(
            self.outline_color.0,
            self.outline_color.1,
            self.outline_color.2,
            self.outline_color.3,
        )
    }
}

#[derive(Resource)]
pub struct UiSettings {
    pub config: UiSettingsConfig,
    pub font: Handle<Font>,
}

pub fn load_ui_settings(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    // Independent of process CWD (fixes `cargo run` from crates/client vs workspace root).
    let ron_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui_settings.ron");

    let config = match std::fs::read_to_string(&ron_path) {
        Ok(s) => match ron::from_str::<UiSettingsConfig>(&s) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::warn!("ui_settings.ron parse error ({}): {}", ron_path.display(), e);
                UiSettingsConfig::default()
            }
        },
        Err(e) => {
            bevy::log::warn!(
                "Could not read {}: {}; using defaults",
                ron_path.display(),
                e
            );
            UiSettingsConfig::default()
        }
    };

    // Embed font so labels render even when AssetServer can't resolve `assets/` (wrong CWD).
    // `font_path` in ui_settings.ron is kept for future hot-reload / overrides only.
    let font_bytes = include_bytes!("../../../assets/fonts/Rajdhani-Medium.ttf");
    let font_handle = match Font::try_from_bytes(font_bytes.to_vec()) {
        Ok(f) => fonts.add(f),
        Err(e) => {
            bevy::log::error!("Rajdhani font bytes invalid: {e}; map labels may be invisible");
            Handle::default()
        }
    };
    commands.insert_resource(UiSettings {
        config,
        font: font_handle,
    });
}
