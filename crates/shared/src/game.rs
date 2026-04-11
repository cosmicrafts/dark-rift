//! Core game state machine and win/loss logic.
//!
//! Manages the lifecycle: Lobby → SpawnSelect → Playing → GameOver.
//! Tracks all players and checks win conditions each tick.

use rand::SeedableRng;
use serde::{Deserialize, Serialize};

use crate::attack::{self, ActionResult};
use crate::config;
use crate::faction::{self, Faction};
use crate::map::{StarMap, SystemId};
use crate::player::{Player, PlayerId};

/// The current phase of the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GamePhase {
    /// Waiting for players to connect.
    Lobby,
    /// Players are choosing spawn locations on the star map.
    SpawnSelect,
    /// The main game loop — expand, attack, conquer.
    Playing,
    /// A winner has been declared.
    GameOver,
}

/// The result of a win check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WinCheckResult {
    /// No winner yet.
    NoWinner,
    /// A player has won by domination (≥85% of systems).
    Domination { winner_id: PlayerId },
    /// A player has won by being the last one standing.
    LastStanding { winner_id: PlayerId },
}

/// Events emitted by the game state for the client/server to react to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GameEvent {
    PlayerEliminated { player_id: PlayerId },
    SystemCaptured { system_id: SystemId, new_owner: PlayerId },
    SystemExpanded { system_id: SystemId, owner: PlayerId },
    AttackFailed { system_id: SystemId, attacker_id: PlayerId },
    GameOver { winner_id: PlayerId },
}

/// The full game state — owned by the server, replicated pieces sent to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, bevy::prelude::Resource)]
pub struct GameState {
    pub phase: GamePhase,
    pub map: StarMap,
    pub players: Vec<Player>,
    pub factions: Vec<Faction>,
    pub tick: u32,
    pub events: Vec<GameEvent>,
    pub winner: Option<PlayerId>,
    pub seed: u64,
}

impl GameState {
    /// Create a new game with a generated map and AI factions.
    pub fn new(seed: u64) -> Self {
        let map = StarMap::generate(seed);
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed.wrapping_add(999));

        // Generate AI factions.
        // Faction IDs start at 1000 to leave room for human player IDs (0-999).
        let factions = faction::generate_factions(
            config::DEFAULT_FACTION_COUNT,
            1000,
            &mut rng,
        );

        let mut state = GameState {
            phase: GamePhase::Lobby,
            map,
            players: Vec::new(),
            factions: factions.clone(),
            tick: 0,
            events: Vec::new(),
            winner: None,
            seed,
        };

        // Create Player entries for each faction and assign starting systems.
        for faction in &factions {
            let player = Player::new_faction(
                faction.id,
                faction.name.clone(),
                faction.color,
            );
            state.players.push(player);
        }

        // Assign starting territory to factions.
        state.assign_faction_territory(&mut rng);

