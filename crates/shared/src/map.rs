//! Star map generation using Poisson disk sampling + Delaunay triangulation.
//!
//! The map is a graph of StarSystems (nodes) connected by hyperspace lanes (edges).
//! This is the core data structure that replaces OpenFront's pixel-grid map.

use bevy::math::Vec2;
use rand::Rng;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};

use crate::config;

/// A unique identifier for a star system node.
pub type SystemId = u32;

/// A single star system on the map — equivalent to a "tile" in OpenFront,
/// but operating at node-level instead of pixel-level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StarSystem {
    pub id: SystemId,
    /// Position in 2D world space (for rendering and distance checks).
    pub position: Vec2,
    /// Procedurally generated name for this system.
    pub name: String,
    /// Who owns this system. `None` = unclaimed (TerraNullius equivalent).
    pub owner: Option<u32>,
    /// Ship garrison stationed at this system.
    pub ships: f32,
    /// IDs of adjacent systems connected by hyperspace lanes.
    pub connections: Vec<SystemId>,
}

/// The full star map — a graph of systems connected by lanes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StarMap {
    pub systems: Vec<StarSystem>,
    pub width: f32,
    pub height: f32,
}

impl StarMap {
    /// Generate a new star map with Poisson disk sampling for node placement
    /// and Delaunay-inspired neighbor linking.
    pub fn generate(seed: u64) -> Self {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let points = poisson_disk_sampling(
            config::MAP_WIDTH,
            config::MAP_HEIGHT,
            config::MIN_SYSTEM_DISTANCE,
            config::DEFAULT_SYSTEM_COUNT,
            &mut rng,
        );

        // Generate system names
        let names = generate_system_names(points.len(), &mut rng);

        // Build star systems
        let mut systems: Vec<StarSystem> = points
            .iter()
            .enumerate()
            .map(|(i, &pos)| StarSystem {
                id: i as u32,
                position: pos,
                name: names[i].clone(),
                owner: None,
                ships: 0.0,
                connections: Vec::new(),
            })
            .collect();

        // Connect systems: link each system to nearby systems within MAX_CONNECTION_DISTANCE.
        // This approximates a Delaunay triangulation without needing a full Delaunay library.
        let max_dist_sq =
            config::MAX_CONNECTION_DISTANCE * config::MAX_CONNECTION_DISTANCE;

        for i in 0..systems.len() {
            let pos_i = systems[i].position;
            for j in (i + 1)..systems.len() {
                let pos_j = systems[j].position;
                let dist_sq = pos_i.distance_squared(pos_j);
                if dist_sq <= max_dist_sq {
                    systems[i].connections.push(j as u32);
                    systems[j].connections.push(i as u32);
                }
            }
        }

        // Ensure every system has at least one connection (connect to nearest if isolated).
        for i in 0..systems.len() {
            if systems[i].connections.is_empty() {
                let pos_i = systems[i].position;
                let mut nearest = 0u32;
                let mut nearest_dist = f32::MAX;
                for j in 0..systems.len() {
                    if i == j {
                        continue;
                    }
                    let d = pos_i.distance_squared(systems[j].position);
                    if d < nearest_dist {
                        nearest_dist = d;
                        nearest = j as u32;
                    }
                }
                systems[i].connections.push(nearest);
                systems[nearest as usize].connections.push(i as u32);
            }
        }

        // Deduplicate connection lists.
        for sys in &mut systems {
            sys.connections.sort_unstable();
            sys.connections.dedup();
        }

        StarMap {
            systems,
            width: config::MAP_WIDTH,
            height: config::MAP_HEIGHT,
        }
    }

    /// Get a system by ID.
    pub fn system(&self, id: SystemId) -> &StarSystem {
        &self.systems[id as usize]
    }

    /// Get a mutable reference to a system by ID.
    pub fn system_mut(&mut self, id: SystemId) -> &mut StarSystem {
        &mut self.systems[id as usize]
    }

    /// Count total systems.
    pub fn system_count(&self) -> u32 {
        self.systems.len() as u32
    }

    /// Count systems owned by a specific player.
    pub fn systems_owned_by(&self, player_id: u32) -> u32 {
        self.systems
            .iter()
            .filter(|s| s.owner == Some(player_id))
            .count() as u32
    }

    /// Get all unclaimed system IDs.
    pub fn unclaimed_systems(&self) -> Vec<SystemId> {
        self.systems
            .iter()
            .filter(|s| s.owner.is_none())
            .map(|s| s.id)
            .collect()
    }

    /// Check if two systems are connected by a hyperspace lane.
    pub fn are_connected(&self, a: SystemId, b: SystemId) -> bool {
        self.systems[a as usize].connections.contains(&b)
    }

