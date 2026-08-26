// ============================================================================
// WARNING / ANTI-PATTERN FOR FUTURE AGENTS:
// NEVER inject bulk mass rewards into early quests that destroy the player's
// onboarding tempo. If a player is supposed to experience the gradual growth
// from 0 to 10 kg, injecting a +5 kg or +10 kg quest reward on the 3rd click
// completely skips the tutorial stage and breaks the game feel.
// Quests must only activate or scale rewards AFTER the player has mastered
// the basic loop.
// ============================================================================

use crate::math::MassiveNumber;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestId {
    FirstAccretionMilestone,
    QuantumExpansion,
    RelativisticSpeed,
    TechnologicalSingularity,
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
        _total_pulses: u64,
        current_mass: MassiveNumber,
        quantum_tiers: u64,
        tech_upgrades: u32,
        _penrose_active: bool,
        terrestrial_planets: u64,
        total_collapses: u64,
    ) -> Option<MassiveNumber> {
        let quest = self.current_quest_mut()?;
        if quest.is_completed {
            return None;
        }

        match quest.id {
            QuestId::FirstAccretionMilestone => {
                quest.current_progress = current_mass.to_f64().min(quest.target_progress);
            }
            QuestId::QuantumExpansion => {
                quest.current_progress = quantum_tiers as f64;
            }
            QuestId::RelativisticSpeed => {
                quest.current_progress = current_mass.to_f64().min(quest.target_progress);
            }
            QuestId::TechnologicalSingularity => {
                quest.current_progress = tech_upgrades as f64;
            }
            QuestId::PlanetaryDestruction => {
                quest.current_progress = terrestrial_planets as f64;
            }
            QuestId::TheBigCrunch => {
                quest.current_progress = total_collapses as f64;
            }
            QuestId::CompletedAll => {}
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
            QuestId::FirstAccretionMilestone,
            "1. EXPAND THE EVENT HORIZON",
            "Reach 50 kg of mass using passive matter generators.",
            50.0,
            MassiveNumber::from_f64(25.0),
            0,
        ),
        Quest::new(
            QuestId::QuantumExpansion,
            "2. QUANTUM HARVEST",
            "Own at least 5 Quantum Gas & Photons generators.",
            5.0,
            MassiveNumber::from_f64(50.0),
            0,
        ),
        Quest::new(
            QuestId::RelativisticSpeed,
            "3. RELATIVISTIC CRITICALITY",
            "Reach 150 kg mass to unlock Singularity Technologies.",
            150.0,
            MassiveNumber::from_f64(100.0),
            0,
        ),
        Quest::new(
            QuestId::TechnologicalSingularity,
            "4. TECHNOLOGICAL ADVANCE",
            "Purchase 3 upgrades in the Tech panel.",
            3.0,
            MassiveNumber::from_f64(500.0),
            0,
        ),
        Quest::new(
            QuestId::PlanetaryDestruction,
            "5. PLANETARY CONSUMPTION",
            "Own 1 Terrestrial Planet matter harvester.",
            1.0,
            MassiveNumber::from_f64(10_000.0),
            0,
        ),
        Quest::new(
            QuestId::TheBigCrunch,
            "6. COSMIC COLLAPSE",
            "Trigger the Big Crunch to unlock Primordial Dark Matter Shards.",
            1.0,
            MassiveNumber::ZERO,
            1,
        ),
    ]
}
