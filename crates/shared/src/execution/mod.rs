use std::cmp::Ordering;
use std::collections::BinaryHeap;
use bevy::prelude::*;
use crate::game::GameState;
use crate::map::TerrainType;
use bevy_prng::WyRand;
use crate::rng::NextIntExt;

pub mod income;
pub mod combat;

pub use income::*;
pub use combat::*;

/// Fraction of refunded troops lost when retreating from an attack on another player (OpenFront parity).
pub const RETREAT_PENALTY_VS_PLAYER: f64 = 0.25;

#[derive(Component)]
pub struct PlayerExecutionTag {
    pub player_id: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrioritizedTile {
    pub priority: i64, // Lower is processed first
    pub insert_seq: u32, // Deterministic BFS tie-breaker
    pub x: u32,
    pub y: u32,
}

impl Ord for PrioritizedTile {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse order so lowest priority behaves as max in BinaryHeap
        match other.priority.cmp(&self.priority) {
            Ordering::Equal => {
                // Return exact Insertion Time inverse!
                // Older items (smallest insert_seq) pop FIRST, guaranteeing deterministic BFS organicity!
                match other.insert_seq.cmp(&self.insert_seq) {
                    Ordering::Equal => {
                        // Flawless Secondary Tie-Breaker: Coordinate space is immutable and unique per tile
                        match other.y.cmp(&self.y) {
                            Ordering::Equal => other.x.cmp(&self.x),
                            ord => ord,
                        }
                    },
                    ord => ord,
                }
            },
            ord => ord,
        }
    }
}

impl PartialOrd for PrioritizedTile {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Component)]
pub struct AttackExecution {
    pub id: u64,
    pub owner_id: u16,
    pub target_owner: u16,
    pub troops: f64,
    pub initial_troops: f64,
    pub to_conquer: BinaryHeap<PrioritizedTile>,
    pub insert_seq_counter: u32,
    pub rng: WyRand,
    /// Player cancelled via HUD; next tick refunds remaining troops (with penalty vs players).
    pub retreating: bool,
}

impl AttackExecution {
    pub fn calc_priority(&mut self, num_owned_by_me: u32, terrain: TerrainType, tick: u64) -> i64 {
        let mag_x2 = match terrain {
            TerrainType::Land => 2,
            TerrainType::Highland => 3,
            TerrainType::Mountain => 4,
            TerrainType::Water | TerrainType::Lake => 3, // Won't happen normally, but just in case
        };
        let r = self.rng.next_int(0, 7) as i64;
        // Formula scaled by 4 to maintain OpenFront quartiles in integer space
        (r + 10) * (4 - (num_owned_by_me as i64 * 2) + mag_x2) + (tick as i64 * 4)
    }
}

#[inline]
pub fn fractional_extra_tiles_milli(max_tiles_f64: f64, roll_milli: u32) -> u32 {
    let frac = max_tiles_f64.fract().clamp(0.0, 0.999_999_999_999);
    let threshold_milli = (frac * 1000.0).floor() as u32; // 0..=999
    if threshold_milli > 0 && roll_milli < threshold_milli {
        1
    } else {
        0
    }
}

pub fn system_game_tick(game_opt: Option<ResMut<GameState>>) {
    let Some(mut game) = game_opt else { return };
    game.tick();
}

pub fn refund_fleet_troops_to_player(game: &mut GameState, owner_id: u16, troops: f64) {
    if let Some(p) = game.player_mut(owner_id) {
        p.troops = (p.troops + troops.max(0.0)).min(p.max_troops);
    }
}
