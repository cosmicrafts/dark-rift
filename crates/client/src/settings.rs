use bevy::prelude::*;
use serde::Deserialize;

fn default_true() -> bool { true }

fn default_false() -> bool {
    false
}

fn default_attack_ratio() -> f32 {
    0.2
}

#[derive(Debug, Deserialize, Clone)]
pub struct VisualSettingsConfig {
    /// Reserved for a future particle path; Enoki was removed (despawn races + perf).
    #[serde(default = "default_false")]
    #[allow(dead_code)]
    pub enable_combat_particles: bool,
    #[serde(default = "default_true")]
    pub enable_hdr_bloom: bool,
    #[serde(default = "default_true")]
    pub enable_territory_expansion_animations: bool,
    #[serde(default = "default_true")]
    pub enable_floating_labels: bool,
}

impl Default for VisualSettingsConfig {
    fn default() -> Self {
        Self {
            enable_combat_particles: false,
            enable_hdr_bloom: true,
            enable_territory_expansion_animations: true,
            enable_floating_labels: true,
        }
    }
}

impl VisualSettingsConfig {
    #[inline]
    pub fn hdr_bloom_enabled(&self) -> bool {
        self.enable_hdr_bloom
    }

    #[inline]
    pub fn territory_expansion_animations_enabled(&self) -> bool {
        self.enable_territory_expansion_animations
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct UiSettingsConfig {
    /// Reserved for future AssetServer-based font override; labels use the embedded TTF.
    #[allow(dead_code)]
    pub font_path: String,
    /// Minimum label size; effective size is `font_size * scale` (scale ≥ 1 from territory).
    pub font_size: f32,
    pub label_offset_y: f32,
    /// At this many owned tiles, scale is 1.0; above that, size grows ~√(tiles/ref).
    pub label_size_ref_tiles: f32,
    /// Max multiplier over `font_size` so huge empires do not cover the map.
    pub label_max_scale: f32,
    /// Initial attack troop fraction (OpenFront `settings.attackRatio`); slider clamps to [0.01, 1.0].
    #[serde(default = "default_attack_ratio")]
    pub attack_ratio: f32,
}

impl Default for UiSettingsConfig {
    fn default() -> Self {
        Self {
            font_path: "".to_string(),
            font_size: 16.0,
            label_offset_y: 12.0,
            label_size_ref_tiles: 64.0,
            label_max_scale: 5.0,
            attack_ratio: 0.2,
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ClientSettingsConfig {
    #[serde(default)]
    pub visuals: VisualSettingsConfig,
    #[serde(default)]
    pub ui: UiSettingsConfig,
}

#[derive(Resource)]
pub struct ClientSettings {
    pub config: ClientSettingsConfig,
    pub font: Handle<Font>,
}

pub fn load_client_settings(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    // Independent of process CWD (fixes `cargo run` from crates/client vs workspace root).
    let ron_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/settings.ron");

    let config = match std::fs::read_to_string(&ron_path) {
        Ok(s) => match ron::from_str::<ClientSettingsConfig>(&s) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::warn!("settings.ron parse error ({}): {}", ron_path.display(), e);
                ClientSettingsConfig::default()
            }
        },
        Err(e) => {
            bevy::log::warn!(
                "Could not read {}: {}; using defaults",
                ron_path.display(),
                e
            );
            ClientSettingsConfig::default()
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
    commands.insert_resource(ClientSettings {
        config,
        font: font_handle,
    });
}
