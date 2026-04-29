//! Railway simulation (OpenFront parity): runs after building construction, before `game.tick()`.

use bevy::prelude::*;

use crate::building::core::Building;
use crate::game::GameState;
use crate::railway::{run_rail_simulation, RailPathScratch};

#[derive(Resource, Default)]
pub struct RailPathScratchResource(pub RailPathScratch);

pub fn system_rail_simulation(mut game: ResMut<GameState>, mut scratch: ResMut<RailPathScratchResource>, q_building: Query<&Building>) {
    let mut buildings: Vec<Building> = q_building.iter().copied().collect();
    buildings.sort_by_key(|b| b.id);
    run_rail_simulation(&mut game, &buildings, &mut scratch.0);
}
