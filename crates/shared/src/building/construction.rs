use bevy::prelude::*;
use crate::game::{GameEvent, GamePhase, GameState};
use super::core::Building;
/// Advance construction timers; emits [`GameEvent::StructureReady`].
/// Split out for unit tests (`advance_building_construction_tick`).
pub fn advance_building_construction_tick(
    game: &mut GameState,
    q: &mut Query<(Entity, &mut Building)>,
) {
    if game.phase != GamePhase::Playing {
        return;
    }

    let all_buildings: Vec<Building> = q.iter().map(|(_, b)| *b).collect();

    let mut scratch: Vec<(Entity, u64)> = q.iter().map(|(e, b)| (e, b.id)).collect();
    scratch.sort_by_key(|&(_, id)| id);

    for (entity, _) in scratch {
        let Ok((_, mut b)) = q.get_mut(entity) else {
            continue;
        };
        if !b.under_construction {
            continue;
        }
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
            let bcopy = *b;
            let map_ref = &game.map;
            let world_seed = game.seed;
            game
                .rail
                .on_structure_ready(map_ref, world_seed, &all_buildings, &bcopy);
        }
    }
}

/// Advance construction timers; emits [`GameEvent::StructureReady`].
pub fn system_building_construction(mut game: ResMut<GameState>, mut q: Query<(Entity, &mut Building)>) {
    advance_building_construction_tick(&mut *game, &mut q);
}
