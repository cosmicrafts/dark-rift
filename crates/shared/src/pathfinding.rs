//! Deterministic water A* (OpenFront `AStar.Water.ts` port). No HashMap iteration; integer costs only.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

use bevy::prelude::Resource;

use crate::map::GameMap;

const LAND_BIT: u8 = 7;
const MAGNITUDE_MASK: u8 = 0x1f;
const COST_SCALE: u32 = 100;
const BASE_COST: u32 = 1 * COST_SCALE;

#[inline]
fn magnitude_penalty(magnitude: u8) -> u32 {
    if magnitude < 3 {
        10 * COST_SCALE
    } else if magnitude <= 10 {
        0
    } else {
        1 * COST_SCALE
    }
}

#[inline]
fn cross_tie_breaker(
    nx: u32,
    ny: u32,
    goal_x: u32,
    goal_y: u32,
    start_x: u32,
    start_y: u32,
    cross_norm: u32,
) -> u32 {
    let dx_goal = goal_x as i64 - start_x as i64;
    let dy_goal = goal_y as i64 - start_y as i64;
    let dx_n = nx as i64 - goal_x as i64;
    let dy_n = ny as i64 - goal_y as i64;
    let cross = (dx_goal * dy_n - dy_goal * dx_n).wrapping_abs() as u64;
    let cn = cross_norm.max(1) as u64;
    ((cross * (COST_SCALE - 1) as u64) / cn / cn) as u32
}

/// Open heap node: max-heap by `Ord` pops smallest `f_score` first (see `cmp`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AStarNode {
    f_score: u32,
    insert_seq: u32,
    idx: u32,
}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        match other.f_score.cmp(&self.f_score) {
            Ordering::Equal => match other.insert_seq.cmp(&self.insert_seq) {
                Ordering::Equal => other.idx.cmp(&self.idx),
                o => o,
            },
            o => o,
        }
    }
}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Reusable buffers for water-only 4-neighbor A* (matches OpenFront defaults).
#[derive(Debug, Clone)]
pub struct WaterAStar {
    width: u32,
    height: u32,
    stamp: u32,
    closed_stamp: Vec<u32>,
    gscore_stamp: Vec<u32>,
    gscore: Vec<u32>,
    came_from: Vec<i32>,
    heap: BinaryHeap<AStarNode>,
    heuristic_weight: u32,
    max_iterations: u32,
    push_seq: u32,
}

impl Default for WaterAStar {
    fn default() -> Self {
        Self::new()
    }
}

