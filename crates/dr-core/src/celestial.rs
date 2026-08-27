use crate::math::MassiveNumber;
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

pub const MAX_CELESTIAL_BODIES: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CelestialType {
    QuantumGas,      // Cyan/azure quantum gas cloud
    MicroMeteor,     // Golden molten meteor
    DenseAsteroid,   // Heavy iron-nickel asteroid
    AntimatterShard, // High-energy violet anomaly
    ProtoMoon,       // Massive rocky planetoid
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Serialize, Deserialize)]
pub struct CelestialGpu {
    pub pos: [f32; 2],
    pub vel: [f32; 2],
    pub color: [f32; 4],
    pub size: f32,
    pub spaghettification: f32,
    pub tier: u32,
    pub _pad: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CelestialBody {
    pub id: u32,
    pub body_type: CelestialType,
    pub pos: [f32; 2],
    pub vel: [f32; 2],
    pub mass: f64,
    pub visual_size: f32,
    pub color: [f32; 4],
    pub hp: u32,
    pub max_hp: u32,
    pub is_captured: bool,
    pub is_active: bool,
    pub trail_timer: f32,
}

impl Default for CelestialBody {
    fn default() -> Self {
        Self {
            id: 0,
            body_type: CelestialType::QuantumGas,
            pos: [0.0, 0.0],
            vel: [0.0, 0.0],
            mass: 1.0,
            visual_size: 0.02,
            color: [0.3, 0.8, 1.0, 0.85],
            hp: 1,
            max_hp: 1,
            is_captured: false,
            is_active: false,
            trail_timer: 0.0,
        }
    }
}

pub struct AccretionEvent {
    pub mass_gained: MassiveNumber,
    pub body_type: CelestialType,
    pub pos: [f32; 2],
    pub is_antimatter: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CelestialField {
    pub bodies: Vec<CelestialBody>,
    pub gpu_instances: Vec<CelestialGpu>,
    pub active_count: usize,
    pub next_id: u32,
    pub spawn_timer: f32,
    pub shockwave_timer: f32,
    pub shockwave_radius: f32,
    rng_state: u64,
}

impl Default for CelestialField {
    fn default() -> Self {
        Self::new()
    }
}

impl CelestialField {
    pub fn new() -> Self {
        let mut field = Self {
            bodies: vec![CelestialBody::default(); MAX_CELESTIAL_BODIES],
            gpu_instances: Vec::with_capacity(MAX_CELESTIAL_BODIES),
            active_count: 0,
            next_id: 1,
            spawn_timer: 0.0,
            shockwave_timer: 0.0,
            shockwave_radius: 0.0,
            rng_state: 0x2545f4914f6cdd1d,
        };

        for _ in 0..MAX_CELESTIAL_BODIES {
            field.gpu_instances.push(CelestialGpu {
                pos: [0.0, 0.0],
                vel: [0.0, 0.0],
                color: [0.0, 0.0, 0.0, 0.0],
                size: 0.0,
                spaghettification: 1.0,
                tier: 0,
                _pad: 0.0,
            });
        }

        // Spawn initial 3 graceful quantum gas clouds drifting in orbit
        for i in 0..3 {
            let angle = (i as f32) * (std::f32::consts::TAU / 3.0) + 0.5;
            let dist = 0.38 + (i as f32) * 0.08;
            field.spawn_exact(
                CelestialType::QuantumGas,
                [angle.cos() * dist, angle.sin() * dist],
                [-angle.sin() * 0.05, angle.cos() * 0.05],
                1.0,
            );
        }

        field
    }

    #[inline]
    fn next_f32(&mut self) -> f32 {
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 7;
        self.rng_state ^= self.rng_state << 17;
        ((self.rng_state & 0x00ffffff) as f32) / (16777216.0)
    }

