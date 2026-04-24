//! Connected-component labeling of the water plane (OpenFront parity for
//! `WaterManager.getWaterComponent`). Two shoreline tiles share a `component`
//! iff they touch the same connected body of water — ocean, lake, or river.
//!
//! Computed once at map-load (terrain is static). Each client does this locally
//! off the deterministic terrain bytes, so no network payload required.

use std::collections::VecDeque;

use bevy::prelude::Resource;

use crate::map::GameMap;

/// Component labels indexed by linear tile index (`y * width + x`).
///
/// - Water tile → ID of its flood-fill group (`>= 1`)
/// - Land + shoreline tile → ID of the smallest-ID water component any 4-neighbor
///   water tile belongs to (deterministic tie-break). This lets callers ask
///   "does my shore and the target's shore touch the same water body?" in O(1).
/// - Any other tile → `0` (no water component, not launchable, not landable).
#[derive(Resource, Debug, Clone, Default)]
pub struct WaterComponents {
    pub components: Vec<u32>,
    pub count: u32,
}

impl WaterComponents {
    /// One-shot flood-fill (4-connectivity) over water tiles, followed by shore
    /// inheritance. `O(width * height)` time, `O(width * height)` memory.
    pub fn compute(map: &GameMap) -> Self {
        let n = (map.width as usize) * (map.height as usize);
        if n == 0 {
            return Self::default();
        }
        let w = map.width;
        let mut components = vec![0u32; n];
        let mut count: u32 = 0;
        let mut queue: VecDeque<u32> = VecDeque::new();

        // ── 1. Label water bodies ────────────────────────────────────────
        for start in 0..n {
            if map.terrain[start].is_land() || components[start] != 0 {
                continue;
            }
            count += 1;
            let id = count;
            components[start] = id;
            queue.clear();
            queue.push_back(start as u32);
            while let Some(t) = queue.pop_front() {
                let x = t % w;
                let y = t / w;
                map.for_each_neighbor(x, y, |nx, ny| {
                    let ni = (ny * w + nx) as usize;
                    if !map.terrain[ni].is_land() && components[ni] == 0 {
                        components[ni] = id;
                        queue.push_back(ni as u32);
                    }
                });
            }
        }

        // ── 2. Shore tiles inherit the smallest adjacent water component ─
        //    (min-ID tie-break keeps this 100% deterministic across clients)
        for idx in 0..n {
            let tile = map.terrain[idx];
            if !tile.is_land() || !tile.is_shoreline() {
                continue;
            }
            let x = (idx as u32) % w;
            let y = (idx as u32) / w;
            let mut best: u32 = 0;
            map.for_each_neighbor(x, y, |nx, ny| {
                let ni = (ny * w + nx) as usize;
                if !map.terrain[ni].is_land() {
                    let c = components[ni];
                    if c > 0 && (best == 0 || c < best) {
                        best = c;
                    }
                }
            });
            components[idx] = best;
        }

        Self { components, count }
    }

    #[inline]
    pub fn component_of(&self, idx: u32) -> u32 {
        self.components.get(idx as usize).copied().unwrap_or(0)
    }
}
