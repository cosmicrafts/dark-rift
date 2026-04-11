//! Player state and management.

use serde::{Deserialize, Serialize};

/// Unique player identifier.
pub type PlayerId = u32;

/// Types of players in the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerType {
    /// A human player connected via the network.
    Human,
    /// An AI faction that holds territory.
    Faction,
}

/// Represents a player (human or AI faction) in the game.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Player {
    /// Unique player ID. For factions, this matches the Faction::id.
    pub id: PlayerId,
    /// Display name.
    pub name: String,
    /// Player type.
    pub player_type: PlayerType,
    /// Total ships available (not garrisoned in systems).
    pub reserve_ships: f32,
    /// RGB color for this player's territory.
    pub color: [f32; 3],
    /// Whether the player is still in the game.
    pub alive: bool,
    /// Whether the player has completed spawn selection.
    pub has_spawned: bool,
}

impl Player {
    /// Create a new human player.
    pub fn new_human(id: PlayerId, name: String, color: [f32; 3]) -> Self {
        Self {
            id,
            name,
            player_type: PlayerType::Human,
            reserve_ships: 0.0,
            color,
            alive: true,
            has_spawned: false,
        }
    }

    /// Create a new AI faction player.
    pub fn new_faction(id: PlayerId, name: String, color: [f32; 3]) -> Self {
        Self {
            id,
            name,
            player_type: PlayerType::Faction,
            reserve_ships: 0.0,
            color,
            alive: true,
            has_spawned: true, // Factions spawn automatically.
        }
    }

    /// Check if this player is a human.
    pub fn is_human(&self) -> bool {
        self.player_type == PlayerType::Human
    }
}

/// Color palette for human players.
pub fn human_player_colors() -> Vec<[f32; 3]> {
    vec![
        [0.0, 0.8, 1.0],   // Cyan
        [1.0, 0.3, 0.3],   // Red
        [0.3, 1.0, 0.3],   // Green
        [1.0, 0.8, 0.0],   // Yellow
        [1.0, 0.4, 0.8],   // Pink
        [0.6, 0.4, 1.0],   // Purple
        [1.0, 0.6, 0.2],   // Orange
        [0.4, 0.9, 0.8],   // Mint
    ]
}
