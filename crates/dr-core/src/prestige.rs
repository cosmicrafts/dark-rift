use crate::math::MassiveNumber;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShardUpgradeId {
    PrimordialSingularity,
    ChronosDilation,
    VacuumPolarization,
    KerrVortexResonance,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShardUpgrade {
    pub id: ShardUpgradeId,
    pub name: String,
    pub description: String,
    pub level: u32,
    pub max_level: u32,
    pub base_cost_shards: u64,
    pub cost_multiplier: f64,
}

impl ShardUpgrade {
    pub fn new(
        id: ShardUpgradeId,
        name: impl Into<String>,
        description: impl Into<String>,
        max_level: u32,
        base_cost_shards: u64,
        cost_multiplier: f64,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            description: description.into(),
            level: 0,
            max_level,
            base_cost_shards,
            cost_multiplier,
        }
    }

    pub fn cost(&self) -> u64 {
        if self.level >= self.max_level {
            return u64::MAX;
        }
        let scale = libm::pow(self.cost_multiplier, self.level as f64);
        ((self.base_cost_shards as f64) * scale).round() as u64
    }

    pub fn can_buy(&self, available_shards: u64) -> bool {
        self.level < self.max_level && available_shards >= self.cost()
    }

    pub fn buy(&mut self, available_shards: &mut u64) -> bool {
        let cost = self.cost();
        if self.level < self.max_level && *available_shards >= cost {
            *available_shards -= cost;
            self.level += 1;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrestigeState {
    pub dark_matter_shards: u64,
    pub total_shards_earned: u64,
    pub total_collapses: u64,
    pub upgrades: Vec<ShardUpgrade>,
}

impl Default for PrestigeState {
    fn default() -> Self {
        Self::new()
    }
}

impl PrestigeState {
    pub fn new() -> Self {
        Self {
            dark_matter_shards: 0,
            total_shards_earned: 0,
            total_collapses: 0,
            upgrades: default_shard_upgrades(),
        }
    }

    pub fn pending_shards(&self, current_mass: MassiveNumber) -> u64 {
        let threshold = MassiveNumber::from_f64(1.0e9);
        if current_mass < threshold {
            return 0;
        }
        let ratio = current_mass / threshold;
        let sqrt_ratio = ratio.sqrt();
        let shards_num = sqrt_ratio.mul_f64(10.0);
        shards_num.to_f64().floor() as u64
    }

    pub fn global_multiplier(&self) -> MassiveNumber {
        let shard_bonus = 1.0 + (self.dark_matter_shards as f64) * 0.10;
        let primordial_lvl = self.upgrade_level(ShardUpgradeId::PrimordialSingularity);
        let primordial_mult = 1.0 + (primordial_lvl as f64) * 1.0;
        MassiveNumber::from_f64(shard_bonus * primordial_mult)
    }

    pub fn offline_efficiency(&self) -> f64 {
        let dilation_lvl = self.upgrade_level(ShardUpgradeId::ChronosDilation);
        (0.20 + (dilation_lvl as f64) * 0.20).clamp(0.20, 1.0)
    }

    pub fn cost_discount(&self) -> f64 {
        let vac_lvl = self.upgrade_level(ShardUpgradeId::VacuumPolarization);
        (vac_lvl as f64) * 0.08
    }

    pub fn upgrade_level(&self, id: ShardUpgradeId) -> u32 {
        self.upgrades
            .iter()
            .find(|u| u.id == id)
            .map(|u| u.level)
            .unwrap_or(0)
    }
}

pub fn default_shard_upgrades() -> Vec<ShardUpgrade> {
    vec![
        ShardUpgrade::new(
            ShardUpgradeId::PrimordialSingularity,
            "Primordial Graviton Core",
            "Permanently increases all mass generation by +100% per level.",
            50,
            1,
            2.0,
        ),
        ShardUpgrade::new(
            ShardUpgradeId::ChronosDilation,
            "Chronos Dilation Warp",
            "Increases offline mass generation efficiency by +20% per level (up to 100%).",
            4,
            2,
            3.0,
        ),
        ShardUpgrade::new(
            ShardUpgradeId::VacuumPolarization,
            "Vacuum Polarization",
            "Discounts all matter tier purchase costs by -8% per level.",
            8,
            3,
            2.5,
        ),
        ShardUpgrade::new(
            ShardUpgradeId::KerrVortexResonance,
            "Kerr Vortex Singularity",
            "Increases black hole spin and accretion rotation bonuses by +50% per level.",
            20,
            5,
            2.2,
        ),
    ]
}
