use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TutorialStep {
    PulseThreeTimes,     // Step 0: Click 3 times to awaken the void
    ReachTenKg,          // Step 1: Click until reaching 10 kg
    BuyFirstTier,        // Step 2: Click the highlighted "Buy Quantum Gas" button
    ExplainPassive,      // Step 3: Explain the passive accretion rate
    Completed,           // Step 4: Full free play
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TutorialState {
    pub step: TutorialStep,
    pub clicks_done: u32,
    pub clicks_target: u32,
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
            step: TutorialStep::PulseThreeTimes,
            clicks_done: 0,
            clicks_target: 3,
            timer: 0.0,
        }
    }

    pub fn is_completed(&self) -> bool {
        self.step == TutorialStep::Completed
    }

    pub fn on_click(&mut self, current_mass_kg: f64) {
        match self.step {
            TutorialStep::PulseThreeTimes => {
                self.clicks_done += 1;
                if self.clicks_done >= self.clicks_target {
                    self.step = TutorialStep::ReachTenKg;
                }
            }
            TutorialStep::ReachTenKg => {
                if current_mass_kg >= 10.0 {
                    self.step = TutorialStep::BuyFirstTier;
                }
            }
            _ => {}
        }
    }

    pub fn on_tier_bought(&mut self) {
        if self.step == TutorialStep::BuyFirstTier {
            self.step = TutorialStep::ExplainPassive;
            self.timer = 0.0;
        }
    }

    pub fn update(&mut self, dt: f32, current_mass_kg: f64, tier_count: u64) {
        self.timer += dt;

        match self.step {
            TutorialStep::ReachTenKg => {
                if current_mass_kg >= 10.0 {
                    self.step = TutorialStep::BuyFirstTier;
                }
            }
            TutorialStep::BuyFirstTier => {
                if tier_count > 0 {
                    self.step = TutorialStep::ExplainPassive;
                    self.timer = 0.0;
                }
            }
            TutorialStep::ExplainPassive => {
                // Auto-advance after 5 seconds or on dismiss
                if self.timer >= 5.0 {
                    self.step = TutorialStep::Completed;
                }
            }
            _ => {}
        }
    }

    pub fn dismiss_explanation(&mut self) {
        if self.step == TutorialStep::ExplainPassive {
            self.step = TutorialStep::Completed;
        }
    }
}
