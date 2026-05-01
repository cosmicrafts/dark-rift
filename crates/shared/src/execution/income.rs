use bevy::prelude::*;
use crate::building::{aggregate_buildings_per_player, Building};
use crate::config;
use crate::game::{GamePhase, GameState};

// System analogous to PlayerExecution.tick() in OpenFront
pub fn system_player_income(
    game_opt: Option<ResMut<GameState>>,
    q_building: Query<&Building>,
    mut buildings_scratch: Local<Vec<Building>>,
) {
    let Some(mut game) = game_opt else { return };
    if game.phase != GamePhase::Playing {
        return;
    }

    let max_pid = game.players.iter().map(|p| p.id as usize).max().unwrap_or(0);
    buildings_scratch.clear();
    buildings_scratch.extend(q_building.iter().copied());
    let aggs = aggregate_buildings_per_player(buildings_scratch.iter().copied(), max_pid);

    let config = game.config.clone();
    let GameState { map: _, players, .. } = &mut *game;

    for player in players.iter_mut().filter(|p| p.alive) {
        let tiles_owned = player.tile_count;
        if tiles_owned == 0 {
            player.alive = false; // Dead
            continue;
        }

        let agg = aggs
            .get(player.id as usize)
            .copied()
            .unwrap_or_default();

        // Defensive mathematical bound: if player troops violently drain below zero during massive intent execution bursts, 
        // the calculation mathematically breaks and introduces desyncing NaN generations universally. We clamp to safely avoid this!
        let safe_troops = player.troops.max(0.0);
        
        // Use strictly deterministic IEEE-754 sqrt instead of libm powf!
        // tiles_owned^0.625 = tiles_owned^(1/2) * tiles_owned^(1/8)
        let t_f64 = tiles_owned as f64;
        let t_half = t_f64.sqrt();
        let t_quarter = t_half.sqrt();
        let t_eighth = t_quarter.sqrt();
        let max_troops_bonus = t_half * t_eighth;
        
        player.max_troops = config.max_troops_base
            + max_troops_bonus * config.max_troops_scale
            + agg.city_levels as f64 * config.city_max_troops_per_level;

        // safe_troops^0.75 = safe_troops^(1/2) * safe_troops^(1/4)
        let s_half = safe_troops.sqrt();
        let s_quarter = s_half.sqrt();
        let s_75 = s_half * s_quarter;
        
        let raw_income = config.troop_base_income + (s_75 / 4.6);
        let ratio = 1.0 - (safe_troops / player.max_troops).min(1.0);
        let factory_extra = (agg.factory_levels as f64 * config.factory_income_bonus_per_level)
            .min(config.factory_income_bonus_cap - 1.0);
        let factory_mult = 1.0 + factory_extra;
        let income = raw_income * ratio * factory_mult;

        player.troops = (safe_troops + income).min(player.max_troops);

        let safe_gold = player.gold.max(0.0);
        let gold_income = config.gold_base_income
            + agg.city_levels as f64 * config.gold_income_per_city_level;
        player.gold = safe_gold + gold_income;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::{App, MinimalPlugins};
    use crate::game::BuildingKind;
    use crate::player::Player;

    #[test]
    fn city_levels_raise_max_troops_via_income_system() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut game = GameState::new(42, 8, 8, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default()));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = 50;
            p.troops = 10.0;
        }
        app.insert_resource(game);
        app.world_mut().spawn(Building {
            id: 1,
            owner_id: 1,
            tile_idx: 0,
            kind: BuildingKind::City,
            level: 4,
            under_construction: false,
            ticks_until_complete: 0,
        });
        app.add_systems(Update, system_player_income);
        app.update();
        let game = app.world().resource::<GameState>();
        let p = game.player(1).unwrap();
        let expected_floor =
            config::MAX_TROOPS_BASE + 4.0 * config::CITY_MAX_TROOPS_PER_LEVEL;
        assert!(
            p.max_troops + 0.01 >= expected_floor,
            "max_troops={} expected>={}",
            p.max_troops,
            expected_floor
        );
    }

    #[test]
    fn factory_income_multiplier_respects_cap() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut game = GameState::new(43, 8, 8, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default()));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = 50;
            p.troops = 5.0;
        }
        app.insert_resource(game);
        for i in 0u32..20 {
            app.world_mut().spawn(Building {
                id: u64::from(i + 1),
                owner_id: 1,
                tile_idx: i,
                kind: BuildingKind::Factory,
                level: 1,
                under_construction: false,
                ticks_until_complete: 0,
            });
        }
        app.add_systems(Update, system_player_income);
        app.update();
        let game = app.world().resource::<GameState>();
        let p = game.player(1).unwrap();
        let low = 5.0_f64;
        let s_half = low.sqrt();
        let s_quarter = s_half.sqrt();
        let s_75 = s_half * s_quarter;
        let raw_income = config::TROOP_BASE_INCOME + (s_75 / 4.6);
        let ratio = 1.0 - (low / p.max_troops).min(1.0);
        let uncapped = raw_income * ratio * (1.0 + 20.0 * config::FACTORY_INCOME_BONUS_PER_LEVEL);
        let actual_gain = p.troops - low;
        assert!(
            actual_gain <= uncapped + 0.001,
            "income should be capped by FACTORY_INCOME_BONUS_CAP"
        );
        assert!(
            actual_gain <= raw_income * ratio * config::FACTORY_INCOME_BONUS_CAP + 0.02,
            "gain {} exceeds cap-scaled income",
            actual_gain
        );
    }

    #[test]
    fn gold_increments_each_income_tick() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut game = GameState::new(44, 8, 8, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default()));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = 10;
            p.gold = 100.0;
        }
        app.insert_resource(game);
        app.add_systems(Update, system_player_income);
        app.update();
        let game = app.world().resource::<GameState>();
        let p = game.player(1).unwrap();
        let delta = p.gold - 100.0;
        assert!(
            (delta - config::GOLD_BASE_INCOME).abs() < 0.001,
            "gold delta {} expected {}",
            delta,
            config::GOLD_BASE_INCOME
        );
    }

    #[test]
    fn gold_income_scales_with_ready_city_levels() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let mut game = GameState::new(45, 8, 8, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(1, "p".into(), [1.0, 0.0, 0.0], &crate::game_config::GameConfig::default()));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = 20;
            p.gold = 0.0;
        }
        app.insert_resource(game);
        app.world_mut().spawn(Building {
            id: 1,
            owner_id: 1,
            tile_idx: 0,
            kind: BuildingKind::City,
            level: 3,
            under_construction: false,
            ticks_until_complete: 0,
        });
        app.add_systems(Update, system_player_income);
        app.update();
        let game = app.world().resource::<GameState>();
        let p = game.player(1).unwrap();
        let expected = config::GOLD_BASE_INCOME + 3.0 * config::GOLD_INCOME_PER_CITY_LEVEL;
        assert!((p.gold - expected).abs() < 0.001, "gold={}", p.gold);
    }
}
