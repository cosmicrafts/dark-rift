pub mod core;
pub mod cost;
pub mod placement;
pub mod upgrade;
pub mod construction;
pub mod hud;

pub use core::*;
pub use cost::*;
pub use placement::*;
pub use upgrade::*;
pub use construction::*;
pub use hud::*;

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::{Entity, Query, ResMut, World};
    use crate::game::BuildingKind;
    use crate::map::{GameMap, MapTile};

    fn tiny_owned_map() -> (GameMap, u16) {
        let mut m = GameMap::new(5, 1);
        let owner = 1u16;
        for x in 0..5 {
            m.set_owner_id(x, 0, owner);
            let ri = m.ref_id(x, 0);
            m.terrain[ri] = MapTile::from_byte(0b1000_0000); // land
        }
        // Shore at ends for port tests
        let r0 = m.ref_id(0, 0);
        m.terrain[r0] = MapTile::from_byte(0b1100_0000);
        let r4 = m.ref_id(4, 0);
        m.terrain[r4] = MapTile::from_byte(0b1100_0000);
        (m, owner)
    }

    #[test]
    fn valid_land_center_click() {
        let (map, owner) = tiny_owned_map();
        let existing: Vec<(u32, u32)> = Vec::new();
        let v = valid_land_structure_indices(&map, owner, 2, &existing);
        assert!(v.contains(&2));
    }

    #[test]
    fn spacing_excludes_nearby() {
        let w = 32u32;
        let mut m = GameMap::new(w, 1);
        let owner = 1u16;
        for x in 0..w {
            m.set_owner_id(x, 0, owner);
            let ri = m.ref_id(x, 0);
            m.terrain[ri] = MapTile::from_byte(0b1000_0000);
        }
        let click = 16u32;
        // Structure one tile away from click: click tile is excluded (too close).
        let existing = vec![(15u32, 0u32)];
        let v = valid_land_structure_indices(&m, owner, click, &existing);
        assert!(!v.contains(&15));
        assert!(!v.is_empty());
    }

    #[test]
    fn port_picks_shore_in_valid_set() {
        let (map, owner) = tiny_owned_map();
        let valid = valid_land_structure_indices(&map, owner, 2, &[]);
        let p = resolve_port_spawn_tile(&map, owner, 2, &valid);
        assert!(p.is_some());
        let (x, y) = idx_xy(p.unwrap(), map.width);
        assert!(is_shore_land_tile(&map, x, y));
    }

    #[test]
    fn upgrade_closest_by_manhattan() {
        let w = 20u32;
        let mut b1 = Building {
            id: 1,
            owner_id: 1,
            tile_idx: xy_idx(5, 5, w),
            kind: BuildingKind::City,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let b2 = Building {
            id: 2,
            owner_id: 1,
            tile_idx: xy_idx(6, 5, w),
            kind: BuildingKind::City,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let map = GameMap::new(w, 20);
        let click = xy_idx(5, 5, w);
        let id = find_upgrade_target_id(&map, 1, BuildingKind::City, click, &[b1, b2]);
        assert_eq!(id, Some(1));

        b1.under_construction = true;
        let id2 = find_upgrade_target_id(&map, 1, BuildingKind::City, click, &[b1, b2]);
        assert_eq!(id2, Some(2));
    }

    #[test]
    fn construction_tick_emits_structure_ready() {
        use bevy::ecs::system::SystemState;
        use crate::game::{GameEvent, GamePhase, GameState};
        use crate::player::Player;
        let mut world = World::new();
        let mut game = GameState::new(3, 5, 1);
        game.phase = GamePhase::Playing;
        let (map, owner) = tiny_owned_map();
        game.map = map;
        game.players.push(Player::new_human(owner, "c".into(), [1.0, 0.0, 0.0]));
        game.player_lookup.resize(owner as usize + 1, None);
        game.player_lookup[owner as usize] = Some(0);
        world.insert_resource(game);
        world.spawn(Building {
            id: 7,
            owner_id: owner,
            tile_idx: 2,
            kind: BuildingKind::City,
            level: 1,
            under_construction: true,
            ticks_until_complete: 1,
        });

        {
            let mut state =
                SystemState::<(ResMut<GameState>, Query<(Entity, &mut Building)>)>::new(&mut world);
            let (mut game, mut q) = state.get_mut(&mut world);
            advance_building_construction_tick(&mut game, &mut q);
        }
        world.flush();

        let game = world.resource::<GameState>();
        assert!(game
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::StructureReady { id: 7, .. })));
    }

    #[test]
    fn construction_ready_events_sorted_by_building_id() {
        use bevy::ecs::system::SystemState;
        use crate::game::{GameEvent, GamePhase, GameState};
        use crate::player::Player;
        let mut world = World::new();
        let mut game = GameState::new(4, 5, 1);
        game.phase = GamePhase::Playing;
        let (map, owner) = tiny_owned_map();
        game.map = map;
        game.players.push(Player::new_human(owner, "d".into(), [1.0, 0.0, 0.0]));
        game.player_lookup.resize(owner as usize + 1, None);
        game.player_lookup[owner as usize] = Some(0);
        world.insert_resource(game);
        world.spawn(Building {
            id: 2,
            owner_id: owner,
            tile_idx: 1,
            kind: BuildingKind::Factory,
            level: 1,
            under_construction: true,
            ticks_until_complete: 1,
        });
        world.spawn(Building {
            id: 1,
            owner_id: owner,
            tile_idx: 3,
            kind: BuildingKind::City,
            level: 1,
            under_construction: true,
            ticks_until_complete: 1,
        });

        {
            let mut state =
                SystemState::<(ResMut<GameState>, Query<(Entity, &mut Building)>)>::new(&mut world);
            let (mut game, mut q) = state.get_mut(&mut world);
            advance_building_construction_tick(&mut game, &mut q);
        }
        world.flush();

        let game = world.resource::<GameState>();
        let ready_ids: Vec<u64> = game
            .events
            .iter()
            .filter_map(|e| match e {
                GameEvent::StructureReady { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(ready_ids, vec![1, 2]);
    }

    #[test]
    fn compute_buildables_disables_when_too_poor() {
        let (map, owner) = tiny_owned_map();
        let rows = compute_buildables_for_player(&map, owner, 0.0, &[]);
        for e in rows {
            assert!(!e.can_build, "{:?}", e.kind);
        }
    }

    #[test]
    fn compute_buildables_port_invalid_without_shore() {
        let w = 6u32;
        let mut m = GameMap::new(w, 1);
        let owner = 1u16;
        for x in 0..w {
            m.set_owner_id(x, 0, owner);
            let ri = m.ref_id(x, 0);
            // Land but no shoreline bit — cannot place Port.
            m.terrain[ri] = MapTile::from_byte(0b1000_0000);
        }
        let rows = compute_buildables_for_player(&m, owner, 50_000.0, &[]);
        let port = rows.iter().find(|e| e.kind == BuildingKind::Port).unwrap();
        assert!(!port.can_build);
    }

    #[test]
    fn aggregate_ignores_under_construction() {
        let b = [
            Building {
                id: 1,
                owner_id: 1,
                tile_idx: 0,
                kind: BuildingKind::City,
                level: 5,
                under_construction: true,
                ticks_until_complete: 3,
            },
            Building {
                id: 2,
                owner_id: 1,
                tile_idx: 1,
                kind: BuildingKind::City,
                level: 2,
                under_construction: false,
                ticks_until_complete: 0,
            },
        ];
        let aggs = aggregate_buildings_per_player(b.into_iter(), 2);
        assert_eq!(aggs[1].city_levels, 2);
    }

    #[test]
    fn defense_post_bonus_scales_with_level() {
        let w = 20u32;
        let b = Building {
            id: 1,
            owner_id: 2,
            tile_idx: xy_idx(10, 10, w),
            kind: BuildingKind::DefensePost,
            level: 2,
            under_construction: false,
            ticks_until_complete: 0,
        };
        let bonus = defense_post_priority_bonus(&[b], 10, 10, w);
        assert_eq!(bonus, 2 * crate::config::DEFENSE_POST_PRIORITY_PER_LEVEL);
        let bonus_far = defense_post_priority_bonus(&[b], 0, 0, w);
        assert_eq!(bonus_far, 0);
    }

    #[test]
    fn openfront_cost_scaling_city_and_port_factory_shared_counter() {
        let s = crate::config::OPENFRONT_GOLD_SCALE;
        let owner = 1u16;
        let city0 = structure_build_cost_gold(BuildingKind::City, owner, &[]);
        assert_eq!(city0, 125_000.0 / s);

        let one_city = [Building {
            id: 1,
            owner_id: owner,
            tile_idx: 0,
            kind: BuildingKind::City,
            level: 1,
            under_construction: false,
            ticks_until_complete: 0,
        }];
        let city1 = structure_build_cost_gold(BuildingKind::City, owner, &one_city);
        assert_eq!(city1, 250_000.0 / s);

        let pf = [
            Building {
                id: 2,
                owner_id: owner,
                tile_idx: 1,
                kind: BuildingKind::Port,
                level: 1,
                under_construction: false,
                ticks_until_complete: 0,
            },
            Building {
                id: 3,
                owner_id: owner,
                tile_idx: 2,
                kind: BuildingKind::Factory,
                level: 1,
                under_construction: false,
                ticks_until_complete: 0,
            },
        ];
        let next_pf = structure_build_cost_gold(BuildingKind::Factory, owner, &pf);
        assert_eq!(next_pf, 500_000.0 / s);
    }

    #[test]
    fn missile_structures_disabled_in_buildables_when_feature_off() {
        let (map, owner) = tiny_owned_map();
        let rows = compute_buildables_for_player(&map, owner, 1_000_000.0, &[]);
        let sam = rows
            .iter()
            .find(|e| e.kind == BuildingKind::SamLauncher)
            .unwrap();
        let silo = rows
            .iter()
            .find(|e| e.kind == BuildingKind::MissileSilo)
            .unwrap();
        assert!(!sam.can_build);
        assert!(!silo.can_build);
        assert!(!sam.can_upgrade);
        assert!(!silo.can_upgrade);
    }
}
