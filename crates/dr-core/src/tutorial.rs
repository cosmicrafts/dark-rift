use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TutorialStep {
    AwakenSingularity,   // Step 1: Click the black hole to reach 10 kg
    BuyFirstGenerator,   // Step 2: Buy "Quantum Gas & Photons" on the right panel
    ExplainPassive,      // Step 3: Explain passive accretion rate (+1 kg/s)
    Completed,           // Step 4: Free play & milestone quests
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TutorialState {
    pub step: TutorialStep,
    pub clicks_done: u32,
    pub timer: f32,
}

impl Default for TutorialState {
    fn default() -> Self {
        Self::new()
    }
}

impl TutorialState {
    pub fn new() -> Self {
        Self {
            step: TutorialStep::AwakenSingularity,
            clicks_done: 0,
            timer: 0.0,
        }
    }

    pub fn is_completed(&self) -> bool {
        self.step == TutorialStep::Completed
    }

    pub fn on_click(&mut self, current_mass_kg: f64) {
        self.clicks_done += 1;
        if self.step == TutorialStep::AwakenSingularity && current_mass_kg >= 10.0 {
            self.step = TutorialStep::BuyFirstGenerator;
        }
    }

    pub fn on_generator_bought(&mut self) {
        if self.step == TutorialStep::BuyFirstGenerator {
            self.step = TutorialStep::ExplainPassive;
            self.timer = 0.0;
        }
    }

    pub fn update(&mut self, dt: f32, current_mass_kg: f64, first_tier_count: u64) {
        self.timer += dt;

        match self.step {
            TutorialStep::AwakenSingularity => {
                if current_mass_kg >= 10.0 {
                    self.step = TutorialStep::BuyFirstGenerator;
                }
            }
            TutorialStep::BuyFirstGenerator => {
                if first_tier_count > 0 {
                    self.step = TutorialStep::ExplainPassive;
                    self.timer = 0.0;
                }
            }
            TutorialStep::ExplainPassive => {
                // Auto dismiss after 6 seconds or on manual button click
                if self.timer >= 6.0 {
                    self.step = TutorialStep::Completed;
                }
            }
            TutorialStep::Completed => {}
        }
    }

    pub fn dismiss_explanation(&mut self) {
        if self.step == TutorialStep::ExplainPassive {
            self.step = TutorialStep::Completed;
        }
    }
}
