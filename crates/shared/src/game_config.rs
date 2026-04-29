use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BotConfig {
    pub attack_interval_base: u64,
    pub trigger_ratio: f64,
    pub reserve_ratio: f64,
    pub expand_ratio: f64,
    pub random_attack_div: i32,
    pub sam_ratio: f64,
    pub defense_post_ratio: f64,
    pub port_ratio: f64,
    pub factory_ratio: f64,
    pub silo_ratio: f64,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            attack_interval_base: 250,
            trigger_ratio: 0.6,
            reserve_ratio: 0.3,
            expand_ratio: 0.15,
            random_attack_div: 5,
            sam_ratio: 0.20,
            defense_post_ratio: 0.25,
            port_ratio: 0.75,
            factory_ratio: 0.75,
            silo_ratio: 0.20,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, bevy::prelude::Resource)]
pub struct BotProfile {
    pub attack_interval_ticks: u64,
    pub trigger_ratio: f64,
    pub reserve_ratio: f64,
    pub expand_ratio: f64,
}

impl Default for BotProfile {
    fn default() -> Self {
        Self {
            attack_interval_ticks: 240, // 4 seconds by default
            trigger_ratio: 0.6,
            reserve_ratio: 0.3,
            expand_ratio: 0.15,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, bevy::prelude::Resource)]
pub struct GameConfig {
    pub max_players: u32,
    pub bot_count: u32,
    pub bot: BotConfig,

    // Map Config
    pub map_name: String,
    pub map_width: u32,
    pub map_height: u32,
    pub random_spawn: bool,
    // Gameplay Modifiers
    pub tick_rate_ms: f32,
    pub attack_cost_enemy: f64,
    pub attack_cost_neutral: f64,
    pub neutral_expansion_multiplier: f64,
    pub pvp_expansion_multiplier: f64,
    pub bot_attack_interval_ticks: u64,
    
    // Economy Modifiers
    pub starting_troops: f64,
    pub starting_gold: f64,
    pub gold_base_income: f64,
    pub troop_base_income: f64,
    pub troop_per_tile: f64,
    pub max_troops_base: f64,
    pub max_troops_scale: f64,
    pub city_max_troops_per_level: f64,
    pub factory_income_bonus_per_level: f64,
    pub factory_income_bonus_cap: f64,
    pub gold_income_per_city_level: f64,
    /// Multiplier for train trade gold (OpenFront `goldMultiplierFor`).
    #[serde(default = "default_train_gold_multiplier")]
    pub train_gold_multiplier: f64,
}

fn default_train_gold_multiplier() -> f64 {
    1.0
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            max_players: 12,
            bot_count: 200,
            bot: BotConfig::default(),
            map_name: "europe".to_string(),
            map_width: 2904,
            map_height: 1672,
            random_spawn: false,
            tick_rate_ms: 50.0,
            attack_cost_enemy: 5.0,
            attack_cost_neutral: 1.0,
            neutral_expansion_multiplier: 2.4,
            pvp_expansion_multiplier: 1.5,
            bot_attack_interval_ticks: 24,
            starting_troops: 100.0,
            starting_gold: 250.0,
            gold_base_income: 1.25,
            troop_base_income: 2.0,
            troop_per_tile: 0.05,
            max_troops_base: 100.0,
            max_troops_scale: 50.0,
            city_max_troops_per_level: 2000.0,
            factory_income_bonus_per_level: 0.10,
            factory_income_bonus_cap: 1.50,
            gold_income_per_city_level: 0.35,
            train_gold_multiplier: 1.0,
        }
    }
}