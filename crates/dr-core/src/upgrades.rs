use crate::math::MassiveNumber;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UpgradeId {
    GravitonResonator,
    FrameDraggingCatalyst,
    HawkingSiphon,
    RelativisticJetHarvester,
    PenroseResonanceChamber,
    TidalDisruptionTensors,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Upgrade {
    pub id: UpgradeId,
    pub name: String,
    pub description: String,
    pub level: u32,
    pub max_level: u32,
    pub base_cost: MassiveNumber,
    pub cost_multiplier: f64,
    pub unlock_mass: MassiveNumber,
    pub is_unlocked: bool,
}

impl Upgrade {
    pub fn new(
        id: UpgradeId,
        name: impl Into<String>,
        description: impl Into<String>,
        max_level: u32,
        base_cost: MassiveNumber,
        cost_multiplier: f64,
        unlock_mass: MassiveNumber,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            description: description.into(),
            level: 0,
            max_level,
            base_cost,
            cost_multiplier,
            unlock_mass,
            is_unlocked: false,
        }
    }

    pub fn cost(&self) -> MassiveNumber {
        if self.level >= self.max_level {
            return MassiveNumber::ZERO;
        }
        let scale = libm::pow(self.cost_multiplier, self.level as f64);
        self.base_cost.mul_f64(scale)
    }

    pub fn can_buy(&self, available_mass: MassiveNumber) -> bool {
        self.level < self.max_level && available_mass >= self.cost()
    }

    pub fn buy(&mut self) {
        if self.level < self.max_level {
            self.level += 1;
        }
    }
}

pub fn default_upgrades() -> Vec<Upgrade> {
    vec![
        Upgrade::new(
            UpgradeId::GravitonResonator,
            "Graviton Resonator",
            "Amplifies manual gravitational pulse shockwaves (+100% click power per level).",
            50,
            MassiveNumber::from_f64(50.0),
            1.75,
            MassiveNumber::from_f64(20.0),
        ),
        Upgrade::new(
            UpgradeId::FrameDraggingCatalyst,
            "Frame-Dragging Catalyst",
            "Accelerates spacetime rotation around the ergosphere (+25% accretion DPS per level).",
            50,
            MassiveNumber::from_f64(500.0),
            1.85,
            MassiveNumber::from_f64(250.0),
        ),
        Upgrade::new(
            UpgradeId::HawkingSiphon,
            "Hawking Radiation Siphon",
            "Converts quantum horizon fluctuations into raw mass (+30% global mass per level).",
            40,
            MassiveNumber::from_f64(10_000.0),
            2.10,
            MassiveNumber::from_f64(5_000.0),
        ),
        Upgrade::new(
            UpgradeId::RelativisticJetHarvester,
            "Relativistic Jet Harvester",
            "Harnesses magnetic collimation for periodic polar plasma eruptions (+50% DPS per level).",
            30,
            MassiveNumber::from_f64(250_000.0),
            2.40,
            MassiveNumber::from_f64(100_000.0),
        ),
        Upgrade::new(
            UpgradeId::PenroseResonanceChamber,
            "Penrose Resonance Chamber",
            "Doubles the active Penrose Boost multiplier and adds +2s duration per level.",
            20,
            MassiveNumber::from_f64(5.0e6),
            3.0,
            MassiveNumber::from_f64(2.0e6),
        ),
        Upgrade::new(
            UpgradeId::TidalDisruptionTensors,
            "Tidal Disruption Tensors",
            "Optimizes tidal shear stress, reducing all Matter Tier costs by 4% compound per level.",
            25,
            MassiveNumber::from_f64(1.0e8),
            2.5,
            MassiveNumber::from_f64(5.0e7),
        ),
    ]
}
