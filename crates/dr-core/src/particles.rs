use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

pub const MAX_PARTICLES: usize = 4096;

/// GPU-compatible particle representation, formatted for instanced vertex expansion.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Serialize, Deserialize)]
pub struct ParticleGpu {
    pub pos: [f32; 2],
    pub vel: [f32; 2],
    pub color: [f32; 4],
    pub size: f32,
    pub spaghettification: f32,
    pub tier: u32,
    pub _pad: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParticleSim {
    pub particles: Vec<ParticleGpu>,
    pub active_count: usize,
    rng_state: u64,
}

impl Default for ParticleSim {
    fn default() -> Self {
        Self::new()
    }
}

impl ParticleSim {
    pub fn new() -> Self {
        let mut sim = Self {
            particles: Vec::with_capacity(MAX_PARTICLES),
            active_count: 12, // Start with calm, gentle sparse particle field for Stage 0
            rng_state: 0x9e3779b97f4a7c15,
        };
        sim.init_particles();
        sim
    }

    #[inline]
    fn next_f32(&mut self) -> f32 {
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 7;
        self.rng_state ^= self.rng_state << 17;
        ((self.rng_state & 0x00ffffff) as f32) / (16777216.0)
    }

    pub fn init_particles(&mut self) {
        self.particles.clear();
        for i in 0..MAX_PARTICLES {
            let p = self.spawn_particle(i as u32);
            self.particles.push(p);
        }
    }

    pub fn spawn_particle(&mut self, idx: u32) -> ParticleGpu {
        let angle = self.next_f32() * std::f32::consts::TAU;
        let dist = 0.25 + self.next_f32() * 0.65;
        let x = dist * libm::cosf(angle);
        let y = dist * libm::sinf(angle);

        // Tangential orbital velocity: v = sqrt(mu / r) perpendicular to radius vector
        let speed = (0.30 / (dist + 0.05).sqrt()) * (0.8 + self.next_f32() * 0.4);
        let vx = -libm::sinf(angle) * speed;
        let vy = libm::cosf(angle) * speed;

        let tier = (idx % 11) as u32;
        let color = match tier {
            0 => [0.2, 0.8, 1.0, 0.85],  // Quantum cyan
            1 => [0.4, 0.9, 0.9, 0.85],  // Dust ice
            2 => [0.9, 0.7, 0.4, 0.9],   // Asteroid gold/rock
            3 => [0.8, 0.5, 0.3, 0.9],   // Moon rust
            4 => [0.2, 0.9, 0.5, 0.9],   // Earth emerald
            5 => [1.0, 0.6, 0.2, 0.95],  // Jupiter amber
            6 => [1.0, 0.3, 0.1, 1.0],   // Stellar red/orange
            7 => [0.3, 0.4, 1.0, 1.0],   // Blue hypergiant
            8 => [0.8, 0.2, 1.0, 1.0],   // Neutron purple
            9 => [1.0, 0.1, 0.6, 1.0],   // Galactic magenta
            _ => [1.0, 1.0, 1.0, 1.0],   // Void rift white
        };

        let size = (0.006 + self.next_f32() * 0.008) * (1.0 + (tier as f32) * 0.15);

        ParticleGpu {
            pos: [x, y],
            vel: [vx, vy],
            color,
            size,
            spaghettification: 1.0,
            tier,
            _pad: 0.0,
        }
    }

    /// Update all active particles. Returns the count of particles swallowed this tick.
    pub fn update(
        &mut self,
        dt: f32,
        visual_rs: f32,
        spin: f32,
        pulse_active: bool,
    ) -> u32 {
        let mut swallowed = 0;
        let count = self.active_count.min(self.particles.len());

        for i in 0..count {
            let (mut x, mut y, mut vx, mut vy, size);
            {
                let p = &self.particles[i];
                x = p.pos[0];
                y = p.pos[1];
                vx = p.vel[0];
                vy = p.vel[1];
                size = p.size;
            }

            let r2 = x * x + y * y;
            let r = r2.sqrt().max(0.001);

            // Inward gravity pull
            let gravity_strength = (visual_rs * 1.5) / (r2 + 0.01);
            let mut ax = -x / r * gravity_strength;
            let mut ay = -y / r * gravity_strength;

            // Frame dragging (adds tangential push in spin direction)
            let frame_drag = (spin * visual_rs) / (r2 * r + 0.01);
            ax += -y / r * frame_drag;
            ay += x / r * frame_drag;

            // Pulse impulse
            if pulse_active {
                ax *= 5.0;
                ay *= 5.0;
            }

            // Integrate velocity & position
            vx += ax * dt;
            vy += ay * dt;

            // Drag
            let drag = 0.08 * dt;
            vx *= 1.0 - drag;
            vy *= 1.0 - drag;

            x += vx * dt;
            y += vy * dt;

            let tidal_factor = (visual_rs * 2.0 / r).clamp(1.0, 5.0);

            // Check if crossed Event Horizon or out of bounds
            if r <= visual_rs * 0.95 {
                swallowed += 1;
                let new_p = self.spawn_particle(i as u32);
                self.particles[i] = new_p;
            } else if r > 1.3 {
                let new_p = self.spawn_particle(i as u32);
                self.particles[i] = new_p;
            } else {
                let p = &mut self.particles[i];
                p.pos[0] = x;
                p.pos[1] = y;
                p.vel[0] = vx;
                p.vel[1] = vy;
                p.size = size;
                p.spaghettification = tidal_factor;
            }
        }

        swallowed
    }

    /// Set active particle count based on tier progression.
    pub fn set_active_count(&mut self, count: usize) {
        self.active_count = count.clamp(8, MAX_PARTICLES);
    }
}
