/// Game balance constants.

/// Map dimensions (in tiles).
pub const MAP_WIDTH: u32 = 200;
pub const MAP_HEIGHT: u32 = 200;

/// Number of deterministic bots to spawn.
pub const BOT_COUNT: u32 = 4;

/// Starting troops for a new human player.
pub const STARTING_TROOPS_HUMAN: f64 = 25_000.0;

/// Starting troops for a new bot player.
pub const STARTING_TROOPS_BOT: f64 = 10_000.0;

/// Radius of the initial spawn cluster (in tiles from center).
pub const SPAWN_RADIUS: u32 = 3;

/// Troop cost to expand into one neutral tile.
pub const EXPAND_COST: f64 = 16.0;

/// Troop cost to attack one enemy tile (attacker pays this).
pub const ATTACK_COST: f64 = 80.0;

/// How many neutral tiles to expand per server tick when expanding.
pub const EXPAND_TILES_PER_TICK: u32 = 3;

/// Fixed troop generation base
pub const TROOP_BASE_INCOME: f64 = 10.0;

/// How max troops scale with territory: max_troops = BASE + TILES^0.6 * SCALE.
/// These values represent the OpenFront formula: 2 * (50_000 + tiles^0.6 * 1000)
pub const MAX_TROOPS_BASE: f64 = 100_000.0;
pub const MAX_TROOPS_SCALE: f64 = 2_000.0;

/// Percentage of tiles needed to win.
pub const WIN_PERCENTAGE: f64 = 80.0;

// --- Visual (client tilemap; OpenFront-style parity) ---

/// Alpha for non-border owned tiles (1.0 = fully opaque). Borders always use 1.0.
pub const TERRITORY_INTERIOR_ALPHA: f32 = 15.0;

/// RGB multiplier for border tiles vs interior (same hue, darker edge). Lower = stronger outline.
pub const TERRITORY_BORDER_RGB_MULTIPLIER: f32 = 0.88;
