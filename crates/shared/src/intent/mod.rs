pub mod bot;
pub mod combat;
pub mod buildings;
pub mod fleet;

pub use bot::*;
pub use combat::*;


use bevy::prelude::*;

use crate::building::Building;
use crate::execution::AttackExecution;
use crate::game::GameState;
use crate::pathfinding::WaterPathfinderScratch;
use crate::protocol::{GameplayIntent, StampedIntent};
use crate::warp_fleet::WarpFleet;
use crate::water_components::WaterComponents;


/// Merge two frontiers: same cell keeps the tile with lower `priority` (better for expansion order).
///
/// **Determinism**: We do NOT use a `HashMap` for dedup because `HashMap` iteration
/// order is randomised by SipHash.  When two tiles at the same `(x,y)` share the
/// exact same `priority`, the `HashMap` would non-deterministically choose which
/// `insert_seq` to keep, silently diverging the `BinaryHeap` across clients.
/// Instead we sort deterministically and `dedup_by_key` — the first entry per key
/// (lowest priority, then lowest insert_seq, then spatial) survives.
pub fn apply_stamped_intent(
    commands: &mut Commands,
    game: &mut GameState,
    q_exec: &mut Query<(Entity, &mut AttackExecution)>,
    q_fleet: &mut Query<(Entity, &mut WarpFleet)>,
    q_building: &mut Query<(Entity, &mut Building)>,
    stamped: &StampedIntent,
    intent_index: u32,
    path_scratch: &mut WaterPathfinderScratch,
    water_components: &WaterComponents,
) {
    match &stamped.intent {
        GameplayIntent::RecallFleet { fleet_id } => {
            let pid = stamped.player_id;
            let mut pairs: Vec<(Entity, u64)> = q_fleet.iter().map(|(e, w)| (e, w.id)).collect();
            pairs.sort_by_key(|&(_, id)| id);
            for (entity, fid) in pairs {
                if fid != *fleet_id {
                    continue;
                }
                let Ok((_, mut wf)) = q_fleet.get_mut(entity) else {
                    continue;
                };
                if wf.owner_id != pid {
                    continue;
                }
                wf.retreating = true;
                wf.retreat_dst = None;
                wf.path.clear();
                wf.path_cursor = 0;
                break;
            }
        }
 GameplayIntent::LaunchFleet {
 target_tile,
 troops,
 } => {
 let mut buildings: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();
 buildings.sort_by_key(|b| b.id);
 fleet::apply_launch_fleet_intent(
                commands,
                game,
                path_scratch,
                water_components,
                q_fleet,
                stamped.player_id,
                *target_tile,
                *troops,
                &buildings,
            );
        }
        GameplayIntent::CancelAttack { attack_id } => {
            let pid = stamped.player_id;
            let mut candidates: Vec<(u64, u64, Entity)> = q_exec
                .iter()
                .filter(|(_, ex)| ex.id == *attack_id && ex.owner_id == pid)
                .map(|(e, ex)| (ex.id, e.to_bits(), e))
                .collect();
            candidates.sort_unstable_by_key(|(id, bits, _)| (*id, *bits));
            for (_, _, e) in candidates {
                if let Ok((_, mut ex)) = q_exec.get_mut(e) {
                    ex.retreating = true;
                    return;
                }
            }
            bevy::log::warn!(
                "apply_stamped_intent: cancel attack_id={} for player {} — not found or not owner",
                attack_id, pid
            );
        }
        GameplayIntent::Attack(attack) => {
            combat::apply_attack_intent(commands, game, q_exec, stamped.player_id, attack, intent_index);
        }
        GameplayIntent::BuildStructure { kind, target_tile } => {
            buildings::apply_build_structure_intent(
                commands,
                game,
                q_building,
                stamped.player_id,
                *kind,
                *target_tile,
            );
        }
        GameplayIntent::UpgradeStructure { building_id } => {
            buildings::apply_upgrade_structure_intent(game, q_building, stamped.player_id, *building_id);
        }
    }
}

