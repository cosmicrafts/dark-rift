use crate::building::aggregate_buildings_per_player;
use crate::game::{GamePhase, GameState};
use crate::engine::DarkRiftEngine;

impl DarkRiftEngine {
    pub fn execute_income(&mut self) {
        if self.state.phase != GamePhase::Playing {
            return;
        }

        let max_pid = self.state.players.iter().map(|p| p.id as usize).max().unwrap_or(0);
        let aggs = aggregate_buildings_per_player(self.buildings.iter().copied(), max_pid);

        let config = self.state.config.clone();
        let GameState { map: _, players, .. } = &mut self.state;

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
        if player.player_type == crate::player::PlayerType::Bot {
            player.max_troops /= 3.0;
        }

        // safe_troops^0.75 = safe_troops^(1/2) * safe_troops^(1/4)
        let s_half = safe_troops.sqrt();
        let s_quarter = s_half.sqrt();
        let s_75 = s_half * s_quarter;
        
        let raw_income = config.troop_base_income + (s_75 / 4.0);
        let ratio = 1.0 - (safe_troops / player.max_troops).min(1.0);
        let factory_extra = (agg.factory_levels as f64 * config.factory_income_bonus_per_level)
            .min(config.factory_income_bonus_cap - 1.0);
        let factory_mult = 1.0 + factory_extra;
        let mut income = raw_income * ratio * factory_mult;
        
        if player.player_type == crate::player::PlayerType::Bot {
            income *= 0.6;
        }

        player.troops = (safe_troops + income).min(player.max_troops);

        let safe_gold = player.gold.max(0.0);
        let gold_income = config.gold_base_income
            + agg.city_levels as f64 * config.gold_income_per_city_level;
        player.gold = safe_gold + gold_income;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::Building;
    use crate::game::BuildingKind;
    use crate::player::Player;
    use crate::water_components::WaterComponents;

    fn engine_one_player(seed: u64, tiles: u32, troops: f64, gold: f64) -> DarkRiftEngine {
        let mut game =
            GameState::new(seed, 8, 8, crate::game_config::GameConfig::default());
        game.phase = GamePhase::Playing;
        game.players.push(Player::new_human(
            1,
            "p".into(),
            [1.0, 0.0, 0.0],
            &crate::game_config::GameConfig::default(),
        ));
        game.player_lookup = vec![None, Some(0)];
        if let Some(p) = game.player_mut(1) {
            p.tile_count = tiles;
            p.troops = troops;
            p.gold = gold;
        }
        DarkRiftEngine::new(game, WaterComponents::default())
    }

    #[test]
    fn city_levels_raise_max_troops_via_income_system() {
        let mut engine = engine_one_player(42, 50, 10.0, 0.0);
        engine.buildings.push(Building {
            id: 1,
            owner_id: 1,
            tile_idx: 0,
            kind: BuildingKind::City,
            level: 4,
            under_construction: false,
            ticks_until_complete: 0,
        });
        engine.execute_income();
        let cfg = &engine.state.config;
        let p = engine.state.player(1).unwrap();
        let expected_floor = cfg.max_troops_base + 4.0 * cfg.city_max_troops_per_level;
        assert!(
            p.max_troops + 0.01 >= expected_floor,
            "max_troops={} expected>={}",
            p.max_troops,
            expected_floor
        );
    }

    #[test]
    fn factory_income_multiplier_respects_cap() {
        let mut engine = engine_one_player(43, 50, 5.0, 0.0);
        for i in 0u32..20 {
            engine.buildings.push(Building {
                id: u64::from(i + 1),
                owner_id: 1,
                tile_idx: i,
                kind: BuildingKind::Factory,
                level: 1,
                under_construction: false,
                ticks_until_complete: 0,
            });
        }
        engine.execute_income();
        let p = engine.state.player(1).unwrap();
        let cfg = &engine.state.config;
        let low = 5.0_f64;
        let s_half = low.sqrt();
        let s_quarter = s_half.sqrt();
        let s_75 = s_half * s_quarter;
        let raw_income = cfg.troop_base_income + (s_75 / 4.0);
        let ratio = 1.0 - (low / p.max_troops).min(1.0);
        let uncapped = raw_income
            * ratio
            * (1.0 + 20.0 * cfg.factory_income_bonus_per_level);
        let actual_gain = p.troops - low;
        assert!(
            actual_gain <= uncapped + 0.001,
            "income should be capped by FACTORY_INCOME_BONUS_CAP"
        );
        assert!(
            actual_gain <= raw_income * ratio * cfg.factory_income_bonus_cap + 0.02,
            "gain {} exceeds cap-scaled income",
            actual_gain
        );
    }

    #[test]
    fn gold_increments_each_income_tick() {
        let mut engine = engine_one_player(44, 10, 0.0, 100.0);
        engine.execute_income();
        let p = engine.state.player(1).unwrap();
        let delta = p.gold - 100.0;
        let g = engine.state.config.gold_base_income;
        assert!(
            (delta - g).abs() < 0.001,
            "gold delta {} expected {}",
            delta,
            g
        );
    }

    #[test]
    fn gold_income_scales_with_ready_city_levels() {
        let mut engine = engine_one_player(45, 20, 0.0, 0.0);
        engine.buildings.push(Building {
            id: 1,
            owner_id: 1,
            tile_idx: 0,
            kind: BuildingKind::City,
            level: 3,
            under_construction: false,
            ticks_until_complete: 0,
        });
        engine.execute_income();
        let p = engine.state.player(1).unwrap();
        let cfg = &engine.state.config;
        let expected =
            cfg.gold_base_income + 3.0 * cfg.gold_income_per_city_level;
        assert!((p.gold - expected).abs() < 0.001, "gold={}", p.gold);
    }
}
