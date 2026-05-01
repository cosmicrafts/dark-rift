pub mod bot;
pub mod combat;
pub mod buildings;
pub mod fleet;

pub use bot::*;
pub use combat::*;


use crate::engine::DarkRiftEngine;
use crate::protocol::{GameplayIntent, StampedIntent};


/// Merge two frontiers: same cell keeps the tile with lower `priority` (better for expansion order).
///
/// **Determinism**: We do NOT use a `HashMap` for dedup because `HashMap` iteration
/// order is randomised by SipHash.  When two tiles at the same `(x,y)` share the
/// exact same `priority`, the `HashMap` would non-deterministically choose which
/// `insert_seq` to keep, silently diverging the `BinaryHeap` across clients.
/// Instead we sort deterministically and `dedup_by_key` — the first entry per key
/// (lowest priority, then lowest insert_seq, then spatial) survives.
impl DarkRiftEngine {
pub fn apply_intents(&mut self, intents: &[StampedIntent]) {
    for (i, stamped) in intents.iter().enumerate() {
        self.apply_stamped_intent(stamped, i as u32);
    }
}

pub fn apply_stamped_intent(
    &mut self,
    stamped: &StampedIntent,
    intent_index: u32,
) {
    match &stamped.intent {
        GameplayIntent::RecallFleet { fleet_id } => {
            let pid = stamped.player_id;
            for wf in &mut self.fleets {
                if wf.id != *fleet_id {
                    continue;
                }
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
 self.apply_launch_fleet_intent(
                stamped.player_id,
                *target_tile,
                *troops,
            );
        }
        GameplayIntent::CancelAttack { attack_id } => {
            let pid = stamped.player_id;
            for ex in &mut self.attacks {
                if ex.id == *attack_id && ex.owner_id == pid {
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
            self.apply_attack_intent(stamped.player_id, attack, intent_index);
        }
        GameplayIntent::BuildStructure { kind, target_tile } => {
            self.apply_build_structure_intent(
                stamped.player_id,
                *kind,
                *target_tile,
            );
        }
        GameplayIntent::UpgradeStructure { building_id } => {
            self.apply_upgrade_structure_intent(stamped.player_id, *building_id);
        }
    }
}
}

