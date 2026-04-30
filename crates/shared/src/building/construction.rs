use bevy::prelude::*;
use crate::game::{GameEvent, GamePhase, GameState};
use super::core::Building;
/// Advance construction timers; emits [`GameEvent::StructureReady`].
/// Split out for unit tests (`advance_building_construction_tick`).
pub fn advance_building_construction_tick(
    game: &mut GameState,
) {
    if game.phase != GamePhase::Playing {
        return;
    }

    let mut any_ready = false;
    let mut ready_buildings = Vec::new();

    for b in game.buildings.iter_mut() {
        if b.under_construction {
            if b.ticks_until_complete > 0 {
                b.ticks_until_complete -= 1;
            }
            if b.ticks_until_complete == 0 {
                b.under_construction = false;
                game.events.push(GameEvent::StructureReady {
                    id: b.id,
                    tile_idx: b.tile_idx,
                    kind: b.kind,
                });
                any_ready = true;
                ready_buildings.push(*b);
            }
        }
    }

    if any_ready {
        let all_buildings: Vec<Building> = game.buildings.clone();
        let map_ref = &game.map;
        let world_seed = game.seed;
        for bcopy in ready_buildings {
            game.rail.on_structure_ready(map_ref, world_seed, &all_buildings, &bcopy);
        }
    }
}

/// Advance construction timers; emits [`GameEvent::StructureReady`].
pub fn system_building_construction(mut game: ResMut<GameState>) {
    advance_building_construction_tick(&mut *game);
}
