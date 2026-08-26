use crate::math::MassiveNumber;
use serde::{Deserialize, Serialize};

pub const MAX_FLOATING_OBJECTS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectType {
    QuantumPebble,
    DenseAsteroid,
    AntimatterShard,
    ProtoMoon,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct FloatingObject {
    pub id: u32,
    pub pos: [f32; 2],
    pub vel: [f32; 2],
    pub mass: f64,
    pub radius: f32,
    pub obj_type: ObjectType,
    pub hp: u32,
    pub max_hp: u32,
    pub angle: f32,
    pub angular_vel: f32,
    pub is_sucked: bool,
    pub is_active: bool,
    pub color: [f32; 4],
}

impl Default for FloatingObject {
    fn default() -> Self {
        Self {
            id: 0,
            pos: [0.0, 0.0],
            vel: [0.0, 0.0],
            mass: 1.0,
            radius: 0.035,
            obj_type: ObjectType::QuantumPebble,
            hp: 1,
            max_hp: 1,
            angle: 0.0,
            angular_vel: 0.0,
            is_sucked: false,
            is_active: false,
            color: [0.7, 0.8, 0.9, 1.0],
        }
    }
}

pub struct ConsumedEvent {
    pub mass: MassiveNumber,
    pub obj_type: ObjectType,
    pub pos: [f32; 2],
    pub is_antimatter: bool,
}

#[derive(Clone, Debug)]
pub struct FloatingObjectPool {
    
    pub objects: [FloatingObject; MAX_FLOATING_OBJECTS],
    pub next_id: u32,
    pub spawn_timer: f32,
    pub shockwave_timer: f32,
    pub antimatter_active_timer: f32,
    pub active_count: usize,
    rng_state: u64,
}

impl Default for FloatingObjectPool {
    fn default() -> Self {
        Self::new()
    }
}

impl FloatingObjectPool {
    pub fn new() -> Self {
        let mut pool = Self {
            objects: [FloatingObject::default(); MAX_FLOATING_OBJECTS],
            next_id: 1,
            spawn_timer: 0.0,
            shockwave_timer: 0.0,
            antimatter_active_timer: 0.0,
            active_count: 0,
            rng_state: 0x853c49e6748fea9b,
        };

        // Spawn initial 4 starter pebbles for immediate Phase 1 gameplay
        for i in 0..4 {
            let angle = (i as f32) * (std::f32::consts::TAU / 4.0) + 0.4;
            let dist = 0.38 + (i as f32) * 0.05;
            pool.spawn_at(
                ObjectType::QuantumPebble,
                [angle.cos() * dist, angle.sin() * dist],
                [-angle.sin() * 0.04, angle.cos() * 0.04],
                1.0,
            );
        }

        pool
    }

    #[inline]
    fn next_f32(&mut self) -> f32 {
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 7;
        self.rng_state ^= self.rng_state << 17;
        ((self.rng_state & 0x00ffffff) as f32) / (16777216.0)
    }

    pub fn spawn_at(
        &mut self,
        obj_type: ObjectType,
        pos: [f32; 2],
        vel: [f32; 2],
        mass_val: f64,
    ) -> Option<usize> {
        let slot = self.objects.iter().position(|o| !o.is_active)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        let (radius, hp, color) = match obj_type {
            ObjectType::QuantumPebble => (0.032, 1, [0.65, 0.85, 1.0, 1.0]),
            ObjectType::DenseAsteroid => (0.058, 2, [1.0, 0.65, 0.25, 1.0]),
            ObjectType::AntimatterShard => (0.042, 1, [0.95, 0.2, 0.95, 1.0]),
            ObjectType::ProtoMoon => (0.110, 8, [0.85, 0.95, 1.0, 1.0]),
        };

        let angle = self.next_f32() * 6.28;
        let angular_vel = (self.next_f32() - 0.5) * 2.0;

        self.objects[slot] = FloatingObject {
            id,
            pos,
            vel,
            mass: mass_val,
            radius,
            obj_type,
            hp,
            max_hp: hp,
            angle,
            angular_vel,
            is_sucked: false,
            is_active: true,
            color,
        };

        self.active_count += 1;
        Some(slot)
    }

    pub fn tick(
        &mut self,
        dt: f32,
        current_game_mass: f64,
        siphon_dps: f64,
        black_hole_radius: f32,
        mut on_consumed: impl FnMut(ConsumedEvent),
    ) {
        if self.antimatter_active_timer > 0.0 {
            self.antimatter_active_timer = (self.antimatter_active_timer - dt).max(0.0);
        }
        if self.shockwave_timer > 0.0 {
            self.shockwave_timer = (self.shockwave_timer - dt).max(0.0);
        }

        // 1. Procedural Spawner balanced by Game Phase
        self.spawn_timer += dt;
        let spawn_interval = if current_game_mass < 10.0 {
            2.2 // Phase 1: Frequent pebbles
        } else if current_game_mass < 50.0 {
            1.8 // Phase 2: Steady flow
        } else if current_game_mass < 250.0 {
            1.4 // Phase 3: Asteroids & Antimatter
        } else {
            1.2 // Phase 4: Dense cosmic influx
        };

        let max_active = if current_game_mass < 10.0 {
            6
        } else if current_game_mass < 50.0 {
            10
        } else if current_game_mass < 250.0 {
            16
        } else {
            24
        };

        if self.spawn_timer >= spawn_interval && self.active_count < max_active {
            self.spawn_timer = 0.0;
            self.spawn_procedural(current_game_mass);
        }

        // 2. Physics & Gravitational Pull
        let event_horizon = black_hole_radius.max(0.06);

        for obj in self.objects.iter_mut() {
            if !obj.is_active {
                continue;
            }

            obj.angle += obj.angular_vel * dt;

            let dx = -obj.pos[0];
            let dy = -obj.pos[1];
            let dist_sq = dx * dx + dy * dy;
            let dist = dist_sq.sqrt().max(0.001);

            let dir_x = dx / dist;
            let dir_y = dy / dist;

            // Check if captured by passive Siphon or Antimatter Shockwave
            if siphon_dps > 0.0 && dist < (0.28 + (siphon_dps as f32 * 0.02).min(0.35)) {
                obj.is_sucked = true;
            }
            if self.shockwave_timer > 0.0 {
                obj.is_sucked = true;
            }

            if obj.is_sucked {
                // High-speed plunge directly into event horizon
                let suck_speed = 0.75 + (1.0 / dist).min(3.0);
                obj.pos[0] += dir_x * suck_speed * dt;
                obj.pos[1] += dir_y * suck_speed * dt;
            } else {
                // Keplerian orbit + gentle gravitational drift
                let gravity_force = (0.015 / (dist_sq + 0.05)).min(0.6);
                obj.vel[0] += dir_x * gravity_force * dt;
                obj.vel[1] += dir_y * gravity_force * dt;

                // Tangential orbital velocity dampening
                obj.pos[0] += obj.vel[0] * dt;
                obj.pos[1] += obj.vel[1] * dt;
            }

            // 3. Collision with Event Horizon
            if dist <= event_horizon {
                let is_am = obj.obj_type == ObjectType::AntimatterShard;
                let mass_num = MassiveNumber::from_f64(obj.mass);
                let pos = obj.pos;

                obj.is_active = false;
                self.active_count = self.active_count.saturating_sub(1);

                if is_am {
                    self.shockwave_timer = 2.5;
                    self.antimatter_active_timer = 15.0;
                }

                on_consumed(ConsumedEvent {
                    mass: mass_num,
                    obj_type: obj.obj_type,
                    pos,
                    is_antimatter: is_am,
                });
            }
        }
    }

    fn spawn_procedural(&mut self, current_mass: f64) {
        let spawn_angle = self.next_f32() * std::f32::consts::TAU;
        let spawn_dist = 0.55 + self.next_f32() * 0.15;
        let pos = [spawn_angle.cos() * spawn_dist, spawn_angle.sin() * spawn_dist];

        // Tangential orbital launch vector
        let speed = 0.03 + self.next_f32() * 0.03;
        let vel = [-spawn_angle.sin() * speed, spawn_angle.cos() * speed];

        let roll = self.next_f32();

        if current_mass >= 250.0 && roll < 0.15 && !self.has_proto_moon() {
            // Phase 4: Proto-Moon
            self.spawn_at(ObjectType::ProtoMoon, pos, [vel[0] * 0.5, vel[1] * 0.5], 250.0);
        } else if current_mass >= 50.0 && roll < 0.25 {
            // Phase 3: Antimatter Shard
            self.spawn_at(ObjectType::AntimatterShard, pos, [vel[0] * 2.0, vel[1] * 2.0], 50.0);
        } else if current_mass >= 25.0 && roll < 0.55 {
            // Phase 3: Dense Asteroid
            self.spawn_at(ObjectType::DenseAsteroid, pos, vel, 15.0);
        } else {
            // Phase 1 & 2: Quantum Pebble
            let pebble_mass = 0.5 + (current_mass * 0.02).min(5.0);
            self.spawn_at(ObjectType::QuantumPebble, pos, vel, pebble_mass);
        }
    }

    pub fn has_proto_moon(&self) -> bool {
        self.objects.iter().any(|o| o.is_active && o.obj_type == ObjectType::ProtoMoon)
    }

    pub fn hit_test(
        &mut self,
        click_screen_pos: [f32; 2],
        bh_center_screen: [f32; 2],
        aspect: f32,
        screen_height: f32,
    ) -> Option<(u32, ObjectType, bool)> {
        // Convert screen coordinates to normalized shader offset space
        let dx_screen = click_screen_pos[0] - bh_center_screen[0];
        let dy_screen = click_screen_pos[1] - bh_center_screen[1];

        let click_uv_x = (dx_screen / screen_height) * 2.0 * aspect;
        let click_uv_y = (dy_screen / screen_height) * 2.0;

        for obj in self.objects.iter_mut() {
            if !obj.is_active {
                continue;
            }

            let ox = obj.pos[0] * 2.0 * aspect;
            let oy = obj.pos[1] * 2.0;

            let dist = ((click_uv_x - ox).powi(2) + (click_uv_y - oy).powi(2)).sqrt();
            let hit_radius = (obj.radius * 2.0 * aspect).max(0.08);

            if dist <= hit_radius {
                obj.hp = obj.hp.saturating_sub(1);
                let is_destroyed = obj.hp == 0;
                if is_destroyed {
                    obj.is_sucked = true;
                }
                return Some((obj.id, obj.obj_type, is_destroyed));
            }
        }

        None
    }
}
