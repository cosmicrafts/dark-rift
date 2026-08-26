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
        black_hole.mass = MassiveNumber::ZERO; // Start at 0 kg
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
            version: 3,
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

        // Update tutorial
        let tier0_count = self.tiers.first().map(|t| t.count).unwrap_or(0);
        self.tutorial.update(dt, self.black_hole.mass.to_f64(), tier0_count);

        // Check tier unlock conditions
        for tier in &mut self.tiers {
            if !tier.is_unlocked && self.black_hole.lifetime_mass >= tier.unlock_mass {
                tier.is_unlocked = true;
            }
        }

        // Check upgrade unlock conditions
        for upgrade in &mut self.upgrades {
            if !upgrade.is_unlocked && self.black_hole.lifetime_mass >= upgrade.unlock_mass {
                upgrade.is_unlocked = true;
            }
        }

        // Update black hole mass & accretion
        let dps = self.calculate_total_dps();
        self.black_hole.update(dt, dps);

        // Spin adjustment based on frame dragging upgrade & prestige
        let kerr_lvl = self.prestige.upgrade_level(ShardUpgradeId::KerrVortexResonance);
        let frame_drag_lvl = self.upgrade_level(UpgradeId::FrameDraggingCatalyst);
        self.black_hole.spin = (0.10 + (frame_drag_lvl as f32) * 0.015 + (kerr_lvl as f32) * 0.03).min(0.998);

        // Pulse timer decay
        if self.pulse_timer > 0.0 {
            self.pulse_timer = (self.pulse_timer - dt).max(0.0);
            if self.pulse_timer == 0.0 {
                self.pulse_active = false;
            }
        }

        // Polar jet periodic eruption
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

        // Dynamic particle scaling
        let total_bought: u64 = self.tiers.iter().map(|t| t.count).sum();
        let target_particles = if total_bought == 0 && self.black_hole.mass.to_f64() < 10.0 {
            12
        } else {
            16 + ((total_bought as usize) * 24).min(4080)
        };
        self.particles.set_active_count(target_particles);

        // Update particles
        let visual_rs = self.black_hole.visual_radius;
        let spin = self.black_hole.spin;
        let swallowed = self.particles.update(dt, visual_rs, spin, self.pulse_active);
        if swallowed > 0 {
            let bonus_mass = self.black_hole.pulse_mass.mul_f64((swallowed as f64) * 0.1) * self.calculate_pulse_multiplier();
            self.black_hole.mass += bonus_mass;
            self.black_hole.lifetime_mass += bonus_mass;
        }

        // Update quests
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

    pub fn trigger_pulse(&mut self) -> MassiveNumber {
        let mult = self.calculate_pulse_multiplier();
        let gained = self.black_hole.trigger_pulse(mult);
        self.pulse_active = true;
        self.pulse_timer = 0.35;

        // Advance tutorial on click
        self.tutorial.on_click(self.black_hole.mass.to_f64());

        // Penrose Boost
        if self.black_hole.penrose_timer <= 0.0 && self.black_hole.accretion_flash > 0.3 {
            let penrose_lvl = self.upgrade_level(UpgradeId::PenroseResonanceChamber);
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
                    self.tutorial.on_tier_bought();
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
        assert!(game.black_hole.mass > MassiveNumber::ZERO);
        assert!(game.pulse_active);
    }

    #[test]
    fn test_tutorial_progression() {
        let mut game = GameState::new();
        assert_eq!(game.tutorial.step, crate::tutorial::TutorialStep::PulseThreeTimes);

        game.trigger_pulse();
        game.trigger_pulse();
        game.trigger_pulse();
        assert_eq!(game.tutorial.step, crate::tutorial::TutorialStep::ReachTenKg);

        game.black_hole.mass = MassiveNumber::from_f64(10.0);
        game.tick(0.1);
        assert_eq!(game.tutorial.step, crate::tutorial::TutorialStep::BuyFirstTier);

        let bought = game.buy_tier(MatterTierId::QuantumDust, 1);
        assert!(bought);
        assert_eq!(game.tutorial.step, crate::tutorial::TutorialStep::ExplainPassive);
    }
}
