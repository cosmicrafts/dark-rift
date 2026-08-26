use crate::math::MassiveNumber;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestId {
    AwakenSingularity,
    FirstAccretion,
    QuantumGenerator,
    CriticalDensity,
    TechnologicalSingularity,
    PenroseResonance,
    PlanetaryDestruction,
    TheBigCrunch,
    CompletedAll,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quest {
    pub id: QuestId,
    pub title: String,
    pub instruction: String,
    pub current_progress: f64,
    pub target_progress: f64,
    pub reward_mass: MassiveNumber,
    pub reward_shards: u64,
    pub is_completed: bool,
}

impl Quest {
    pub fn new(
        id: QuestId,
        title: impl Into<String>,
        instruction: impl Into<String>,
        target_progress: f64,
        reward_mass: MassiveNumber,
        reward_shards: u64,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            instruction: instruction.into(),
            current_progress: 0.0,
            target_progress,
            reward_mass,
            reward_shards,
            is_completed: false,
        }
    }

    pub fn progress_fraction(&self) -> f32 {
        if self.target_progress <= 0.0 {
            1.0
        } else {
            ((self.current_progress / self.target_progress) as f32).clamp(0.0, 1.0)
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuestManager {
    pub quests: Vec<Quest>,
    pub current_quest_index: usize,
}

impl Default for QuestManager {
    fn default() -> Self {
        Self::new()
    }
}

impl QuestManager {
    pub fn new() -> Self {
        Self {
            quests: default_quests(),
            current_quest_index: 0,
        }
    }

    pub fn current_quest(&self) -> Option<&Quest> {
        self.quests.get(self.current_quest_index)
    }

    pub fn current_quest_mut(&mut self) -> Option<&mut Quest> {
        self.quests.get_mut(self.current_quest_index)
    }

    pub fn update(
        &mut self,
        total_pulses: u64,
        current_mass: MassiveNumber,
        quantum_tiers: u64,
        tech_upgrades: u32,
        penrose_active: bool,
        terrestrial_planets: u64,
        total_collapses: u64,
    ) -> Option<MassiveNumber> {
        let quest = self.current_quest_mut()?;
        if quest.is_completed {
            return None;
        }

        match quest.id {
            QuestId::AwakenSingularity => {
                quest.current_progress = total_pulses as f64;
            }
            QuestId::FirstAccretion => {
                quest.current_progress = current_mass.to_f64().min(quest.target_progress);
            }
            QuestId::QuantumGenerator => {
                quest.current_progress = quantum_tiers as f64;
            }
            QuestId::CriticalDensity => {
                quest.current_progress = current_mass.to_f64().min(quest.target_progress);
            }
            QuestId::TechnologicalSingularity => {
                quest.current_progress = tech_upgrades as f64;
            }
            QuestId::PenroseResonance => {
                if penrose_active {
                    quest.current_progress = 1.0;
                }
            }
            QuestId::PlanetaryDestruction => {
                quest.current_progress = terrestrial_planets as f64;
            }
            QuestId::TheBigCrunch => {
                quest.current_progress = total_collapses as f64;
            }
            QuestId::CompletedAll => {
                quest.current_progress = 1.0;
            }
        }

        if quest.current_progress >= quest.target_progress {
            quest.is_completed = true;
            let reward = quest.reward_mass;
            self.current_quest_index += 1;
            Some(reward)
        } else {
            None
        }
    }
}

pub fn default_quests() -> Vec<Quest> {
    vec![
        Quest::new(
            QuestId::AwakenSingularity,
            "1. AWAKEN THE SINGULARITY",
            "Click on the Black Hole 3 times to pulse spacetime.",
            3.0,
            MassiveNumber::from_f64(5.0),
            0,
        ),
        Quest::new(
            QuestId::FirstAccretion,
            "2. GATHER INITIAL MASS",
            "Reach 10 kg of mass to unlock the Matter Streams.",
            10.0,
            MassiveNumber::from_f64(10.0),
            0,
        ),
        Quest::new(
            QuestId::QuantumGenerator,
            "3. AUTOMATED ACCRETION",
            "Purchase 1x Quantum Gas & Photons in the Matter tab.",
            1.0,
            MassiveNumber::from_f64(20.0),
            0,
        ),
        Quest::new(
            QuestId::CriticalDensity,
            "4. CRITICAL MASS",
            "Accumulate 100 kg of mass to destabilize local space.",
            100.0,
            MassiveNumber::from_f64(50.0),
            0,
        ),
        Quest::new(
            QuestId::TechnologicalSingularity,
            "5. SINGULARITY TECHNOLOGY",
            "Purchase 1x Upgrade in the Tech tab.",
            1.0,
            MassiveNumber::from_f64(250.0),
            0,
        ),
        Quest::new(
            QuestId::PenroseResonance,
            "6. HARNESS THE ERGOSPHERE",
            "Click during an Accretion Flash to trigger Penrose Boost!",
            1.0,
            MassiveNumber::from_f64(1_000.0),
            0,
        ),
        Quest::new(
            QuestId::PlanetaryDestruction,
            "7. TERRESTRIAL DEVOURER",
            "Unlock and purchase 1x Terrestrial Planet.",
            1.0,
            MassiveNumber::from_f64(500_000.0),
            0,
        ),
        Quest::new(
            QuestId::TheBigCrunch,
            "8. THE POINT OF NO RETURN",
            "Reach 1,000,000 t mass and trigger The Big Crunch.",
            1.0,
            MassiveNumber::ZERO,
            10,
        ),
        Quest::new(
            QuestId::CompletedAll,
            "★ MASTER OF THE DARK RIFT ★",
            "The universe bows to your singularity.",
            1.0,
            MassiveNumber::ZERO,
            0,
        ),
    ]
}
