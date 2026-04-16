use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum BotDifficulty {
    BrainDead,
    #[default]
    Vanilla,
    Terminator,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, bevy::prelude::Resource)]
pub struct GameConfig {
    pub max_players: u32,
    pub bot_count: u32,
    pub bot_difficulty: BotDifficulty,

    // Map Config
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
            bot_count: 4,
            bot_difficulty: BotDifficulty::Vanilla,
            map_width: 200,
            map_height: 200,
            random_spawn: false,
            base_income: 1.0,
            attack_cost: 0.15,
            expand_cost: 0.2,
        }
    }
}
