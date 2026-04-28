//! Player state and management.

use serde::{Deserialize, Serialize};

use bevy_prng::WyRand;
use rand_core::SeedableRng;
use crate::rng::NextIntExt;

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

fn default_player_gold() -> f64 {
    crate::config::STARTING_GOLD
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
    /// Current troop count. Used to expand/attack and launch fleets.
    pub troops: f64,
    /// Maximum troops this player can hold (grows with territory).
    pub max_troops: f64,
    /// Gold for structures and upgrades (attacks/expansion still use troops).
    #[serde(default = "default_player_gold")]
    pub gold: f64,
    /// RGB color for this player's territory.
    pub color: [f32; 3],
    /// Whether the player is still in the game.
    pub alive: bool,
    /// Whether the player has completed initial spawn.
    pub has_spawned: bool,
    /// Sum of x coordinates of all owned tiles (for O(1) center calculation).
    pub sum_x: u64,
    /// Sum of y coordinates of all owned tiles (for O(1) center calculation).
    pub sum_y: u64,
    /// Number of tiles owned by the player.
    pub tile_count: u32,
    /// Flat cache of linear indices of border tiles. Order is insertion-deterministic
    /// across all platforms because the simulation loop is deterministic.
    /// Uses Vec for contiguous-memory iteration (cache-line friendly on every tick).
    pub border_tiles: Vec<u32>,
    /// RNG stream for bot logic. Persists across ticks and serializes natively.
    pub bot_rng: WyRand,
    /// Number of factories owned.
    pub factories: u32,
    /// Number of cities owned.
    pub cities: u32,
}

impl Player {
    /// Create a new human player.
    pub fn new_human(id: u16, name: String, color: [f32; 3], config: &crate::game_config::GameConfig) -> Self {
        Self {
            id,
            alive: true,
            player_type: PlayerType::Human,
            name,
            color,
            troops: config.starting_troops,
            max_troops: config.max_troops_base,
            gold: config.starting_gold,
            has_spawned: false,
            sum_x: 0,
            sum_y: 0,
            tile_count: 0,
            border_tiles: Vec::new(),
            bot_rng: WyRand::seed_from_u64(id as u64),
            factories: 0,
            cities: 0,
        }
    }

    /// Create a new bot player.
    pub fn new_bot(id: u16, name: String, color: [f32; 3], config: &crate::game_config::GameConfig) -> Self {
        Self {
            id,
            alive: true,
            player_type: PlayerType::Bot,
            name,
            color,
            troops: config.starting_troops,
            max_troops: config.max_troops_base,
            gold: config.starting_gold,
            has_spawned: false,
            sum_x: 0,
            sum_y: 0,
            tile_count: 0,
            border_tiles: Vec::new(),
            bot_rng: WyRand::seed_from_u64(id as u64),
            factories: 0,
            cities: 0,
        }
    }

    /// Check if this player is a human.
    pub fn is_human(&self) -> bool {
        self.player_type == PlayerType::Human
    }

    /// Decode linear indices to (x, y) coordinates.
    pub fn border_coords(&self, map_width: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.border_tiles.iter().map(move |&idx| (idx % map_width, idx / map_width))
    }

    /// Absolute spatial order insert via `binary_search`.
    /// Guarantees strict determinism regardless of network arrival time,
    /// and replaces O(N) contains with O(log N) search.
    #[inline]
    pub fn border_insert(&mut self, idx: u32) {
        if let Err(pos) = self.border_tiles.binary_search(&idx) {
            self.border_tiles.insert(pos, idx);
        }
    }

    /// Absolute spatial order remove via `binary_search`.
    /// `swap_remove` is completely banned by the determinism contract.
    #[inline]
    pub fn border_remove(&mut self, idx: u32) {
        if let Ok(pos) = self.border_tiles.binary_search(&idx) {
            self.border_tiles.remove(pos);
        }
    }
}

/// Color palette for human players (very high chroma; bots stay softer via `bot_territory_color`).
pub fn player_colors() -> Vec<[f32; 3]> {
    vec![
        [0.0, 1.0, 1.0],    // Cyan
        [1.0, 0.02, 0.08],  // Red
        [0.02, 1.0, 0.08],  // Green
        [1.0, 1.0, 0.0],    // Yellow
        [1.0, 0.05, 0.95],  // Magenta
        [0.45, 0.08, 1.0],  // Violet
        [1.0, 0.32, 0.0],   // Orange
        [0.0, 0.98, 0.88],  // Teal
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

/// Deterministic softer territory tint for bots (still less saturated than `player_colors()`).
/// PRNG stream is independent of spawn-placement RNG so map layout stays unchanged.
pub fn bot_territory_color(game_seed: u64, bot_id: u16) -> [f32; 3] {
    let mix = game_seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (bot_id as u64).wrapping_shl(32)
        ^ (bot_id as u64);
    let mut rng = WyRand::seed_from_u64(mix);
    let h = rng.next_int(0, 10_000) as f32 / 10_000.0;
    let s = 0.28 + rng.next_int(0, 1000) as f32 / 1000.0 * 0.18;
    let v = 0.52 + rng.next_int(0, 1000) as f32 / 1000.0 * 0.18;
    let [r, g, b] = hsv_to_rgb(h, s, v);
    [
        r.clamp(0.16, 0.88),
        g.clamp(0.16, 0.88),
        b.clamp(0.16, 0.88),
    ]
}
