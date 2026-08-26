use crate::black_hole::BlackHole;
use crate::math::MassiveNumber;
use crate::particles::ParticleSim;
use crate::prestige::{PrestigeState, ShardUpgradeId};
use crate::quests::QuestManager;
use crate::tiers::{default_matter_tiers, MatterTier, MatterTierId};
use crate::tutorial::TutorialState;
use crate::upgrades::{default_upgrades, Upgrade, UpgradeId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameState {
    pub black_hole: BlackHole,
    pub tiers: Vec<MatterTier>,
    pub upgrades: Vec<Upgrade>,
    pub prestige: PrestigeState,
    pub particles: ParticleSim,
    pub quests: QuestManager,
    pub tutorial: TutorialState,
    pub time_played_secs: f64,
    pub pulse_active: bool,
    pub pulse_timer: f32,
    pub polar_jet_timer: f32,
    pub polar_jet_active: bool,
    pub version: u32,
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

impl GameState {
    pub fn new() -> Self {
        let mut black_hole = BlackHole::new();
        black_hole.mass = MassiveNumber::ZERO; // Start at exact 0.0 kg
        black_hole.lifetime_mass = MassiveNumber::ZERO;

        Self {
            black_hole,
            tiers: default_matter_tiers(),
            upgrades: default_upgrades(),
            prestige: PrestigeState::new(),
            particles: ParticleSim::new(),
            quests: QuestManager::new(),
            tutorial: TutorialState::new(),
            time_played_secs: 0.0,
            pulse_active: false,
            pulse_timer: 0.0,
            polar_jet_timer: 0.0,
            polar_jet_active: false,
            version: 5,
        }
    }

    pub fn calculate_total_dps(&self) -> MassiveNumber {
        let frame_drag_lvl = self.upgrade_level(UpgradeId::FrameDraggingCatalyst);
        let frame_drag_mult = MassiveNumber::from_f64(1.0 + (frame_drag_lvl as f64) * 0.25);

        let hawking_lvl = self.upgrade_level(UpgradeId::HawkingSiphon);
        let hawking_mult = MassiveNumber::from_f64(1.0 + (hawking_lvl as f64) * 0.30);

        let jet_lvl = self.upgrade_level(UpgradeId::RelativisticJetHarvester);
        let jet_mult = MassiveNumber::from_f64(1.0 + (jet_lvl as f64) * 0.50);

        let global_prestige = self.prestige.global_multiplier();
        let combined_multiplier = frame_drag_mult * hawking_mult * jet_mult * global_prestige;

        let mut total = MassiveNumber::ZERO;
        for tier in &self.tiers {
            if tier.is_unlocked {
                total += tier.dps(combined_multiplier);
            }
        }
        total
    }

    pub fn calculate_pulse_multiplier(&self) -> MassiveNumber {
        let pulse_lvl = self.upgrade_level(UpgradeId::GravitonResonator);
        let pulse_mult = MassiveNumber::from_f64(1.0 + (pulse_lvl as f64) * 1.0);
        pulse_mult * self.prestige.global_multiplier()
    }

    pub fn tick(&mut self, dt: f32) {
        self.time_played_secs += dt as f64;

        // 1. Update tutorial state
        let first_tier_count = self.tiers.first().map(|t| t.count).unwrap_or(0);
        self.tutorial.update(dt, self.black_hole.mass.to_f64(), first_tier_count);

        // 2. Check tier unlock conditions
        for tier in &mut self.tiers {
            if !tier.is_unlocked && self.black_hole.lifetime_mass >= tier.unlock_mass {
                tier.is_unlocked = true;
            }
        }

        // 3. Check upgrade unlock conditions
        for upgrade in &mut self.upgrades {
            if !upgrade.is_unlocked && self.black_hole.lifetime_mass >= upgrade.unlock_mass {
                upgrade.is_unlocked = true;
            }
        }

        // 4. Update black hole mass & accretion
        let dps = self.calculate_total_dps();
        self.black_hole.update(dt, dps);

        // 5. Dynamic particle simulation based on active generators
        let total_generators: u64 = self.tiers.iter().map(|t| t.count).sum();
        if total_generators > 0 {
            let target_count = 16 + ((total_generators as usize) * 8).min(480);
            self.particles.set_active_count(target_count);
            self.particles.update(
                dt,
                self.black_hole.visual_radius,
                self.black_hole.spin,
                self.pulse_active,
            );
        } else {
            self.particles.set_active_count(0);
        }

        // 6. Spin adjustment based on frame dragging upgrade & prestige
        let kerr_lvl = self.prestige.upgrade_level(ShardUpgradeId::KerrVortexResonance);
        let frame_drag_lvl = self.upgrade_level(UpgradeId::FrameDraggingCatalyst);
        self.black_hole.spin = (0.10 + (frame_drag_lvl as f32) * 0.015 + (kerr_lvl as f32) * 0.03).min(0.998);

        // 7. Pulse timer decay
        if self.pulse_timer > 0.0 {
            self.pulse_timer = (self.pulse_timer - dt).max(0.0);
            if self.pulse_timer == 0.0 {
                self.pulse_active = false;
            }
        }

        // 8. Polar jet periodic eruption (only if unlocked)
        self.polar_jet_timer += dt;
        if self.polar_jet_timer >= 12.0 {
            self.polar_jet_timer = 0.0;
            let jet_lvl = self.upgrade_level(UpgradeId::RelativisticJetHarvester);
            if jet_lvl > 0 {
                self.polar_jet_active = true;
                let burst = dps.mul_f64(5.0 * (jet_lvl as f64));
                self.black_hole.mass += burst;
                self.black_hole.lifetime_mass += burst;
            }
        } else if self.polar_jet_timer > 2.0 {
            self.polar_jet_active = false;
        }

        // 9. Update quests (only after tutorial is fully completed)
        if self.tutorial.is_completed() {
            let total_tech: u32 = self.upgrades.iter().map(|u| u.level).sum();
            let terrestrial_count = self.tiers.iter().find(|t| t.id == MatterTierId::TerrestrialPlanets).map(|t| t.count).unwrap_or(0);
            if let Some(reward) = self.quests.update(
                self.black_hole.total_pulses,
                self.black_hole.mass,
                self.tiers[0].count,
                total_tech,
                self.black_hole.penrose_timer > 0.0,
                terrestrial_count,
                self.prestige.total_collapses,
            ) {
                if reward > MassiveNumber::ZERO {
                    self.black_hole.mass += reward;
                    self.black_hole.lifetime_mass += reward;
                    self.black_hole.accretion_flash = 1.0;
                }
            }
        }
    }

    pub fn trigger_pulse(&mut self) -> MassiveNumber {
        let mult = self.calculate_pulse_multiplier();
        let gained = self.black_hole.trigger_pulse(mult);
        self.pulse_active = true;
        self.pulse_timer = 0.35;

        // Advance tutorial
        self.tutorial.on_click(self.black_hole.mass.to_f64());

        // ============================================================================
        // WARNING / ANTI-PATTERN FOR FUTURE AGENTS:
        // NEVER trigger locked mechanics autonomously! ALWAYS check upgrade level > 0.
        // Failing to verify `penrose_lvl > 0` caused Penrose Boost to fire on the
        // player's first click, displaying confusing banners and multiplying mass
        // without user intent.
        // ============================================================================
        let penrose_lvl = self.upgrade_level(UpgradeId::PenroseResonanceChamber);
        if penrose_lvl > 0 && self.black_hole.penrose_timer <= 0.0 && self.black_hole.accretion_flash > 0.4 {
            let duration = 3.0 + (penrose_lvl as f32) * 1.5;
            let boost_mult = 2.0 + (penrose_lvl as f32) * 0.5;
            self.black_hole.activate_penrose_boost(duration, boost_mult);
        }

        gained
    }

    pub fn buy_tier(&mut self, id: MatterTierId, amount: u64) -> bool {
        let discount = self.prestige.cost_discount();
        let cost_opt = self.tiers.iter().find(|t| t.id == id).map(|t| t.cost_for(amount, discount));
        if let Some(cost) = cost_opt {
            if cost <= self.black_hole.mass && amount > 0 {
                self.black_hole.mass -= cost;
                if let Some(tier) = self.tiers.iter_mut().find(|t| t.id == id) {
                    tier.buy(amount);
                    self.tutorial.on_generator_bought();
                    return true;
                }
            }
        }
        false
    }

    pub fn buy_upgrade(&mut self, id: UpgradeId) -> bool {
        let cost_opt = self.upgrades.iter().find(|u| u.id == id).map(|u| u.cost());
        if let Some(cost) = cost_opt {
            if self.black_hole.mass >= cost {
                if let Some(upgrade) = self.upgrades.iter_mut().find(|u| u.id == id) {
                    if upgrade.level < upgrade.max_level {
                        self.black_hole.mass -= cost;
                        upgrade.buy();
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn buy_shard_upgrade(&mut self, id: ShardUpgradeId) -> bool {
        if let Some(upgrade) = self.prestige.upgrades.iter_mut().find(|u| u.id == id) {
            return upgrade.buy(&mut self.prestige.dark_matter_shards);
        }
        false
    }

    pub fn big_crunch(&mut self) -> bool {
        let shards = self.prestige.pending_shards(self.black_hole.mass);
        if shards == 0 {
            return false;
        }

        self.prestige.dark_matter_shards += shards;
        self.prestige.total_shards_earned += shards;
        self.prestige.total_collapses += 1;

        self.black_hole = BlackHole::new();
        self.tiers = default_matter_tiers();
        self.upgrades = default_upgrades();
        self.particles.init_particles();
        true
    }

    pub fn process_offline_progress(&mut self, elapsed_secs: f64) -> MassiveNumber {
        if elapsed_secs < 5.0 {
            return MassiveNumber::ZERO;
        }

        let clamped_secs = elapsed_secs.min(86400.0 * 7.0);
        let dps = self.calculate_total_dps();
        let eff = self.prestige.offline_efficiency();
        let gained = dps.mul_f64(clamped_secs * eff);

        self.black_hole.mass += gained;
        self.black_hole.lifetime_mass += gained;
        gained
    }

    pub fn upgrade_level(&self, id: UpgradeId) -> u32 {
        self.upgrades
            .iter()
            .find(|u| u.id == id)
            .map(|u| u.level)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_tick_and_pulse() {
        let mut game = GameState::new();
        assert_eq!(game.black_hole.mass.to_f64(), 0.0);

        game.trigger_pulse();
        assert_eq!(game.black_hole.mass.to_f64(), 1.0);
        assert!(game.pulse_active);
    }

    #[test]
    fn test_tutorial_progression() {
        let mut game = GameState::new();
        assert_eq!(game.tutorial.step, TutorialStep::AwakenSingularity);

        for _ in 0..10 {
            game.trigger_pulse();
        }
        assert_eq!(game.tutorial.step, TutorialStep::BuyFirstGenerator);
        assert_eq!(game.black_hole.mass.to_f64(), 10.0);

        let bought = game.buy_tier(MatterTierId::QuantumDust, 1);
        assert!(bought);
        assert_eq!(game.tutorial.step, TutorialStep::ExplainPassive);

        game.tick(7.0);
        assert_eq!(game.tutorial.step, TutorialStep::Completed);
    }
}
