//! Player state and management.

use serde::{Deserialize, Serialize};

/// Unique player identifier.
pub type PlayerId = u16;

/// Types of players in the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerType {
    /// A human player connected via the network.
    Human,
    /// An AI bot.
    Bot,
}

/// Represents a player (human or AI) in the game.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Player {
    /// Unique player ID.
    pub id: PlayerId,
    /// Display name.
    pub name: String,
    /// Player type.
    pub player_type: PlayerType,
    /// Current troop count (the single resource). Used to expand/attack.
    pub troops: f64,
    /// Maximum troops this player can hold (grows with territory).
    pub max_troops: f64,
    /// RGB color for this player's territory.
    pub color: [f32; 3],
    /// Whether the player is still in the game.
    pub alive: bool,
    /// Whether the player has completed initial spawn.
    pub has_spawned: bool,
}

impl Player {
    /// Create a new human player.
    pub fn new_human(id: PlayerId, name: String, color: [f32; 3]) -> Self {
        Self {
            id,
            name,
            player_type: PlayerType::Human,
            troops: 100.0,
            max_troops: 500.0,
            color,
            alive: true,
            has_spawned: false,
        }
    }

    /// Create a new bot player.
    pub fn new_bot(id: PlayerId, name: String, color: [f32; 3]) -> Self {
        Self {
            id,
            name,
            player_type: PlayerType::Bot,
            troops: 100.0,
            max_troops: 500.0,
            color,
            alive: true,
            has_spawned: false,
        }
    }

    /// Check if this player is a human.
    pub fn is_human(&self) -> bool {
        self.player_type == PlayerType::Human
    }
}

/// Color palette for players.
pub fn player_colors() -> Vec<[f32; 3]> {
    vec![
        [0.0, 0.75, 1.0],   // Cyan
        [1.0, 0.25, 0.25],  // Red
        [0.2, 0.9, 0.3],    // Green
        [1.0, 0.85, 0.0],   // Gold
        [1.0, 0.35, 0.75],  // Pink
        [0.55, 0.35, 1.0],  // Violet
        [1.0, 0.55, 0.15],  // Orange
        [0.35, 0.9, 0.8],   // Teal
    ]
}
