use crate::game::GameState;
use crate::execution::AttackExecution;
use crate::warp_fleet::WarpFleet;
use crate::building::Building;
use crate::pathfinding::WaterPathfinderScratch;
use crate::water_components::WaterComponents;

use bevy::prelude::Resource;

#[derive(Clone, Resource)]
pub struct DarkRiftEngine {
    pub state: GameState,
    pub attacks: Vec<AttackExecution>,
    pub fleets: Vec<WarpFleet>,
    pub buildings: Vec<Building>,
    pub water: WaterComponents,
    pub path_scratch: WaterPathfinderScratch,
    pub defense_grid: crate::building::DefenseGrid,
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
            defense_grid: crate::building::DefenseGrid::default(),
        }
    }
}
