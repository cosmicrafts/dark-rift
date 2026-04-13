use bevy::prelude::*;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct UiSettingsConfig {
    pub font_path: String,
    pub font_size: f32,
    pub outline_thickness: f32,
    pub outline_color: (f32, f32, f32, f32),
    pub name_color_ally: (f32, f32, f32, f32),
    pub name_color_enemy: (f32, f32, f32, f32),
    pub label_offset_y: f32,
}

impl Default for UiSettingsConfig {
    fn default() -> Self {
        Self {
            font_path: "".to_string(),
            font_size: 16.0,
            outline_thickness: 1.0,
            outline_color: (0.0, 0.0, 0.0, 1.0),
            name_color_ally: (0.2, 1.0, 0.4, 1.0),
            name_color_enemy: (1.0, 0.35, 0.28, 1.0),
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
    
    pub fn get_ally_color(&self) -> Color {
        Color::srgba(
            self.name_color_ally.0,
            self.name_color_ally.1,
            self.name_color_ally.2,
            self.name_color_ally.3,
        )
    }
    
    pub fn get_enemy_color(&self) -> Color {
        Color::srgba(
            self.name_color_enemy.0,
            self.name_color_enemy.1,
            self.name_color_enemy.2,
            self.name_color_enemy.3,
        )
    }
}

#[derive(Resource)]
pub struct UiSettings {
    pub config: UiSettingsConfig,
    pub font: Handle<Font>,
}

pub fn load_ui_settings(mut commands: Commands, asset_server: Res<AssetServer>) {
    let ron_path = "assets/ui_settings.ron";
    
    let config = match std::fs::read_to_string(ron_path) {
        Ok(s) => match ron::from_str::<UiSettingsConfig>(&s) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::warn!("ui_settings.ron parse error: {}", e);
                UiSettingsConfig::default()
            }
        },
        Err(e) => {
            bevy::log::warn!("Could not read ui_settings.ron: {}; using defaults", e);
            UiSettingsConfig::default()
        }
    };
    
    let font_handle = if !config.font_path.is_empty() {
        asset_server.load(&config.font_path)
    } else {
        Handle::default()
    };
    
    commands.insert_resource(UiSettings {
        config,
        font: font_handle,
    });
}