        state
    }

    /// Assign starting systems to AI factions — spread them across the map.
    fn assign_faction_territory(&mut self, _rng: &mut impl rand::Rng) {
        let faction_ids: Vec<u32> = self.factions.iter().map(|f| f.id).collect();

        for (i, &faction_id) in faction_ids.iter().enumerate() {
            // Pick a starting system well-distributed across the map.
            // Divide the map into sectors and place each faction in a different sector.
            let sector_width = self.map.width / (faction_ids.len() as f32).sqrt().ceil();
            let sector_height = self.map.height / (faction_ids.len() as f32).sqrt().ceil();
            let cols = (self.map.width / sector_width).ceil() as usize;
            let row = i / cols;
            let col = i % cols;

            let center_x = (col as f32 + 0.5) * sector_width;
            let center_y = (row as f32 + 0.5) * sector_height;

            // Find the nearest unclaimed system to this sector center.
            let mut best_system = None;
            let mut best_dist = f32::MAX;
            for sys in &self.map.systems {
                if sys.owner.is_some() {
                    continue;
                }
                let dx = sys.position.x - center_x;
                let dy = sys.position.y - center_y;
                let dist = dx * dx + dy * dy;
                if dist < best_dist {
                    best_dist = dist;
                    best_system = Some(sys.id);
                }
            }

            if let Some(start_id) = best_system {
                // Claim the starting system.
                let sys = self.map.system_mut(start_id);
                sys.owner = Some(faction_id);
                sys.ships = 20.0;

                // Also claim some adjacent systems.
                let connections = self.map.system(start_id).connections.clone();
                let mut claimed = 1u32;
                for &conn_id in &connections {
                    if claimed >= config::FACTION_STARTING_SYSTEMS {
                        break;
                    }
                    if self.map.system(conn_id).owner.is_none() {
                        let conn = self.map.system_mut(conn_id);
                        conn.owner = Some(faction_id);
                        conn.ships = 10.0;
                        claimed += 1;
                    }
                }
            }
        }
    }

    /// Add a human player to the game.
    pub fn add_human_player(&mut self, name: String) -> PlayerId {
        let id = self.players.len() as PlayerId;
        let colors = crate::player::human_player_colors();
        let color_idx = self.players.iter().filter(|p| p.is_human()).count();
        let color = colors[color_idx % colors.len()];

        let player = Player::new_human(id, name, color);
        self.players.push(player);
        id
    }

    /// Start the spawn selection phase.
    pub fn start_spawn_phase(&mut self) {
        self.phase = GamePhase::SpawnSelect;
    }

    /// Handle a player selecting their spawn system.
    pub fn player_spawn(&mut self, player_id: PlayerId, system_id: SystemId) -> bool {
        // Validate: system must be unclaimed and far from other human spawns.
        if self.map.system(system_id).owner.is_some() {
            return false;
        }

        // Check distance from other human spawn locations.
        for p in &self.players {
            if p.is_human() && p.has_spawned {
                // Find their systems and check distance.
                for sys in &self.map.systems {
                    if sys.owner == Some(p.id) {
                        let dist = sys.position.distance(
                            self.map.system(system_id).position,
                        );
                        if dist < config::MIN_SPAWN_DISTANCE {
                            return false;
                        }
                    }
                }
            }
        }

        // Claim the spawn system and adjacent unclaimed systems.
        let sys = self.map.system_mut(system_id);
        sys.owner = Some(player_id);
        sys.ships = 15.0;

        let connections = self.map.system(system_id).connections.clone();
        let mut claimed = 1u32;
        for &conn_id in &connections {
            if claimed >= config::PLAYER_STARTING_SYSTEMS {
                break;
            }
            if self.map.system(conn_id).owner.is_none() {
                let conn = self.map.system_mut(conn_id);
                conn.owner = Some(player_id);
                conn.ships = 5.0;
                claimed += 1;
            }
        }

        if let Some(player) = self.players.iter_mut().find(|p| p.id == player_id) {
            player.has_spawned = true;
            player.reserve_ships = 20.0;
        }

        true
    }

    /// Transition from SpawnSelect to Playing.
    pub fn start_playing(&mut self) {
        self.phase = GamePhase::Playing;
    }

    /// Execute one game tick — ship generation + win check.
    pub fn tick(&mut self) {
        if self.phase != GamePhase::Playing {
            return;
        }

        self.tick += 1;
        self.events.clear();

        // Ship generation: each owned system generates ships for its owner.
        let mut ship_income: std::collections::HashMap<PlayerId, f32> =
            std::collections::HashMap::new();
        for sys in &self.map.systems {
            if let Some(owner_id) = sys.owner {
                *ship_income.entry(owner_id).or_insert(0.0) +=
                    config::SHIPS_PER_SYSTEM_PER_TICK;
            }
        }
        for player in &mut self.players {
            if player.alive {
                if let Some(&income) = ship_income.get(&player.id) {
                    player.reserve_ships += income;
                }
            }
        }

        // Check for eliminated players (0 systems owned).
        let _total_systems = self.map.system_count();
        for player in &mut self.players {
            if player.alive && player.has_spawned {
                let owned = self.map.systems_owned_by(player.id);
                if owned == 0 {
                    player.alive = false;
                    self.events.push(GameEvent::PlayerEliminated {
                        player_id: player.id,
                    });
                }
            }
        }

        // Win condition check.
        if self.tick % config::WIN_CHECK_INTERVAL == 0 {
            match self.check_win() {
                WinCheckResult::NoWinner => {}
                WinCheckResult::Domination { winner_id }
                | WinCheckResult::LastStanding { winner_id } => {
                    self.phase = GamePhase::GameOver;
                    self.winner = Some(winner_id);
                    self.events.push(GameEvent::GameOver { winner_id });
                }
            }
        }
    }

    /// Handle a player action (expand or attack).
    pub fn player_action(
        &mut self,
        player_id: PlayerId,
        target_system_id: SystemId,
    ) -> ActionResult {
        if self.phase != GamePhase::Playing {
            return ActionResult::Invalid {
                reason: "Game is not in Playing phase.".to_string(),
            };
        }

        let player = match self.players.iter_mut().find(|p| p.id == player_id) {
            Some(p) if p.alive => p,
            _ => {
                return ActionResult::Invalid {
                    reason: "Player not found or not alive.".to_string(),
                };
            }
        };

        let mut reserve = player.reserve_ships;
        let result =
            attack::execute_action(&mut self.map, player_id, target_system_id, &mut reserve);
        player.reserve_ships = reserve;

        // Emit events based on result.
        match &result {
            ActionResult::Expanded { system_id } => {
                self.events.push(GameEvent::SystemExpanded {
                    system_id: *system_id,
                    owner: player_id,
                });
            }
            ActionResult::Conquered { system_id, .. } => {
                self.events.push(GameEvent::SystemCaptured {
                    system_id: *system_id,
                    new_owner: player_id,
                });
            }
            ActionResult::AttackFailed { system_id, .. } => {
                self.events.push(GameEvent::AttackFailed {
                    system_id: *system_id,
                    attacker_id: player_id,
                });
            }
            ActionResult::Invalid { .. } => {}
        }

        result
    }

    /// Check if anyone has won.
    fn check_win(&self) -> WinCheckResult {
        let total = self.map.system_count() as f32;

        // Check domination (≥85% of systems).
        for player in &self.players {
            if !player.alive {
                continue;
            }
            let owned = self.map.systems_owned_by(player.id) as f32;
            let pct = (owned / total) * 100.0;
            if pct >= config::WIN_PERCENTAGE {
                return WinCheckResult::Domination {
                    winner_id: player.id,
                };
            }
        }

        // Check last standing among human players.
        let alive_humans: Vec<_> = self
            .players
            .iter()
            .filter(|p| p.alive && p.is_human())
            .collect();
        if alive_humans.len() == 1 {
            return WinCheckResult::LastStanding {
                winner_id: alive_humans[0].id,
            };
        }

        WinCheckResult::NoWinner
    }

    /// Drain pending events (for sending to clients).
    pub fn drain_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events)
    }

    /// Get a player by ID.
    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    /// Get a mutable player by ID.
    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// Get the leaderboard sorted by systems owned (descending).
    pub fn leaderboard(&self) -> Vec<(PlayerId, String, u32)> {
        let mut board: Vec<_> = self
            .players
            .iter()
            .filter(|p| p.alive)
            .map(|p| {
                let owned = self.map.systems_owned_by(p.id);
                (p.id, p.name.clone(), owned)
            })
            .collect();
        board.sort_by(|a, b| b.2.cmp(&a.2));
        board
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_creation() {
        let game = GameState::new(42);
        assert_eq!(game.phase, GamePhase::Lobby);
        assert!(!game.factions.is_empty());
        assert!(game.map.system_count() > 50);

        // Factions should have been assigned some territory.
        let faction_systems: u32 = game
            .factions
            .iter()
            .map(|f| game.map.systems_owned_by(f.id))
            .sum();
        assert!(faction_systems > 0, "Factions should have territory");
    }

    #[test]
    fn test_add_human_player() {
        let mut game = GameState::new(42);
        let player_id = game.add_human_player("TestPlayer".to_string());
        assert_eq!(game.players.iter().filter(|p| p.is_human()).count(), 1);
        assert!(game.player(player_id).is_some());
    }

    #[test]
    fn test_spawn_phase() {
        let mut game = GameState::new(42);
        let player_id = game.add_human_player("TestPlayer".to_string());
        game.start_spawn_phase();

        // Find an unclaimed system to spawn on.
        let unclaimed = game.map.unclaimed_systems();
        assert!(!unclaimed.is_empty());

        let spawned = game.player_spawn(player_id, unclaimed[0]);
        assert!(spawned);
        assert!(game.player(player_id).unwrap().has_spawned);
        assert!(game.map.systems_owned_by(player_id) > 0);
    }

    #[test]
    fn test_ship_generation() {
        let mut game = GameState::new(42);
        let player_id = game.add_human_player("TestPlayer".to_string());
        game.start_spawn_phase();
        let unclaimed = game.map.unclaimed_systems();
        game.player_spawn(player_id, unclaimed[0]);
        game.start_playing();

        let ships_before = game.player(player_id).unwrap().reserve_ships;
        game.tick();
        let ships_after = game.player(player_id).unwrap().reserve_ships;
        assert!(ships_after > ships_before, "Ships should increase each tick");
    }
}
