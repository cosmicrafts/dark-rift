/// Game balance constants for Dark Rift.

/// Number of star systems to generate on the map.
pub const DEFAULT_SYSTEM_COUNT: u32 = 300;

/// Map dimensions (world units).
pub const MAP_WIDTH: f32 = 1600.0;
pub const MAP_HEIGHT: f32 = 1000.0;

/// Minimum distance between any two star systems (Poisson disk radius).
pub const MIN_SYSTEM_DISTANCE: f32 = 40.0;

/// Maximum edge length for connections between star systems.
/// Edges longer than this are pruned from the Delaunay triangulation.
pub const MAX_CONNECTION_DISTANCE: f32 = 120.0;

/// Number of AI factions to seed on the map.
pub const DEFAULT_FACTION_COUNT: u32 = 8;

/// Number of systems each faction starts with.
pub const FACTION_STARTING_SYSTEMS: u32 = 5;

/// Number of systems a human player starts with (the picked one + neighbors).
pub const PLAYER_STARTING_SYSTEMS: u32 = 3;

/// Base ship generation rate per owned system per tick.
pub const SHIPS_PER_SYSTEM_PER_TICK: f32 = 0.05;

/// Cost in ships to expand into an unclaimed system.
pub const EXPAND_COST: f32 = 5.0;

/// Minimum ships required to launch an attack.
pub const MIN_ATTACK_SHIPS: f32 = 10.0;

/// Attacker troop loss multiplier when attacking an enemy system.
/// attacker_losses = defender_ships * ATTACKER_LOSS_RATIO
pub const ATTACKER_LOSS_RATIO: f32 = 0.5;

/// Defender troop loss multiplier when being attacked.
/// defender_losses = attacker_ships * DEFENDER_LOSS_RATIO
pub const DEFENDER_LOSS_RATIO: f32 = 0.8;

/// Percentage of total systems a player must own to trigger a Domination win.
pub const WIN_PERCENTAGE: f32 = 85.0;

/// How often (in ticks) to check win conditions.
pub const WIN_CHECK_INTERVAL: u32 = 60;

/// Minimum distance (in world units) between player spawn locations.
pub const MIN_SPAWN_DISTANCE: f32 = 200.0;

/// Maximum number of players per game.
pub const MAX_PLAYERS: u32 = 8;

/// Duration of the spawn selection phase in seconds.
pub const SPAWN_PHASE_DURATION_SECS: f32 = 30.0;

/// Game tick rate (ticks per second for the fixed timestep).
pub const TICK_RATE: f64 = 10.0;
