//! Procedural faction name generator — galactic-themed funny names.
//!
//! Inspired by OpenFront's NationCreation.ts but with a space opera flavor.
//! Generates names like "Quantum Kraken Fleet" or "Cursed Nebula Cartel".

use rand::Rng;
use serde::{Deserialize, Serialize};

/// A faction is an AI-controlled entity that holds territory on the map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Faction {
    /// Unique ID for this faction (also used as player_id in the map).
    pub id: u32,
    /// Procedurally generated faction name.
    pub name: String,
    /// RGB color for rendering this faction's territory.
    pub color: [f32; 3],
}

/// Generate a list of unique factions with funny galactic names.
pub fn generate_factions(count: u32, start_id: u32, rng: &mut impl Rng) -> Vec<Faction> {
    let mut factions = Vec::new();
    let mut used_names = std::collections::HashSet::new();

    let colors = faction_color_palette();

    for i in 0..count {
        let name = loop {
            let candidate = generate_faction_name(rng);
            if !used_names.contains(&candidate) {
                break candidate;
            }
        };
        used_names.insert(name.clone());

        let color = colors[(i as usize) % colors.len()];
        factions.push(Faction {
            id: start_id + i,
            name,
            color,
        });
    }

    factions
}

/// Generate a single faction name from templates.
fn generate_faction_name(rng: &mut impl Rng) -> String {
    let style = rng.gen_range(0..4);
    match style {
        0 => {
            // "[Adjective] [Noun] [Suffix]" — e.g. "Quantum Kraken Fleet"
            let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
            let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
            let suffix = SUFFIXES[rng.gen_range(0..SUFFIXES.len())];
            format!("{adj} {noun} {suffix}")
        }
        1 => {
            // "[Prefix] of [Noun]" — e.g. "Order of the Void"
            let prefix = PREFIXES[rng.gen_range(0..PREFIXES.len())];
            let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
            format!("{prefix} of the {noun}")
        }
        2 => {
            // "[Adjective] [Suffix]" — e.g. "Feral Armada"
            let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
            let suffix = SUFFIXES[rng.gen_range(0..SUFFIXES.len())];
            format!("{adj} {suffix}")
        }
        _ => {
            // "[Noun] [Suffix]" — e.g. "Nebula Syndicate"
            let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
            let suffix = SUFFIXES[rng.gen_range(0..SUFFIXES.len())];
            format!("{noun} {suffix}")
        }
    }
}

static ADJECTIVES: &[&str] = &[
    "Void", "Rogue", "Cosmic", "Legendary", "Cursed", "Quantum",
    "Pirate", "Shadow", "Feral", "Sentient", "Chaotic", "Eternal",
    "Forbidden", "Radioactive", "Weaponized", "Haunted", "Blessed",
    "Bootleg", "Counterfeit", "Turbo", "Hyperdrive", "Warp-Speed",
    "Deep Space", "Galactic", "Interstellar", "Nebular", "Solar",
    "Lunar", "Astral", "Phantom", "Spectral", "Crimson", "Obsidian",
    "Titanium", "Plasma", "Antimatter", "Dark Matter", "Singularity",
    "Zero-G", "Superluminal", "Subspace", "Transwarp", "Exo",
    "Mildly Threatening", "Suspiciously Quiet", "Aggressively Friendly",
    "Legally Distinct", "Patent Pending", "Off-Brand", "Budget",
    "Premium", "Clearance", "AFK", "Tryhard", "Cracked", "OP",
];

static NOUNS: &[&str] = &[
    "Kraken", "Nebula", "Warlord", "Leviathan", "Phoenix", "Hydra",
    "Chimera", "Dragon", "Sphinx", "Basilisk", "Mammoth", "Cyclops",
    "Titan", "Comet", "Pulsar", "Quasar", "Supernova", "Meteor",
    "Asteroid", "Wormhole", "Stargate", "Void", "Abyss", "Rift",
    "Anomaly", "Singularity", "Entropy", "Photon", "Neutron", "Ion",
    "Proton", "Graviton", "Tachyon", "Meson", "Fermion", "Boson",
    "Dreadnought", "Corsair", "Marauder", "Buccaneer", "Privateer",
    "Raider", "Scavenger", "Harbinger", "Sentinel", "Juggernaut",
    "Berserker", "Revenant", "Specter", "Wraith", "Phantom",
    "Snail", "Penguin", "Capybara", "Axolotl", "Tardigrade",
    "Toaster", "Spatula", "Pickle", "Waffle", "Noodle",
];

static SUFFIXES: &[&str] = &[
    "Empire", "Fleet", "Dominion", "Cartel", "Swarm", "Collective",
    "Syndicate", "Armada", "Confederation", "Republic", "Federation",
    "Horde", "Legion", "Pact", "Coalition", "Vanguard", "Enclave",
    "Conclave", "Consortium", "Cabal", "Order", "Assembly",
    "Rebellion", "Resistance", "Expedition", "Thunderdome",
    "Fan Club", "Appreciation Society", "Inc", "Corporation",
    "Network", "Alliance", "Union", "Dynasty", "Monarchy",
];

static PREFIXES: &[&str] = &[
    "Order", "Knights", "Cult", "League", "Band", "Council",
    "Assembly", "Brotherhood", "Sisterhood", "Guild", "Sect",
    "Circle", "House", "Court", "Tribunal", "Chapter",
];

/// A curated palette of distinct, vibrant colors for factions.
fn faction_color_palette() -> Vec<[f32; 3]> {
    vec![
        [0.85, 0.2, 0.2],   // Crimson
        [0.2, 0.6, 0.85],   // Azure
        [0.2, 0.8, 0.3],    // Emerald
        [0.85, 0.6, 0.1],   // Amber
        [0.7, 0.2, 0.85],   // Violet
        [0.1, 0.8, 0.8],    // Teal
        [0.9, 0.4, 0.6],    // Rose
        [0.5, 0.8, 0.2],    // Lime
        [0.9, 0.5, 0.1],    // Orange
        [0.4, 0.3, 0.8],    // Indigo
        [0.8, 0.8, 0.1],    // Gold
        [0.3, 0.7, 0.6],    // Seafoam
        [0.8, 0.3, 0.5],    // Magenta
        [0.6, 0.4, 0.2],    // Bronze
        [0.3, 0.5, 0.9],    // Cobalt
        [0.7, 0.7, 0.3],    // Olive
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn test_faction_generation() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let factions = generate_factions(10, 1000, &mut rng);
        assert_eq!(factions.len(), 10);

        // All names should be unique.
        let names: std::collections::HashSet<_> =
            factions.iter().map(|f| &f.name).collect();
        assert_eq!(names.len(), 10, "Faction names should be unique");

        // IDs should be sequential starting from start_id.
        for (i, f) in factions.iter().enumerate() {
            assert_eq!(f.id, 1000 + i as u32);
        }
    }

    #[test]
    fn test_faction_name_not_empty() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(123);
        for _ in 0..100 {
            let name = generate_faction_name(&mut rng);
            assert!(!name.is_empty());
            assert!(name.len() > 3);
        }
    }
}