    /// Check if a player owns any system adjacent to the given system.
    pub fn player_has_adjacent(&self, system_id: SystemId, player_id: u32) -> bool {
        let sys = &self.systems[system_id as usize];
        sys.connections
            .iter()
            .any(|&conn| self.systems[conn as usize].owner == Some(player_id))
    }
}

/// Poisson disk sampling — generates well-distributed random points.
/// This ensures star systems aren't clumped together or too evenly spaced.
fn poisson_disk_sampling(
    width: f32,
    height: f32,
    min_dist: f32,
    target_count: u32,
    rng: &mut impl Rng,
) -> Vec<Vec2> {
    let mut points = Vec::new();
    let margin = min_dist; // Keep systems away from map edges.
    let max_attempts = target_count * 30;
    let mut attempts = 0u32;

    while (points.len() as u32) < target_count && attempts < max_attempts {
        attempts += 1;
        let candidate = Vec2::new(
            rng.gen_range(margin..(width - margin)),
            rng.gen_range(margin..(height - margin)),
        );

        // Check minimum distance from all existing points.
        let too_close = points
            .iter()
            .any(|p: &Vec2| p.distance(candidate) < min_dist);

        if !too_close {
            points.push(candidate);
        }
    }

    points
}

/// Generate unique star system names.
fn generate_system_names(count: usize, rng: &mut impl Rng) -> Vec<String> {
    let prefixes = [
        "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta",
        "Iota", "Kappa", "Lambda", "Sigma", "Tau", "Omega", "Nova", "Vega",
        "Rigel", "Altair", "Sirius", "Orion", "Draco", "Cygnus", "Lyra",
        "Hydra", "Corvus", "Vulpes", "Fornax", "Cetus", "Aquila", "Pyxis",
        "Volans", "Musca", "Ara", "Pavo", "Grus", "Indus", "Apus", "Norma",
        "Vela", "Crux", "Lupus", "Mensa", "Pictor", "Caelum", "Dorado",
        "Tucana", "Phoenix", "Eridanus", "Centauri", "Scorpii",
    ];

    let suffixes = [
        "Prime", "Major", "Minor", "Nexus", "Reach", "Drift", "Hollow",
        "Deep", "Gate", "Beacon", "Spire", "Haven", "Verge", "Rift",
        "Terminus", "Core", "Edge", "Expanse", "Bastion", "Forge",
    ];

    let greek_letters = [
        "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X",
        "XI", "XII", "XIII", "XIV", "XV", "XVI",
    ];

    let mut names = Vec::with_capacity(count);
    let mut used = std::collections::HashSet::new();

    for _ in 0..count {
        let name = loop {
            let style = rng.gen_range(0..3);
            let candidate = match style {
                0 => {
                    // "Alpha Nexus" style
                    let p = prefixes[rng.gen_range(0..prefixes.len())];
                    let s = suffixes[rng.gen_range(0..suffixes.len())];
                    format!("{p} {s}")
                }
                1 => {
                    // "Vega VII" style
                    let p = prefixes[rng.gen_range(0..prefixes.len())];
                    let g = greek_letters[rng.gen_range(0..greek_letters.len())];
                    format!("{p} {g}")
                }
                _ => {
                    // "DR-1742" style (short code)
                    let num = rng.gen_range(1000..9999);
                    format!("DR-{num}")
                }
            };
            if !used.contains(&candidate) {
                break candidate;
            }
        };
        used.insert(name.clone());
        names.push(name);
    }

    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_generation() {
        let map = StarMap::generate(42);
        // Should generate a reasonable number of systems.
        assert!(map.systems.len() > 50, "Too few systems: {}", map.systems.len());
        assert!(
            map.systems.len() <= config::DEFAULT_SYSTEM_COUNT as usize + 10,
            "Too many systems"
        );

        // Every system should have at least one connection.
        for sys in &map.systems {
            assert!(
                !sys.connections.is_empty(),
                "System {} has no connections",
                sys.id
            );
        }

        // All systems should be within map bounds.
        for sys in &map.systems {
            assert!(sys.position.x >= 0.0 && sys.position.x <= map.width);
            assert!(sys.position.y >= 0.0 && sys.position.y <= map.height);
        }
    }

    #[test]
    fn test_connectivity_check() {
        let map = StarMap::generate(42);
        // Pick first system and its first connection — should be bidirectional.
        let sys0 = &map.systems[0];
        if let Some(&neighbor) = sys0.connections.first() {
            assert!(map.are_connected(0, neighbor));
            assert!(map.are_connected(neighbor, 0));
        }
    }
}
