//! Railway simulation (OpenFront parity): runs after building construction, before `game.tick()`.

use bevy::prelude::*;

use crate::game::GameState;
use crate::railway::{run_rail_simulation, RailPathScratch};

#[derive(Resource, Default)]
pub struct RailPathScratchResource(pub RailPathScratch);

pub fn system_rail_simulation(mut game: ResMut<GameState>, mut scratch: ResMut<RailPathScratchResource>) {
    let cloned_buildings = game.buildings.clone();
    run_rail_simulation(&mut game, &cloned_buildings, &mut scratch.0);
}
