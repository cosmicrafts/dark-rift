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

/// OpenFront structure types (mirrors `Structures` in OpenFront `Game.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BuildingKind {
    City,
    Factory,
    Port,
    DefensePost,
    SamLauncher,
    MissileSilo,
}

impl BuildingKind {
    pub const ALL: [BuildingKind; 6] = [
        BuildingKind::City,
        BuildingKind::Factory,
        BuildingKind::Port,
        BuildingKind::DefensePost,
        BuildingKind::SamLauncher,
        BuildingKind::MissileSilo,
    ];

    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            BuildingKind::City => "City",
            BuildingKind::Factory => "Factory",
            BuildingKind::Port => "Port",
            BuildingKind::DefensePost => "DefensePost",
            BuildingKind::SamLauncher => "SAM",
            BuildingKind::MissileSilo => "Silo",
        }
    }

    /// OpenFront `DefaultConfig.unitInfo(...).upgradable`.
    pub fn upgradable(self) -> bool {
        match self {
            BuildingKind::DefensePost => false,
            _ => true,
        }
    }

    /// Construction duration in simulation ticks (one per `SimulationSchedule` chain).
    pub fn construction_duration_ticks(self) -> u32 {
        match self {
            BuildingKind::City | BuildingKind::Factory | BuildingKind::Port => 20,
            BuildingKind::DefensePost => 50,
            BuildingKind::SamLauncher => 300,
            BuildingKind::MissileSilo => 100,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum GameEvent {
    TileExpanded { x: u32, y: u32, owner: u16 },
    TileCaptured { x: u32, y: u32, new_owner: u16 },
    PlayerEliminated { player_id: u16 },
    GameOver { winner_id: u16 },
    /// A new structure entity was spawned (construction may still be running).
    StructureSpawned {
        id: u64,
        owner_id: u16,
        tile_idx: u32,
        kind: BuildingKind,
        level: u8,
    },
    /// Construction finished; structure is active for gameplay effects (MVP: event only).
    StructureReady {
        id: u64,
        tile_idx: u32,
        kind: BuildingKind,
    },
    StructureUpgraded {
        id: u64,
        tile_idx: u32,
        kind: BuildingKind,
        level: u8,
    },
}


#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Resource)]
pub struct GameState {
    pub seed: u64,
    pub phase: GamePhase,
    pub map: GameMap,
    pub players: Vec<Player>,
    #[serde(skip)]
    pub player_lookup: Vec<Option<usize>>,
    pub tick: u64,
    pub winner: Option<u16>,
    pub events: Vec<GameEvent>,
    /// Monotonic id for `WarpFleet` (deterministic ordering across clients).
    #[serde(default = "default_next_fleet_id")]
    pub next_fleet_id: u64,
    /// Monotonic id for structure/building entities.
    #[serde(default = "default_next_building_id")]
    pub next_building_id: u64,
    /// Monotonic id for `AttackExecution` entities (ground + fleet merge roots).
    #[serde(default = "default_next_attack_id")]
    pub next_attack_id: u64,
}

fn default_next_fleet_id() -> u64 {
    1
}

fn default_next_building_id() -> u64 {
    1
}

fn default_next_attack_id() -> u64 {
    1
}

impl GameState {
    pub fn new(seed: u64, width: u32, height: u32) -> Self {
        Self {
            seed,
            phase: GamePhase::Playing,
            map: GameMap::new(width, height),
            players: Vec::new(),
            player_lookup: Vec::new(),
            tick: 0,
            winner: None,
            events: Vec::new(),
            next_fleet_id: 1,
            next_building_id: 1,
            next_attack_id: 1,
        }
    }

    /// Spawn a player at the given center with a cluster of tiles.
    pub fn spawn_player(&mut self, player: Player, cx: u32, cy: u32) {
        let pid = player.id;
        let index = self.players.len();
        self.players.push(player);
        
        let pid_usize = pid as usize;
        if pid_usize >= self.player_lookup.len() {
            self.player_lookup.resize(pid_usize + 1, None);
        }
        self.player_lookup[pid_usize] = Some(index);

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
                        self.set_tile_owner(ux, uy, pid);
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
        let id_usize = id as usize;
        if id_usize < self.player_lookup.len() {
            if let Some(idx) = self.player_lookup[id_usize] {
                return self.players.get(idx);
            }
        }
        None
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        let id_usize = id as usize;
        if id_usize < self.player_lookup.len() {
            if let Some(idx) = self.player_lookup[id_usize] {
                return self.players.get_mut(idx);
            }
        }
        None
    }

    pub fn set_tile_owner(&mut self, x: u32, y: u32, new_owner: u16) {
        let old_owner = self.map.owner_id(x, y);
        if old_owner == new_owner {
            return;
        }

        let linear_idx = (y * self.map.width + x) as u32;

        // Deduct from old owner
        if old_owner != 0 {
            if let Some(p) = self.player_mut(old_owner) {
                p.sum_x = p.sum_x.saturating_sub(x as u64);
                p.sum_y = p.sum_y.saturating_sub(y as u64);
                p.tile_count = p.tile_count.saturating_sub(1);
                p.border_remove(linear_idx);
            }
        }

        // Add to new owner
        if new_owner != 0 {
            if let Some(p) = self.player_mut(new_owner) {
                p.sum_x += x as u64;
                p.sum_y += y as u64;
                p.tile_count += 1;
            }
        }

        self.map.set_owner_id(x, y, new_owner);

        // Update T in new_owner's borders
        if new_owner != 0 {
            let is_border = self.map.is_border_tile(x, y, new_owner);
            if is_border {
                if let Some(p) = self.player_mut(new_owner) {
                    p.border_insert(linear_idx);
                }
            }
        }

        // Update neighbors
        let neighbors = self.map.neighbors(x, y);
        for (nx, ny) in neighbors {
            let n_owner = self.map.owner_id(nx, ny);
            let n_idx = (ny * self.map.width + nx) as u32;

            if n_owner == old_owner {
                if old_owner != 0 {
                    if let Some(p) = self.player_mut(old_owner) {
                        p.border_insert(n_idx);
                    }
                }
            } else if n_owner == new_owner {
                if new_owner != 0 {
                    let is_border = self.map.is_border_tile(nx, ny, new_owner);
                    if !is_border {
                        if let Some(p) = self.player_mut(new_owner) {
                            p.border_remove(n_idx);
                        }
                    }
                }
            }
        }
    }

}
