use crate::math::MassiveNumber;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MatterTierId {
    QuantumDust,
    MicroMeteors,
    Asteroids,
    ProtoMoons,
    TerrestrialPlanets,
    GasGiants,
    StellarFurnaces,
    BlueHypergiants,
    NeutronStars,
    SupermassiveCores,
    SpacetimeRifts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatterTier {
    pub id: MatterTierId,
    pub name: String,
    pub description: String,
    pub count: u64,
    pub base_cost: MassiveNumber,
    pub cost_multiplier: f64,
    pub base_dps: MassiveNumber,
    pub unlock_mass: MassiveNumber,
    pub is_unlocked: bool,
}

impl MatterTier {
    pub fn new(
        id: MatterTierId,
        name: impl Into<String>,
        description: impl Into<String>,
        base_cost: MassiveNumber,
        cost_multiplier: f64,
        base_dps: MassiveNumber,
        unlock_mass: MassiveNumber,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            description: description.into(),
            count: 0,
            base_cost,
            cost_multiplier,
            base_dps,
            unlock_mass,
            is_unlocked: unlock_mass.is_zero(),
        }
    }

    pub fn cost_for(&self, amount: u64, cost_discount: f64) -> MassiveNumber {
        if amount == 0 {
            return MassiveNumber::ZERO;
        }

        let discount_mult = (1.0 - cost_discount).clamp(0.1, 1.0);
        let r = self.cost_multiplier;
        let base = self.base_cost.mul_f64(discount_mult);

        if amount == 1 {
            let scale = libm::pow(r, self.count as f64);
            base.mul_f64(scale)
        } else {
            let r_k = libm::pow(r, self.count as f64);
            let r_n = libm::pow(r, amount as f64);
            let sum_factor = r_k * (r_n - 1.0) / (r - 1.0);
            base.mul_f64(sum_factor)
        }
    }

    pub fn max_affordable(&self, available_mass: MassiveNumber, cost_discount: f64) -> u64 {
        if self.cost_for(1, cost_discount) > available_mass {
            return 0;
        }

        let mut low = 1u64;
        let mut high = 100_000u64;
        let mut best = 1u64;

        while low <= high {
            let mid = (low + high) / 2;
            let cost = self.cost_for(mid, cost_discount);
            if cost <= available_mass {
                best = mid;
                low = mid + 1;
            } else {
                high = mid - 1;
            }
        }
        best
    }

    pub fn dps(&self, tier_multiplier: MassiveNumber) -> MassiveNumber {
        if self.count == 0 {
            MassiveNumber::ZERO
        } else {
            self.base_dps * MassiveNumber::from_u64(self.count) * tier_multiplier
        }
    }

    pub fn buy(&mut self, amount: u64) {
        self.count += amount;
    }
}

pub fn default_matter_tiers() -> Vec<MatterTier> {
    vec![
        MatterTier::new(
            MatterTierId::QuantumDust,
            "Quantum Gas & Photons",
            "Siphoning virtual particles, neutrinos, and background photons into the event horizon.",
            MassiveNumber::from_f64(10.0),
            1.15,
            MassiveNumber::from_f64(1.0),
            MassiveNumber::ZERO,
        ),
        MatterTier::new(
            MatterTierId::MicroMeteors,
            "Micro-Meteorites",
            "Capturing dense chondrule grains and interplanetary dust ribbons.",
            MassiveNumber::from_f64(120.0),
            1.15,
            MassiveNumber::from_f64(8.0),
            MassiveNumber::from_f64(80.0),
        ),
        MatterTier::new(
            MatterTierId::Asteroids,
            "Nickel-Iron Asteroids",
            "Dragging kilometer-sized metallic asteroids into the tidal destruction zone.",
            MassiveNumber::from_f64(1_500.0),
            1.15,
            MassiveNumber::from_f64(75.0),
            MassiveNumber::from_f64(1_000.0),
        ),
        MatterTier::new(
            MatterTierId::ProtoMoons,
            "Proto-Moons & Planetesimals",
            "Crushing planetary satellites into glowing accretion rings.",
            MassiveNumber::from_f64(22_000.0),
            1.14,
            MassiveNumber::from_f64(900.0),
            MassiveNumber::from_f64(15_000.0),
        ),
        MatterTier::new(
            MatterTierId::TerrestrialPlanets,
            "Terrestrial Planets",
            "Swallowing entire rocky worlds and their oceans in a single orbit.",
            MassiveNumber::from_f64(350_000.0),
            1.14,
            MassiveNumber::from_f64(12_000.0),
            MassiveNumber::from_f64(250_000.0),
        ),
        MatterTier::new(
            MatterTierId::GasGiants,
            "Gas Giants",
            "Devouring colossal hydrogen-helium envelopes with violent relativistic shockwaves.",
            MassiveNumber::from_f64(6.0e6),
            1.13,
            MassiveNumber::from_f64(180_000.0),
            MassiveNumber::from_f64(4.0e6),
        ),
        MatterTier::new(
            MatterTierId::StellarFurnaces,
            "Stellar Furnaces",
            "Direct ingestion of main-sequence stars and red dwarfs.",
            MassiveNumber::from_f64(1.2e8),
            1.13,
            MassiveNumber::from_f64(3.2e6),
            MassiveNumber::from_f64(8.0e7),
        ),
        MatterTier::new(
            MatterTierId::BlueHypergiants,
            "Blue Hypergiants",
            "Super-luminous stellar behemoths crushed by extreme spacetime curvature.",
            MassiveNumber::from_f64(3.0e9),
            1.12,
            MassiveNumber::from_f64(7.5e7),
            MassiveNumber::from_f64(2.0e9),
        ),
        MatterTier::new(
            MatterTierId::NeutronStars,
            "Neutron Stars & Pulsars",
            "Merging ultra-dense degenerate nuclear matter directly into the singularity.",
            MassiveNumber::from_f64(8.0e10),
            1.12,
            MassiveNumber::from_f64(1.8e9),
            MassiveNumber::from_f64(5.0e10),
        ),
        MatterTier::new(
            MatterTierId::SupermassiveCores,
            "Galactic Core Clusters",
            "Consuming whole globular clusters and intermediate black holes.",
            MassiveNumber::from_f64(2.5e12),
            1.11,
            MassiveNumber::from_f64(4.8e10),
            MassiveNumber::from_f64(1.5e12),
        ),
        MatterTier::new(
            MatterTierId::SpacetimeRifts,
            "Spacetime Void Rifts",
            "Ripping open higher-dimensional dark rifts that channel raw vacuum energy.",
            MassiveNumber::from_f64(1.0e15),
            1.10,
            MassiveNumber::from_f64(1.5e13),
            MassiveNumber::from_f64(6.0e14),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_cost_and_dps() {
        let mut tier = MatterTier::new(
            MatterTierId::QuantumDust,
            "Quantum Dust",
            "Test tier",
            MassiveNumber::from_f64(10.0),
            1.15,
            MassiveNumber::from_f64(1.0),
            MassiveNumber::ZERO,
        );

        assert_eq!(tier.count, 0);
        assert_eq!(tier.dps(MassiveNumber::ONE), MassiveNumber::ZERO);

        let cost_1 = tier.cost_for(1, 0.0);
        assert_eq!(cost_1.to_f64(), 10.0);

        let cost_10 = tier.cost_for(10, 0.0);
        assert!(cost_10 > cost_1);

        tier.buy(5);
        assert_eq!(tier.count, 5);
        assert_eq!(tier.dps(MassiveNumber::ONE).to_f64(), 5.0);
    }
}
