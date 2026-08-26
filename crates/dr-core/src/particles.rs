use crate::objects::{FloatingObjectPool, ObjectType};
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

pub const MAX_PARTICLES: usize = 512;

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
            active_count: 32,
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
        for _ in 0..MAX_PARTICLES {
            self.particles.push(ParticleGpu {
                pos: [0.0, 0.0],
                vel: [0.0, 0.0],
                color: [0.0, 0.0, 0.0, 0.0],
                size: 0.0,
                spaghettification: 1.0,
                tier: 0,
                _pad: 0.0,
            });
        }
    }

    /// Synchronize state from FloatingObjectPool into the GPU vertex stream buffer.
    pub fn sync_from_objects(
        &mut self,
        objects: &FloatingObjectPool,
        _visual_rs: f32,
    ) {
        let mut count = 0;

        for obj in objects.objects.iter() {
            if !obj.is_active {
                continue;
            }

            if count < MAX_PARTICLES {
                let (base_size, color_boost) = match obj.obj_type {
                    ObjectType::QuantumPebble => (0.022, 1.2),
                    ObjectType::DenseAsteroid => (0.042, 1.4),
                    ObjectType::AntimatterShard => (0.035, 2.0),
                    ObjectType::ProtoMoon => (0.085, 1.6),
                };

                let stretch = if obj.is_sucked { 3.5 } else { 1.0 };

                self.particles[count] = ParticleGpu {
                    pos: obj.pos,
                    vel: obj.vel,
                    color: [
                        obj.color[0] * color_boost,
                        obj.color[1] * color_boost,
                        obj.color[2] * color_boost,
                        obj.color[3],
                    ],
                    size: base_size,
                    spaghettification: stretch,
                    tier: match obj.obj_type {
                        ObjectType::QuantumPebble => 1,
                        ObjectType::DenseAsteroid => 2,
                        ObjectType::AntimatterShard => 8,
                        ObjectType::ProtoMoon => 4,
                    },
                    _pad: 0.0,
                };
                count += 1;
            }
        }

        self.active_count = count.max(1);
    }
}
