use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TutorialStep {
    ClickFloatingDebris, // Phase 1: Click 3 floating space rocks to suck them in
    ReachTenKg,          // Phase 1: Click debris until reaching 10 kg
    BuyFirstTier,        // Phase 2: Buy Gravitational Siphon on the right panel
    ExplainPassive,      // Phase 2: Explain passive suction field
    CatchAntimatter,     // Phase 3: Catch glowing Antimatter Shard
    Completed,           // Phase 4: Free play & Proto-Moon milestone
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TutorialState {
    pub step: TutorialStep,
    pub rocks_caught: u32,
    pub rocks_target: u32,
    pub antimatter_caught: bool,
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
            step: TutorialStep::ClickFloatingDebris,
            rocks_caught: 0,
            rocks_target: 3,
            antimatter_caught: false,
            timer: 0.0,
        }
    }

    pub fn is_completed(&self) -> bool {
        self.step == TutorialStep::Completed
    }

    pub fn on_rock_clicked(&mut self, is_antimatter: bool, current_mass_kg: f64) {
        match self.step {
            TutorialStep::ClickFloatingDebris => {
                self.rocks_caught += 1;
                if self.rocks_caught >= self.rocks_target {
                    self.step = TutorialStep::ReachTenKg;
                }
            }
            TutorialStep::ReachTenKg => {
                if current_mass_kg >= 10.0 {
                    self.step = TutorialStep::BuyFirstTier;
                }
            }
            TutorialStep::CatchAntimatter => {
                if is_antimatter {
                    self.antimatter_caught = true;
                    self.step = TutorialStep::Completed;
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
                if self.timer >= 6.0 {
                    self.step = if current_mass_kg < 50.0 {
                        TutorialStep::CatchAntimatter
                    } else {
                        TutorialStep::Completed
                    };
                }
            }
            TutorialStep::CatchAntimatter => {
                if current_mass_kg >= 150.0 {
                    self.step = TutorialStep::Completed;
                }
            }
            _ => {}
        }
    }

    pub fn dismiss_explanation(&mut self) {
        if self.step == TutorialStep::ExplainPassive {
            self.step = TutorialStep::CatchAntimatter;
        }
    }
}
