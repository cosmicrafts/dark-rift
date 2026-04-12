use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Resource)]
pub struct GameMap {
    pub width: u32,
    pub height: u32,
    pub terrain: Vec<u8>,
    pub state: Vec<u16>,
}

impl GameMap {
    pub const PLAYER_ID_MASK: u16 = 0x0FFF;

    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height) as usize;
        Self {
            width,
            height,
            terrain: vec![0; size],
            state: vec![0; size],
        }
    }

    pub fn ref_id(&self, x: u32, y: u32) -> usize {
        (y * self.width + x) as usize
    }

    pub fn is_valid_coord(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32
    }

    pub fn owner_id(&self, x: u32, y: u32) -> u16 {
        let r = self.ref_id(x, y);
        self.state[r] & Self::PLAYER_ID_MASK
    }

    pub fn set_owner_id(&mut self, x: u32, y: u32, player_id: u16) {
        let r = self.ref_id(x, y);
        self.state[r] = (self.state[r] & !Self::PLAYER_ID_MASK) | (player_id & Self::PLAYER_ID_MASK);
    }

    /// Get 4-connected neighbors (up, down, left, right) of a tile.
    pub fn neighbors(&self, x: u32, y: u32) -> Vec<(u32, u32)> {
        let mut result = Vec::with_capacity(4);
        if x > 0 { result.push((x - 1, y)); }
        if x + 1 < self.width { result.push((x + 1, y)); }
        if y > 0 { result.push((x, y - 1)); }
        if y + 1 < self.height { result.push((x, y + 1)); }
        result
    }

    /// Check if a tile is on the border of a player's territory
    /// (i.e. at least one neighbor is owned by someone else or unowned).
    pub fn is_border_tile(&self, x: u32, y: u32, player_id: u16) -> bool {
        if self.owner_id(x, y) != player_id {
            return false;
        }
        for (nx, ny) in self.neighbors(x, y) {
            if self.owner_id(nx, ny) != player_id {
                return true;
            }
        }
        false
    }

    /// Count tiles owned by a specific player.
    pub fn tiles_owned_by(&self, player_id: u16) -> u32 {
        self.state.iter()
            .filter(|&&s| (s & Self::PLAYER_ID_MASK) == player_id)
            .count() as u32
    }

    /// Check if a tile is adjacent to territory owned by `player_id`.
    pub fn is_adjacent_to_player(&self, x: u32, y: u32, player_id: u16) -> bool {
        for (nx, ny) in self.neighbors(x, y) {
            if self.owner_id(nx, ny) == player_id {
                return true;
            }
        }
        false
    }
}
