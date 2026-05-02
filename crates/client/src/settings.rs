use bevy::prelude::*;
use rand::Rng;
use serde::{Deserialize, Serialize};

fn default_true() -> bool { true }

fn default_false() -> bool {
    false
}

fn default_attack_ratio() -> f32 {
    0.2
}

/// Default HUD / map feedback scale (plan: ~2× vs legacy hardcoded sizes).
fn default_hud_scale_2() -> f32 {
    2.0
}

fn default_border_darken() -> f32 {
    0.6
}

fn default_border_neighbor_tiles() -> f32 {
    1.0
}

/// GPU map border styling (`BorderMaterial` / `border.wgsl`), merged from legacy `visuals.border.{band,tint}`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BorderVisualSettings {
    /// Cardinal neighbor sample offset in **tile** units (fragment shader converts to `i32`).
    #[serde(default = "default_border_neighbor_tiles")]
    pub neighbor_offset_tiles: f32,
    /// RGB multiplier along territory borders (lower = darker band; legacy tint ~0.25).
    #[serde(default = "default_border_darken")]
    pub darken: f32,
}

impl Default for BorderVisualSettings {
    fn default() -> Self {
        Self {
            neighbor_offset_tiles: 1.0,
            darken: 0.6,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
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
    /// GPU water simplex-noise animation. Disable on low-end mobile GPUs for ~30% frag savings.
    #[serde(default = "default_true")]
    pub enable_water_animation: bool,
    #[serde(default)]
    pub border: BorderVisualSettings,
}

impl Default for VisualSettingsConfig {
    fn default() -> Self {
        Self {
            enable_combat_particles: false,
            enable_hdr_bloom: true,
            enable_territory_expansion_animations: true,
            enable_floating_labels: true,
            enable_water_animation: true,
            border: BorderVisualSettings::default(),
        }
    }
}

impl VisualSettingsConfig {
    #[inline]
    pub fn hdr_bloom_enabled(&self) -> bool {
        self.enable_hdr_bloom
    }

}

#[derive(Debug, Deserialize, Serialize, Clone)]
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
    /// Annulus radii for the click attack ring mesh (`setup_attack_ring_mesh`).
    #[serde(default = "default_hud_scale_2")]
    pub click_feedback_scale: f32,
    /// Troop number label size and offset on the attack ping.
    #[serde(default = "default_hud_scale_2")]
    pub click_troop_label_scale: f32,
    /// Local transport disc mesh scale (`FleetVisualMeshes` boat annulus).
    #[serde(default = "default_hud_scale_2")]
    pub fleet_boat_scale: f32,
    /// Landing target ring mesh scale.
    #[serde(default = "default_hud_scale_2")]
    pub fleet_target_ring_scale: f32,
    /// Wake speck mesh scale.
    #[serde(default = "default_hud_scale_2")]
    pub fleet_wake_scale: f32,
    /// Bottom combat HUD: fonts, row paddings, slider, gaps (`spawn_bottom_combat_hud` + row sync).
    #[serde(default = "default_hud_scale_2")]
    pub combat_hud_scale: f32,
    /// Compact / mobile breakpoint: scale for dock + combat HUD (see `system_update_ui_shell_state`).
    #[serde(default = "default_hud_scale_2")]
    pub mobile_hud_scale: f32,
    /// Show outgoing `WarpFleet` payload rows with recall (OpenFront `AttacksDisplay` transport list).
    #[serde(default = "default_true")]
    pub combat_hud_fleet_rows: bool,
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
            click_feedback_scale: 2.0,
            click_troop_label_scale: 2.0,
            fleet_boat_scale: 2.0,
            fleet_target_ring_scale: 2.0,
            fleet_wake_scale: 2.0,
            combat_hud_scale: 2.0,
            mobile_hud_scale: 2.0,
            combat_hud_fleet_rows: true,
        }
    }
}

fn default_empty_string() -> String {
    String::new()
}

/// Player-chosen display name (menu + `ClientJoinMessage.name`). Persisted in `settings.ron` on native only.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ProfileConfig {
    #[serde(default = "default_empty_string")]
    pub nickname: String,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            nickname: String::new(),
        }
    }
}

impl ProfileConfig {
    pub const NICKNAME_MAX_CHARS: usize = 24;
}

/// Random default like `Anon318` (`Anon000`–`Anon999`). Safe on **wasm32** via workspace `getrandom` / `rand`.
#[must_use]
pub fn generate_default_anon_nickname() -> String {
    let n = rand::thread_rng().gen_range(0..1000u32);
    format!("Anon{n:03}")
}