    pub fn spawn_exact(
        &mut self,
        body_type: CelestialType,
        pos: [f32; 2],
        vel: [f32; 2],
        mass_val: f64,
    ) -> Option<usize> {
        let slot = self.bodies.iter().position(|b| !b.is_active)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        let (visual_size, hp, color) = match body_type {
            CelestialType::QuantumGas => (0.016, 1, [0.25, 0.85, 1.0, 0.80]),
            CelestialType::MicroMeteor => (0.024, 1, [1.0, 0.70, 0.30, 0.90]),
            CelestialType::DenseAsteroid => (0.038, 2, [0.95, 0.45, 0.20, 0.95]),
            CelestialType::AntimatterShard => (0.030, 1, [0.95, 0.20, 0.95, 1.00]),
            CelestialType::ProtoMoon => (0.075, 4, [0.75, 0.90, 1.0, 1.00]),
        };

        self.bodies[slot] = CelestialBody {
            id,
            body_type,
            pos,
            vel,
            mass: mass_val,
            visual_size,
            color,
            hp,
            max_hp: hp,
            is_captured: false,
            is_active: true,
            trail_timer: 0.0,
        };

        self.active_count += 1;
        Some(slot)
    }

    pub fn spawn_procedural(&mut self, current_mass_kg: f64) {
        let angle = self.next_f32() * std::f32::consts::TAU;
        let dist = 0.55 + self.next_f32() * 0.15;
        let pos = [angle.cos() * dist, angle.sin() * dist];

        let orbital_speed = (0.035 / (dist + 0.1).sqrt()) * (0.85 + self.next_f32() * 0.3);
        let vel = [-angle.sin() * orbital_speed, angle.cos() * orbital_speed];

        let roll = self.next_f32();

        if current_mass_kg >= 250.0 && roll < 0.10 {
            self.spawn_exact(CelestialType::ProtoMoon, pos, [vel[0] * 0.5, vel[1] * 0.5], 500.0);
        } else if current_mass_kg >= 60.0 && roll < 0.16 {
            self.spawn_exact(CelestialType::AntimatterShard, pos, [vel[0] * 1.4, vel[1] * 1.4], 50.0);
        } else if current_mass_kg >= 25.0 && roll < 0.40 {
            self.spawn_exact(CelestialType::DenseAsteroid, pos, vel, 15.0);
        } else if current_mass_kg >= 10.0 && roll < 0.70 {
            self.spawn_exact(CelestialType::MicroMeteor, pos, vel, 4.0);
        } else {
            self.spawn_exact(CelestialType::QuantumGas, pos, vel, 1.0);
        }
    }

    pub fn trigger_gravitational_wave(&mut self) {
        self.shockwave_timer = 0.6;
        self.shockwave_radius = 0.1;
        for body in self.bodies.iter_mut() {
            if body.is_active {
                body.is_captured = true;
            }
        }
    }

    pub fn hit_test_at(&mut self, norm_pos: [f32; 2]) -> Option<(u32, CelestialType, bool)> {
        for body in self.bodies.iter_mut() {
            if !body.is_active {
                continue;
            }

            let dx = norm_pos[0] - body.pos[0];
            let dy = norm_pos[1] - body.pos[1];
            let dist = (dx * dx + dy * dy).sqrt();

            let hit_radius = body.visual_size * 2.2;
            if dist <= hit_radius {
                body.hp = body.hp.saturating_sub(1);
                body.is_captured = true;
                let destroyed = body.hp == 0;
                return Some((body.id, body.body_type, destroyed));
            }
        }
        None
    }

