use crate::game::GameState;
use crate::execution::AttackExecution;
use crate::warp_fleet::WarpFleet;
use crate::building::{Building, BuildingGrid};
use crate::pathfinding::WaterPathfinderScratch;
use crate::water_components::WaterComponents;

use bevy::prelude::Resource;

#[derive(Default, Clone)]
pub struct PlacementScratch {
    pub visited_stamp: Vec<u32>,
    pub stamp: u32,
    pub queue: Vec<u32>,
}

#[derive(Clone, Resource)]
pub struct DarkRiftEngine {
    pub state: GameState,
    pub attacks: Vec<AttackExecution>,
    pub fleets: Vec<WarpFleet>,
    pub buildings: Vec<Building>,
    pub water: WaterComponents,
    pub path_scratch: WaterPathfinderScratch,
    pub placement_scratch: PlacementScratch,
    pub defense_grid: crate::building::DefenseGrid,
    pub building_grid: BuildingGrid,
}

impl DarkRiftEngine {
    pub fn new(state: GameState, water: WaterComponents) -> Self {
        Self {
            state,
            attacks: Vec::new(),
            fleets: Vec::new(),
            buildings: Vec::new(),
            water,
            path_scratch: WaterPathfinderScratch::default(),
            placement_scratch: PlacementScratch::default(),
            defense_grid: crate::building::DefenseGrid::default(),
            building_grid: BuildingGrid::default(),
        }
    }

    /// Rebuilds [`BuildingGrid`] when dirty or first use (`grid_w == 0`).
    pub fn refresh_building_grid(&mut self) {
        if !self.building_grid.dirty && self.building_grid.grid_w > 0 {
            return;
        }
        let w = self.state.map.width;
        let h = self.state.map.height;
        self.building_grid.rebuild(self.buildings.iter(), w, h);
    }

    #[inline]
    pub fn add_building(&mut self, b: Building) {
        let pos = self.buildings.partition_point(|x| x.id < b.id);
        self.buildings.insert(pos, b);
        self.building_grid.mark_dirty();
    }

    #[inline]
    pub fn add_attack(&mut self, a: AttackExecution) {
        let pos = self.attacks.partition_point(|x| x.id < a.id);
        self.attacks.insert(pos, a);
    }

    #[inline]
    pub fn add_fleet(&mut self, f: WarpFleet) {
        let pos = self.fleets.partition_point(|x| x.id < f.id);
        self.fleets.insert(pos, f);
    }
}
