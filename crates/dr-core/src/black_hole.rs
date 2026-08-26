use crate::math::MassiveNumber;
use serde::{Deserialize, Serialize};

/// The physical and state properties of the Black Hole at the center of the Dark Rift.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlackHole {
    /// Total mass swallowed in kilograms.
    pub mass: MassiveNumber,
    /// Passive mass consumption rate (kg / sec).
    pub mass_rate: MassiveNumber,
    /// Base click / gravitational pulse mass gain.
    pub pulse_mass: MassiveNumber,
    /// Dimensionless spin parameter a* in [0.0, 0.998].
    pub spin: f32,
    /// Visual Schwarzschild radius in simulation viewport units (0.05 to 0.45).
    pub visual_radius: f32,
    /// Flash intensity (0.0 to 1.0) when consuming large matter chunks or pulsing.
    pub accretion_flash: f32,
    /// Active Penrose mechanism multiplier (1.0 = normal, 2.0+ during timed boost).
    pub penrose_multiplier: f32,
    /// Time remaining on active Penrose boost (seconds).
    pub penrose_timer: f32,
    /// Angle of the accretion disk spin in radians.
    pub disk_angle: f32,
    /// Total lifetime mass consumed across all collapses.
    pub lifetime_mass: MassiveNumber,
    /// Total gravity pulses triggered by player.
    pub total_pulses: u64,
}

impl Default for BlackHole {
    fn default() -> Self {
        Self::new()
    }
}

impl BlackHole {
    pub fn new() -> Self {
        Self {
            mass: MassiveNumber::from_f64(10.0), // Start with 10 kg
            mass_rate: MassiveNumber::ZERO,
            pulse_mass: MassiveNumber::from_f64(1.0),
            spin: 0.1,
            visual_radius: 0.12,
            accretion_flash: 0.0,
            penrose_multiplier: 1.0,
            penrose_timer: 0.0,
            disk_angle: 0.0,
            lifetime_mass: MassiveNumber::from_f64(10.0),
            total_pulses: 0,
        }
    }

    /// Computes physical Schwarzschild radius in meters: Rs = 2 * G * M / c^2.
    /// (G = 6.6743e-11, c = 2.9979e8 => 2G/c^2 ≈ 1.485e-27 m/kg).
    pub fn physical_schwarzschild_radius_m(&self) -> MassiveNumber {
        self.mass.mul_f64(1.4852e-27)
    }

    /// Calculates Hawking Temperature in Kelvin: T_H = (hbar * c^3) / (8 * pi * G * M * k_B).
    /// Constant ≈ 1.227e23 K * kg / M.
    pub fn hawking_temperature_k(&self) -> MassiveNumber {
        if self.mass.is_zero() {
            MassiveNumber::ZERO
        } else {
            MassiveNumber::from_f64(1.227e23) / self.mass
        }
    }

    /// Update black hole state each tick.
    pub fn update(&mut self, dt: f32, total_dps: MassiveNumber) {
        self.mass_rate = total_dps;

        // Apply passive mass growth
        if !total_dps.is_zero() {
            let gained = total_dps.mul_f64(dt as f64) * MassiveNumber::from_f64(self.penrose_multiplier as f64);
            self.mass += gained;
            self.lifetime_mass += gained;
        }

        // Accretion disk rotation speed scales with spin and accretion rate
        let rot_speed = (0.5 + self.spin * 2.0) * (1.0 + (self.penrose_multiplier - 1.0) * 0.5);
        self.disk_angle = (self.disk_angle + rot_speed * dt) % (std::f32::consts::TAU);

        // Visual radius scales smoothly logarithmically from 0.08 to 0.40
        // (10^0 kg -> ~0.08, 10^30 kg (1 M☉) -> ~0.22, 10^50 kg -> ~0.38)
        let log_mass = if self.mass.exponent <= 0 {
            0.0
        } else {
            self.mass.exponent as f32 + self.mass.mantissa.log10() as f32
        };
        let target_radius = (0.08 + (log_mass / 60.0).clamp(0.0, 1.0) * 0.30).clamp(0.08, 0.45);
        self.visual_radius += (target_radius - self.visual_radius) * (1.0 - libm::expf(-4.0 * dt));

        // Decay accretion flash
        if self.accretion_flash > 0.0 {
            self.accretion_flash = (self.accretion_flash - dt * 3.0).max(0.0);
        }

        // Decay Penrose boost timer
        if self.penrose_timer > 0.0 {
            self.penrose_timer = (self.penrose_timer - dt).max(0.0);
            if self.penrose_timer == 0.0 {
                self.penrose_multiplier = 1.0;
            }
        }
    }

    /// Trigger a gravitational pulse (click).
    pub fn trigger_pulse(&mut self, click_multiplier: MassiveNumber) -> MassiveNumber {
        self.total_pulses += 1;
        let gained = self.pulse_mass * click_multiplier * MassiveNumber::from_f64(self.penrose_multiplier as f64);
        self.mass += gained;
        self.lifetime_mass += gained;
        self.accretion_flash = 1.0;
        gained
    }

    /// Activate Penrose Boost (called when player taps/pulses in sync with disk alignment).
    pub fn activate_penrose_boost(&mut self, duration: f32, multiplier: f32) {
        self.penrose_timer = duration;
        self.penrose_multiplier = multiplier;
        self.accretion_flash = 1.0;
    }
}