    pub fn tick(
        &mut self,
        dt: f32,
        current_mass_kg: f64,
        passive_dps: f64,
        schwarzschild_radius: f32,
        spin: f32,
        mut on_accretion: impl FnMut(AccretionEvent),
    ) {
        if self.shockwave_timer > 0.0 {
            self.shockwave_timer -= dt;
            self.shockwave_radius += 1.8 * dt;
        }

        // 1. Procedural Pacing
        self.spawn_timer += dt;
        let (interval, max_bodies) = if current_mass_kg < 10.0 {
            (2.5, 4) // Phase 1: Clean, sparse, focused
        } else if current_mass_kg < 50.0 {
            (1.8, 8)
        } else if current_mass_kg < 250.0 {
            (1.4, 16)
        } else {
            (1.0, 32)
        };

        if self.spawn_timer >= interval && self.active_count < max_bodies {
            self.spawn_timer = 0.0;
            self.spawn_procedural(current_mass_kg);
        }

        // 2. Physics & Accretion Loop
        let rs = schwarzschild_radius.max(0.06);

        for body in self.bodies.iter_mut() {
            if !body.is_active {
                continue;
            }

            let r2 = body.pos[0] * body.pos[0] + body.pos[1] * body.pos[1];
            let r = r2.sqrt().max(0.001);

            let dir_x = -body.pos[0] / r;
            let dir_y = -body.pos[1] / r;

            // Check if captured by passive accretion field or shockwave
            if passive_dps > 0.0 && r < (0.30 + (passive_dps as f32 * 0.015).min(0.25)) {
                body.is_captured = true;
            }

            if body.is_captured {
                // Relativistic spiral suction
                let suck_speed = 0.75 + (1.2 / r).min(3.0);
                body.pos[0] += dir_x * suck_speed * dt;
                body.pos[1] += dir_y * suck_speed * dt;

                // Kerr tangential drag acceleration
                let frame_drag = spin * 1.5 * dt;
                body.pos[0] += -dir_y * frame_drag;
                body.pos[1] += dir_x * frame_drag;
            } else {
                // Orbital Keplerian Gravity
                let grav = (0.018 / (r2 + 0.02)).min(0.35);
                body.vel[0] += dir_x * grav * dt;
                body.vel[1] += dir_y * grav * dt;

                // Frame dragging tangential push
                let drag = (spin * 0.008 / (r + 0.05)) * dt;
                body.vel[0] += -dir_y * drag;
                body.vel[1] += dir_x * drag;

                body.pos[0] += body.vel[0] * dt;
                body.pos[1] += body.vel[1] * dt;
            }

            // 3. Collision with Event Horizon (Rs)
            if r <= rs * 1.05 {
                let is_antimatter = body.body_type == CelestialType::AntimatterShard;
                let mass_num = MassiveNumber::from_f64(body.mass);
                let pos = body.pos;

                body.is_active = false;
                self.active_count = self.active_count.saturating_sub(1);

                on_accretion(AccretionEvent {
                    mass_gained: mass_num,
                    body_type: body.body_type,
                    pos,
                    is_antimatter,
                });
            }
        }

        // 3. Sync to GPU Buffer
        self.sync_gpu_instances(rs);
    }

    fn sync_gpu_instances(&mut self, rs: f32) {
        let mut count = 0;
        for body in self.bodies.iter() {
            if !body.is_active {
                continue;
            }

            if count < self.gpu_instances.len() {
                let r2 = body.pos[0] * body.pos[0] + body.pos[1] * body.pos[1];
                let r = r2.sqrt().max(0.001);
                let tidal_stretch = if body.is_captured {
                    (rs * 2.5 / r).clamp(1.5, 6.0)
                } else {
                    1.0
                };

                let tier = match body.body_type {
                    CelestialType::QuantumGas => 0,
                    CelestialType::MicroMeteor => 1,
                    CelestialType::DenseAsteroid => 2,
                    CelestialType::AntimatterShard => 6,
                    CelestialType::ProtoMoon => 4,
                };

                self.gpu_instances[count] = CelestialGpu {
                    pos: body.pos,
                    vel: body.vel,
                    color: body.color,
                    size: body.visual_size,
                    spaghettification: tidal_stretch,
                    tier,
                    _pad: 0.0,
                };
                count += 1;
            }
        }
        self.active_count = count;
    }
}
