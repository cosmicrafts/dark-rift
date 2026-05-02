use crate::game::{GameEvent, GamePhase};
use crate::engine::DarkRiftEngine;
/// Advance construction timers; emits [`GameEvent::StructureReady`].
/// Split out for unit tests (`advance_building_construction_tick`).
impl DarkRiftEngine {
    pub fn execute_construction(&mut self) {
        if self.state.phase != GamePhase::Playing {
            return;
        }

        self.buildings.sort_by_key(|b| b.id);

        for b in &mut self.buildings {
            if !b.under_construction {
                continue;
            }
            if b.ticks_until_complete > 0 {
                b.ticks_until_complete -= 1;
            }
            if b.ticks_until_complete == 0 {
                b.under_construction = false;
                self.state.events.push(GameEvent::StructureReady {
                    id: b.id,
                    tile_idx: b.tile_idx,
                    kind: b.kind,
                });
            }
        }
    }
}
