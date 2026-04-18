use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum BotDifficulty {
    BrainDead,
    #[default]
    Vanilla,
    Terminator,
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
    pub bot_difficulty: BotDifficulty,

    // Map Config
    pub map_name: String,
    pub map_width: u32,
    pub map_height: u32,

    // Game Rules
    pub random_spawn: bool,
    pub base_income: f32,
    pub attack_cost: f32,
    pub expand_cost: f32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            max_players: 2,
            bot_count: 150,
            bot_difficulty: BotDifficulty::Vanilla,
            map_name: "europe".to_string(),
            map_width: 2904,
            map_height: 1672,
            random_spawn: false,
            base_income: 1.0,
            attack_cost: 0.15,
            expand_cost: 0.2,
        }
    }
}