impl WaterAStar {
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            stamp: 0,
            closed_stamp: Vec::new(),
            gscore_stamp: Vec::new(),
            gscore: Vec::new(),
            came_from: Vec::new(),
            heap: BinaryHeap::new(),
            heuristic_weight: 5,
            max_iterations: 1_000_000,
            push_seq: 0,
        }
    }

    pub fn ensure_capacity(&mut self, map: &GameMap) {
        let n = (map.width * map.height) as usize;
        if self.closed_stamp.len() != n {
            self.closed_stamp.resize(n, 0);
            self.gscore_stamp.resize(n, 0);
            self.gscore.resize(n, 0);
            self.came_from.resize(n, -1);
            self.width = map.width;
            self.height = map.height;
        }
    }

    /// Multi-source start (first start defines cross-tie line), single goal. Tile indices: `y * width + x`.
    pub fn find_path(
        &mut self,
        map: &GameMap,
        starts: &[u32],
        goal: u32,
    ) -> Option<Vec<u32>> {
        if starts.is_empty() {
            return None;
        }
        self.ensure_capacity(map);
        let width = map.width;
        let height = map.height;
        let num_nodes = (width * height) as usize;
        let land_mask = 1u8 << LAND_BIT;

        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.closed_stamp.fill(0);
            self.gscore_stamp.fill(0);
            self.stamp = 1;
        }
        let stamp = self.stamp;
        self.push_seq = 0;

        let goal_x = goal % width;
        let goal_y = goal / width;

        let s0 = starts[0];
        let start_x = s0 % width;
        let start_y = s0 / width;
        let dx_goal = goal_x as i32 - start_x as i32;
        let dy_goal = goal_y as i32 - start_y as i32;
        let cross_norm = (dx_goal.unsigned_abs() + dy_goal.unsigned_abs()).max(1) as u32;

        self.heap.clear();

        for &s in starts {
            if s as usize >= num_nodes {
                continue;
            }
            let sx = s % width;
            let sy = s / width;
            let idx = s as usize;
            self.gscore[idx] = 0;
            self.gscore_stamp[idx] = stamp;
            self.came_from[idx] = -1;
            let h = self
                .heuristic_weight
                .saturating_mul(BASE_COST)
                .saturating_mul(manhattan(sx, sy, goal_x, goal_y));
            self.push_seq = self.push_seq.wrapping_add(1);
            self.heap.push(AStarNode {
                f_score: h,
                insert_seq: self.push_seq,
                idx: s,
            });
        }

        let mut iterations = self.max_iterations;

        while let Some(node) = self.heap.pop() {
            if iterations == 0 {
                return None;
            }
            iterations -= 1;

            let current = node.idx as usize;
            if self.closed_stamp[current] == stamp {
                continue;
            }
            self.closed_stamp[current] = stamp;

            if node.idx == goal {
                return Some(self.build_path(goal as usize));
            }

            let current_g = self.gscore[current];
            let current_x = node.idx % width;
            let current_y = node.idx / width;

            // Neighbor order: N, S, W, E (matches `AStar.Water.ts`)
            let neighbors = [
                current_y.checked_sub(1).map(|ny| (current_x, ny)),
                if current_y + 1 < height {
                    Some((current_x, current_y + 1))
                } else {
                    None
                },
                current_x.checked_sub(1).map(|nx| (nx, current_y)),
                if current_x + 1 < width {
                    Some((current_x + 1, current_y))
                } else {
                    None
                },
            ];

            for opt in neighbors.into_iter().flatten() {
                let (nx, ny) = opt;
                let neighbor = (ny * width + nx) as usize;
                let b = map.terrain[neighbor].as_byte();
                let is_land = (b & land_mask) != 0;
                if neighbor as u32 != goal && is_land {
                    continue;
                }

                if self.closed_stamp[neighbor] == stamp {
                    continue;
                }

                let magnitude = b & MAGNITUDE_MASK;
                let cost = BASE_COST.saturating_add(magnitude_penalty(magnitude));
                let tentative_g = current_g.saturating_add(cost);

                if self.gscore_stamp[neighbor] != stamp || tentative_g < self.gscore[neighbor] {
                    self.came_from[neighbor] = current as i32;
                    self.gscore[neighbor] = tentative_g;
                    self.gscore_stamp[neighbor] = stamp;
                    let h = self
                        .heuristic_weight
                        .saturating_mul(BASE_COST)
                        .saturating_mul(manhattan(nx, ny, goal_x, goal_y));
                    let cross = cross_tie_breaker(nx, ny, goal_x, goal_y, start_x, start_y, cross_norm);
                    let f = tentative_g.saturating_add(h).saturating_add(cross);
                    self.push_seq = self.push_seq.wrapping_add(1);
                    self.heap.push(AStarNode {
                        f_score: f,
                        insert_seq: self.push_seq,
                        idx: neighbor as u32,
                    });
                }
            }
        }

        None
    }

    fn build_path(&self, goal: usize) -> Vec<u32> {
        let mut path = Vec::new();
        let mut current: i32 = goal as i32;
        while current >= 0 {
            let u = current as u32;
            path.push(u);
            current = self.came_from[u as usize];
        }
        path.reverse();
        path
    }
}

#[inline]
fn manhattan(x1: u32, y1: u32, x2: u32, y2: u32) -> u32 {
    x1.abs_diff(x2) + y1.abs_diff(y2)
}

/// Shared scratch buffers for water A* + closest-shore BFS (insert as `Resource` on the Bevy app).
#[derive(Resource, Default, Clone)]
pub struct WaterPathfinderScratch {
    pub astar: WaterAStar,
    pub bfs_queue: VecDeque<u32>,
    pub bfs_visited: Vec<u32>,
    pub bfs_stamp: u32,
}