/// If `profile.nickname` is empty/whitespace, assigns [`generate_default_anon_nickname`]. Returns whether it changed.
pub fn assign_default_anon_if_nickname_empty(config: &mut ClientSettingsConfig) -> bool {
    let trimmed = config.profile.nickname.trim();
    if trimmed.is_empty() || trimmed == "Anon697" {
        config.profile.nickname = generate_default_anon_nickname();
        true
    } else {
        false
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ClientSettingsConfig {
    #[serde(default)]
    pub visuals: VisualSettingsConfig,
    #[serde(default)]
    pub ui: UiSettingsConfig,
    #[serde(default)]
    pub profile: ProfileConfig,
    #[serde(default)]
    pub gameplay: dark_rift_shared::game_config::GameConfig,
}

impl ClientSettingsConfig {
    /// Nickname used for multiplayer join and single-player `PlayerInfo` (non-empty, length-capped).
    #[must_use]
    pub fn display_nickname_for_play(&self) -> String {
        let trimmed = self.profile.nickname.trim();
        if trimmed.is_empty() {
            return generate_default_anon_nickname();
        }
        trimmed
            .chars()
            .take(ProfileConfig::NICKNAME_MAX_CHARS)
            .collect()
    }

    /// Clamp stored nickname to max length (whitespace preserved for editing).
    pub fn clamp_profile_nickname_in_place(&mut self) {
        if self.profile.nickname.chars().count() <= ProfileConfig::NICKNAME_MAX_CHARS {
            return;
        }
        self.profile.nickname = self
            .profile
            .nickname
            .chars()
            .take(ProfileConfig::NICKNAME_MAX_CHARS)
            .collect();
    }
}

/// Path to `assets/settings.ron` relative to the client crate (native read/write).
#[must_use]
#[allow(dead_code)]
pub fn settings_ron_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/settings.ron")
}

/// Writes full client config to disk. **No-op on WASM** (no filesystem); nickname stays in memory for the session.
pub fn save_client_settings_config(config: &ClientSettingsConfig) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = config;
        bevy::log::info!(
            "settings save skipped on wasm32 (profile is session-only until localStorage is wired)"
        );
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = settings_ron_path();
        let pretty = ron::ser::PrettyConfig::new()
            .depth_limit(8)
            .separate_tuple_members(true);
        let body = ron::ser::to_string_pretty(config, pretty)
            .map_err(|e| format!("RON serialize: {e}"))?;
        std::fs::write(&path, body).map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(())
    }
}

#[derive(Resource)]
pub struct ClientSettings {
    pub config: ClientSettingsConfig,
    pub font: Handle<Font>,
}

/// Builds config from disk (or defaults). No Bevy required — safe to call before app.run().
///
/// Empty `profile.nickname` becomes a random `Anon###` once; on **native** that value is written back to `settings.ron`.
pub fn build_client_settings_config() -> ClientSettingsConfig {
    let mut config = {
        #[cfg(target_arch = "wasm32")]
        {
            const EMBEDDED_SETTINGS: &str = include_str!("../../../assets/settings.ron");
            match ron::from_str::<ClientSettingsConfig>(EMBEDDED_SETTINGS) {
                Ok(c) => c,
                Err(e) => {
                    bevy::log::warn!("embedded settings.ron parse error: {e}; using defaults");
                    ClientSettingsConfig::default()
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ron_path = settings_ron_path();
            match std::fs::read_to_string(&ron_path) {
                Ok(s) => match ron::from_str::<ClientSettingsConfig>(&s) {
                    Ok(c) => c,
                    Err(e) => {
                        bevy::log::warn!("settings.ron parse error ({}): {}", ron_path.display(), e);
                        ClientSettingsConfig::default()
                    }
                },
                Err(e) => {
                    bevy::log::warn!("Could not read {}: {}; using defaults", ron_path.display(), e);
                    ClientSettingsConfig::default()
                }
            }
        }
    };

    if assign_default_anon_if_nickname_empty(&mut config) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(e) = save_client_settings_config(&config) {
            bevy::log::warn!("Could not persist default Anon### nickname: {e}");
        }
    }

    config
}

/// System that patches the real font Handle into ClientSettings once Assets<Font> is available.
pub fn init_client_font(mut settings: ResMut<ClientSettings>, mut fonts: ResMut<Assets<Font>>) {
    let font_bytes = include_bytes!("../../../assets/fonts/Rajdhani-Medium.ttf");
    settings.font = match Font::try_from_bytes(font_bytes.to_vec()) {
        Ok(f) => fonts.add(f),
        Err(e) => {
            bevy::log::error!("Rajdhani font bytes invalid: {e}");
            Handle::default()
        }
    };
}

/// Legacy: kept for compatibility, delegates to build+patch.
#[allow(dead_code)]
pub fn load_client_settings(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let config = build_client_settings_config();
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
