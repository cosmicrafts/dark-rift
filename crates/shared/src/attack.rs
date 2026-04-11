//! Attack and expansion mechanics.
//!
//! Simplified from OpenFront's AttackExecution.ts:
//! - Expand: click unclaimed + adjacent = instant claim, costs ships.
//! - Attack: click enemy + adjacent = deterministic combat resolution.

use crate::config;
use crate::map::StarMap;
use crate::player::PlayerId;

/// Result of attempting to expand or attack.
#[derive(Debug, Clone)]
pub enum ActionResult {
    /// Successfully claimed an unclaimed system.
    Expanded {
        system_id: u32,
    },
    /// Successfully conquered an enemy system through combat.
    Conquered {
        system_id: u32,
        attacker_ships_lost: f32,
        defender_ships_lost: f32,
    },
    /// Attack failed — defender held the system.
    AttackFailed {
        system_id: u32,
        attacker_ships_lost: f32,
        defender_ships_lost: f32,
    },
    /// Invalid action — can't reach this system or not enough ships.
    Invalid {
        reason: String,
    },
}

/// Attempt to expand into an unclaimed system or attack an enemy system.
///
/// This is the core player action — equivalent to OpenFront's single-click
/// attack mechanic but operating on graph nodes instead of pixel tiles.
pub fn execute_action(
    map: &mut StarMap,
    actor_id: PlayerId,
    target_system_id: u32,
    actor_reserve_ships: &mut f32,
) -> ActionResult {
    let target = &map.systems[target_system_id as usize];

    // Check: does the player have an adjacent owned system?
    if !map.player_has_adjacent(target_system_id, actor_id) {
        return ActionResult::Invalid {
            reason: "No adjacent owned system — can't reach target.".to_string(),
        };
    }

    match target.owner {
        // === EXPAND into unclaimed system ===
        None => {
            if *actor_reserve_ships < config::EXPAND_COST {
                return ActionResult::Invalid {
                    reason: format!(
                        "Not enough ships to expand. Need {}, have {:.0}.",
                        config::EXPAND_COST, actor_reserve_ships
                    ),
                };
            }

            // Pay the cost and claim the system.
            *actor_reserve_ships -= config::EXPAND_COST;
            let sys = map.system_mut(target_system_id);
            sys.owner = Some(actor_id);
            sys.ships = config::EXPAND_COST * 0.5; // Station half the cost as garrison.

            ActionResult::Expanded {
                system_id: target_system_id,
            }
        }

        // === ATTACK enemy system ===
        Some(defender_id) if defender_id != actor_id => {
            if *actor_reserve_ships < config::MIN_ATTACK_SHIPS {
                return ActionResult::Invalid {
                    reason: format!(
                        "Not enough ships to attack. Need {}, have {:.0}.",
                        config::MIN_ATTACK_SHIPS, actor_reserve_ships
                    ),
                };
            }

            // Commit all reserve ships to the attack.
            let attacker_ships = *actor_reserve_ships;
            let defender_ships = map.system(target_system_id).ships;

            // Deterministic combat resolution.
            let attacker_losses = defender_ships * config::ATTACKER_LOSS_RATIO;
            let defender_losses = attacker_ships * config::DEFENDER_LOSS_RATIO;

            let attacker_remaining = (attacker_ships - attacker_losses).max(0.0);
            let defender_remaining = (defender_ships - defender_losses).max(0.0);

            if attacker_remaining > defender_remaining {
                // Attacker wins — conquer the system.
                *actor_reserve_ships = 0.0;
                let sys = map.system_mut(target_system_id);
                sys.owner = Some(actor_id);
                sys.ships = attacker_remaining - defender_remaining;

                ActionResult::Conquered {
                    system_id: target_system_id,
                    attacker_ships_lost: attacker_losses,
                    defender_ships_lost: defender_ships,
                }
            } else {
                // Defender wins — attacker loses ships but system owner stays.
                *actor_reserve_ships = 0.0;
                let sys = map.system_mut(target_system_id);
                sys.ships = defender_remaining - attacker_remaining;

                ActionResult::AttackFailed {
                    system_id: target_system_id,
                    attacker_ships_lost: attacker_ships,
                    defender_ships_lost: defender_losses,
                }
            }
        }

        // === Clicked own system — no-op ===
        Some(_) => ActionResult::Invalid {
            reason: "Can't attack your own system.".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::StarMap;

    fn setup_map() -> StarMap {
        let mut map = StarMap::generate(42);
        // Give player 0 the first system.
        map.systems[0].owner = Some(0);
        map.systems[0].ships = 10.0;
        map
    }

    #[test]
    fn test_expand_unclaimed() {
        let mut map = setup_map();
        // Find an unclaimed neighbor of system 0.
        let neighbor = map.systems[0].connections[0];
        assert!(map.systems[neighbor as usize].owner.is_none());

        let mut ships = 100.0;
        let result = execute_action(&mut map, 0, neighbor, &mut ships);

        match result {
            ActionResult::Expanded { system_id } => {
                assert_eq!(system_id, neighbor);
                assert_eq!(map.systems[neighbor as usize].owner, Some(0));
                assert!(ships < 100.0); // Ships were spent.
            }
            other => panic!("Expected Expanded, got {other:?}"),
        }
    }

    #[test]
    fn test_expand_no_ships() {
        let mut map = setup_map();
        let neighbor = map.systems[0].connections[0];
        let mut ships = 1.0; // Not enough.

        let result = execute_action(&mut map, 0, neighbor, &mut ships);
        assert!(matches!(result, ActionResult::Invalid { .. }));
    }

    #[test]
    fn test_attack_enemy_wins() {
        let mut map = setup_map();
        let neighbor = map.systems[0].connections[0];
        // Give the enemy a weak garrison.
        map.systems[neighbor as usize].owner = Some(1);
        map.systems[neighbor as usize].ships = 5.0;

        let mut ships = 100.0; // Overwhelming force.
        let result = execute_action(&mut map, 0, neighbor, &mut ships);

        match result {
            ActionResult::Conquered { system_id, .. } => {
                assert_eq!(system_id, neighbor);
                assert_eq!(map.systems[neighbor as usize].owner, Some(0));
            }
            other => panic!("Expected Conquered, got {other:?}"),
        }
    }

    #[test]
    fn test_attack_defender_wins() {
        let mut map = setup_map();
        let neighbor = map.systems[0].connections[0];
        // Give the enemy an overwhelming garrison.
        map.systems[neighbor as usize].owner = Some(1);
        map.systems[neighbor as usize].ships = 500.0;

        let mut ships = 15.0; // Weak attack.
        let result = execute_action(&mut map, 0, neighbor, &mut ships);

        match result {
            ActionResult::AttackFailed { system_id, .. } => {
                assert_eq!(system_id, neighbor);
                assert_eq!(map.systems[neighbor as usize].owner, Some(1)); // Defender keeps it.
            }
            other => panic!("Expected AttackFailed, got {other:?}"),
        }
    }

    #[test]
    fn test_no_adjacent_system() {
        let mut map = setup_map();
        // Try to attack a system far from player 0's territory.
        let far_system = (map.systems.len() - 1) as u32;
        // Make sure it's not connected to system 0.
        if !map.are_connected(0, far_system) {
            let mut ships = 100.0;
            let result = execute_action(&mut map, 0, far_system, &mut ships);
            assert!(matches!(result, ActionResult::Invalid { .. }));
        }
    }
}
