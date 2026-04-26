/// Game balance constants.

/// Starting troops for a new player.
pub const STARTING_TROOPS: f64 = 100.0;

/// OpenFront parity uses much larger gold magnitudes (125_000, 1_000_000, ...).
/// Keep those ratios by scaling into dark-rift's lighter economy.
pub const OPENFRONT_GOLD_SCALE: f64 = 1_000.0;

/// Starting gold for a new player (structures / upgrades consume gold).
pub const STARTING_GOLD: f64 = 250.0;

/// Gold granted per simulation tick (deterministic).
pub const GOLD_BASE_INCOME: f64 = 1.25;

/// Extra gold per tick per **ready** City level (sum of `Building.level` for cities not under construction).
pub const GOLD_INCOME_PER_CITY_LEVEL: f64 = 0.35;

/// Radius of the initial spawn cluster (in tiles from center).
pub const SPAWN_RADIUS: u32 = 3;

/// Troop cost to expand into one neutral tile.
pub const EXPAND_COST: f64 = 1.0;

/// Troop cost to attack one enemy tile (attacker pays this).
pub const ATTACK_COST: f64 = 5.0;

/// How many neutral tiles to expand per server tick when expanding.
pub const EXPAND_TILES_PER_TICK: u32 = 3;

/// Fixed troop generation base: you always gain at least this per tick.
pub const TROOP_BASE_INCOME: f64 = 2.0;

/// Troop generation per owned tile per tick.
pub const TROOP_PER_TILE: f64 = 0.05;

/// How max troops scale with territory: max_troops = BASE + TILES^0.6 * SCALE.
pub const MAX_TROOPS_BASE: f64 = 100.0;
pub const MAX_TROOPS_SCALE: f64 = 50.0;

/// Extra max troops per total **ready** City level (sum of `Building.level`).
pub const CITY_MAX_TROOPS_PER_LEVEL: f64 = 2_000.0;

/// Factory income multiplier: `1.0 + min(factory_levels * STEP, CAP - 1.0)`.
pub const FACTORY_INCOME_BONUS_PER_LEVEL: f64 = 0.10;
pub const FACTORY_INCOME_BONUS_CAP: f64 = 1.50;

/// Manhattan range from a tile for [`crate::building::defense_post_priority_bonus`].
pub const DEFENSE_POST_RANGE: i32 = 8;

/// Extra attack frontier priority per combined DefensePost level near the defender tile
/// (higher `PrioritizedTile::priority` ⇒ later conquest).
pub const DEFENSE_POST_PRIORITY_PER_LEVEL: i64 = 4;

/// SAM coverage radius in tiles (ghost preview; missile combat not implemented yet).
pub const SAM_RANGE_TILES: i32 = 12;

/// Feature gate for incomplete missile gameplay (Silo + SAM interception loop).
pub const ENABLE_MISSILE_STRUCTURES: bool = false;

/// Percentage of tiles needed to win.
pub const WIN_PERCENTAGE: f64 = 75.0;

// --- Visual (client tilemap; OpenFront-style parity) ---

/// Alpha for non-border owned tiles (1.0 = fully opaque). Borders always use 1.0.
pub const TERRITORY_INTERIOR_ALPHA: f32 = 15.0;

/// RGB multiplier for border tiles vs interior (same hue, darker edge). Lower = stronger outline.
pub const TERRITORY_BORDER_RGB_MULTIPLIER: f32 = 0.22;
