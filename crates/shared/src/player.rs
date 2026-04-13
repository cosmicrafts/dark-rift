//! Player state and management.

use serde::{Deserialize, Serialize};

use crate::pseudo_random::PseudoRandom;

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

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.clamp(0.0, 1.0).fract();
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [r, g, b]
}

/// Deterministic pastel territory color for bots (OpenFront-style: hue + moderate saturation).
/// PRNG stream is independent of spawn-placement RNG so map layout stays unchanged.
pub fn bot_territory_color(game_seed: u64, bot_id: u16) -> [f32; 3] {
    let mix = game_seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (bot_id as u64).wrapping_shl(32)
        ^ (bot_id as u64);
    let mut rng = PseudoRandom::new(mix);
    let h = rng.next_int(0, 10_000) as f32 / 10_000.0;
    // Saturated enough to read as a color on the map, still softer than `player_colors()`.
    let s = 0.42 + rng.next_int(0, 1000) as f32 / 1000.0 * 0.24;
    let v = 0.54 + rng.next_int(0, 1000) as f32 / 1000.0 * 0.22;
    let [r, g, b] = hsv_to_rgb(h, s, v);
    [
        r.clamp(0.14, 0.92),
        g.clamp(0.14, 0.92),
        b.clamp(0.14, 0.92),
    ]
}
