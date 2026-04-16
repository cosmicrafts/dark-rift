use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

// No more custom bevy event required
use crate::config;
use crate::map::GameMap;
use crate::player::{Player, PlayerId};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum GamePhase {
    Lobby,
    Playing,
    GameOver,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum GameEvent {
    TileExpanded { x: u32, y: u32, owner: u16 },
    TileCaptured { x: u32, y: u32, new_owner: u16 },
    PlayerEliminated { player_id: u16 },
    GameOver { winner_id: u16 },
}


#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Resource)]
pub struct GameState {
    pub seed: u64,
    pub phase: GamePhase,
    pub map: GameMap,
    pub players: Vec<Player>,
    pub tick: u64,
    pub winner: Option<u16>,
    pub events: Vec<GameEvent>,

}

impl GameState {
    pub fn new(seed: u64, width: u32, height: u32) -> Self {
        Self {
            seed,
            phase: GamePhase::Playing,
            map: GameMap::new(width, height),
            players: Vec::new(),
            tick: 0,
            winner: None,
            events: Vec::new(),
        }
    }

    /// Spawn a player at the given center with a cluster of tiles.
    pub fn spawn_player(&mut self, player: Player, cx: u32, cy: u32) {
        let pid = player.id;
        self.players.push(player);

        let r = config::SPAWN_RADIUS as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                // Circle shape
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let nx = cx as i32 + dx;
                let ny = cy as i32 + dy;
                if self.map.is_valid_coord(nx, ny) {
                    let (ux, uy) = (nx as u32, ny as u32);
                    if self.map.owner_id(ux, uy) == 0 {
                        self.map.set_owner_id(ux, uy, pid);
                    }
                }
            }
        }

        // Mark as spawned
        if let Some(p) = self.player_mut(pid) {
            p.has_spawned = true;
        }
    }

    pub fn tick(&mut self) {
        if self.phase != GamePhase::Playing {
            return;
        }

        self.tick += 1;
        // NOTE: self.events.clear() is intentional omitted here for batched visual processing.
        // It is safely cleared manually by visual consumers instead (e.g. handle_tile_change).
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }
}
