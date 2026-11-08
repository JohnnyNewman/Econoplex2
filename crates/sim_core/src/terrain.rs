//! Terrain and flow fields.
//!
//! The map is a grid of cells, each with a travel cost per unit of distance:
//! 1 on open ground, more on hills, and impassable on rivers except at fords.
//! For every town one flow field is computed once (Dijkstra from the town over
//! the grid): each cell holds the cost of the cheapest road from there to the
//! town. A crowd heading for a town needs nothing else: every walker steps to its
//! neighbor cell with the lowest value, so ten thousand people share one field
//! instead of each searching for a path.
//!
//! The simulation reads the same fields for road lengths between towns: raids
//! march, migrants weigh the journey and traders pay for transport along roads
//! that go around the river, not through it.

use crate::config::{Scenario, TerrainSpec};
use bevy_ecs::prelude::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Resource, Debug, Clone)]
pub struct Terrain {
    pub cell: f32,
    /// World position of the corner of cell (0, 0).
    pub origin: (f32, f32),
    pub width: usize,
    pub height: usize,
    /// Travel cost per unit of distance; infinite where nobody can pass.
    pub cost: Vec<f32>,
    /// Water cells (rivers), for drawing.
    pub water: Vec<bool>,
    /// Per town: cost of the cheapest road from each cell to the town.
    pub fields: Vec<Vec<f32>>,
    /// Per pair of towns: road cost from one to the other.
    pub roads: Vec<Vec<f32>>,
}

/// Cheap deterministic value noise in [0, 1].
fn value_noise(seed: u64, x: f32, y: f32) -> f32 {
    let lattice = |i: i64, j: i64| {
        let mut h = seed
            ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    let (i, j) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let top = lattice(i, j) * (1.0 - sx) + lattice(i + 1, j) * sx;
    let bottom = lattice(i, j + 1) * (1.0 - sx) + lattice(i + 1, j + 1) * sx;
    top * (1.0 - sy) + bottom * sy
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

#[derive(PartialEq)]
struct Visit(f32, usize);
impl Eq for Visit {}
impl Ord for Visit {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on cost, ties by cell index so the result never depends on luck.
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}
impl PartialOrd for Visit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const NEIGHBORS: [(i32, i32, f32); 8] = [
    (1, 0, 1.0),
    (-1, 0, 1.0),
    (0, 1, 1.0),
    (0, -1, 1.0),
    (1, 1, std::f32::consts::SQRT_2),
    (1, -1, std::f32::consts::SQRT_2),
    (-1, 1, std::f32::consts::SQRT_2),
    (-1, -1, std::f32::consts::SQRT_2),
];

impl Terrain {
    /// Build the map from the scenario: flat ground around the towns when it
    /// describes no terrain.
    pub fn build(scenario: &Scenario) -> Self {
        let spec = scenario.terrain.clone().unwrap_or_default();
        let cell = spec.cell.max(1.0);
        // Cover every town with a margin.
        let (mut x0, mut y0, mut x1, mut y1) = (
            -spec.half_size.0,
            -spec.half_size.1,
            spec.half_size.0,
            spec.half_size.1,
        );
        for t in &scenario.towns {
            x0 = x0.min(t.position.0 - 400.0);
            y0 = y0.min(t.position.1 - 400.0);
            x1 = x1.max(t.position.0 + 400.0);
            y1 = y1.max(t.position.1 + 400.0);
        }
        let width = ((x1 - x0) / cell).ceil() as usize;
        let height = ((y1 - y0) / cell).ceil() as usize;
        let mut terrain = Terrain {
            cell,
            origin: (x0, y0),
            width,
            height,
            cost: vec![1.0; width * height],
            water: vec![false; width * height],
            fields: Vec::new(),
            roads: Vec::new(),
        };
        for c in 0..width * height {
            let p = terrain.center(c);
            let hill = value_noise(
                scenario.seed,
                p.0 / spec.hill_scale.max(1.0),
                p.1 / spec.hill_scale.max(1.0),
            );
            // Only the higher half of the noise counts as hills.
            let h = ((hill - 0.5) * 2.0).max(0.0);
            terrain.cost[c] = 1.0 + spec.hill_cost * h * h;
            for river in &spec.rivers {
                let near = river
                    .points
                    .windows(2)
                    .any(|w| segment_distance(p, w[0], w[1]) <= river.width / 2.0);
                let ford = river.fords.iter().any(|f| {
                    ((p.0 - f.0).powi(2) + (p.1 - f.1).powi(2)).sqrt() <= river.ford_width / 2.0
                });
                if near {
                    terrain.water[c] = true;
                    terrain.cost[c] = if ford { river.ford_cost } else { f32::INFINITY };
                }
            }
        }
        // Towns stand on level, dry ground.
        for t in &scenario.towns {
            for c in 0..width * height {
                let p = terrain.center(c);
                if ((p.0 - t.position.0).powi(2) + (p.1 - t.position.1).powi(2)).sqrt() < 160.0 {
                    terrain.cost[c] = 1.0;
                    terrain.water[c] = false;
                }
            }
        }
        terrain.fields = scenario
            .towns
            .iter()
            .map(|t| terrain.flow_field(t.position))
            .collect();
        terrain.roads = scenario
            .towns
            .iter()
            .map(|a| {
                (0..scenario.towns.len())
                    .map(|b| terrain.cost_to(b, a.position))
                    .collect()
            })
            .collect();
        terrain
    }

    pub fn center(&self, c: usize) -> (f32, f32) {
        let (i, j) = (c % self.width, c / self.width);
        (
            self.origin.0 + (i as f32 + 0.5) * self.cell,
            self.origin.1 + (j as f32 + 0.5) * self.cell,
        )
    }

    pub fn cell_at(&self, p: (f32, f32)) -> Option<usize> {
        let i = ((p.0 - self.origin.0) / self.cell).floor();
        let j = ((p.1 - self.origin.1) / self.cell).floor();
        (i >= 0.0 && j >= 0.0 && (i as usize) < self.width && (j as usize) < self.height)
            .then(|| j as usize * self.width + i as usize)
    }

    pub fn passable(&self, p: (f32, f32)) -> bool {
        self.cell_at(p).is_some_and(|c| self.cost[c].is_finite())
    }

    /// The cells next to `c` with the length of the step to each. A diagonal step
    /// is only allowed when both cells beside it are passable, so nobody slips
    /// between two water cells that touch at a corner.
    fn neighbors(&self, c: usize) -> impl Iterator<Item = (usize, f32)> + '_ {
        let (w, h) = (self.width as i32, self.height as i32);
        let (i, j) = ((c % self.width) as i32, (c / self.width) as i32);
        let open = move |x: i32, y: i32| {
            x >= 0 && y >= 0 && x < w && y < h && self.cost[(y * w + x) as usize].is_finite()
        };
        NEIGHBORS.iter().filter_map(move |&(di, dj, step)| {
            let (ni, nj) = (i + di, j + dj);
            if ni < 0 || nj < 0 || ni >= w || nj >= h {
                return None;
            }
            if di != 0 && dj != 0 && !(open(i + di, j) && open(i, j + dj)) {
                return None;
            }
            Some(((nj * w + ni) as usize, step))
        })
    }

    /// Dry, passable ground.
    pub fn buildable(&self, p: (f32, f32)) -> bool {
        self.cell_at(p)
            .is_some_and(|c| self.cost[c].is_finite() && !self.water[c])
    }

    /// Cheapest-road costs from every cell to `goal` (Dijkstra over the grid).
    pub fn flow_field(&self, goal: (f32, f32)) -> Vec<f32> {
        let mut dist = vec![f32::INFINITY; self.cost.len()];
        let Some(start) = self.cell_at(goal) else {
            return dist;
        };
        let mut heap = BinaryHeap::new();
        dist[start] = 0.0;
        heap.push(Visit(0.0, start));
        while let Some(Visit(d, c)) = heap.pop() {
            if d > dist[c] {
                continue;
            }
            for (n, step) in self.neighbors(c) {
                // Moving between two cells costs the mean of both.
                let w = 0.5 * (self.cost[c] + self.cost[n]) * step * self.cell;
                if !w.is_finite() {
                    continue;
                }
                let nd = d + w;
                if nd < dist[n] {
                    dist[n] = nd;
                    heap.push(Visit(nd, n));
                }
            }
        }
        dist
    }

    /// Road cost from `p` to town `town`.
    pub fn cost_to(&self, town: usize, p: (f32, f32)) -> f32 {
        self.cell_at(p)
            .and_then(|c| self.fields.get(town).map(|f| f[c]))
            .unwrap_or(f32::INFINITY)
    }

    /// Road cost between two towns.
    pub fn road(&self, from: usize, to: usize) -> f32 {
        self.roads[from][to]
    }

    /// The direction to walk from `p` toward town `town`: toward the neighboring
    /// cell with the lowest road cost. `None` at the town or where no road leads.
    pub fn step_toward(&self, town: usize, p: (f32, f32)) -> Option<(f32, f32)> {
        let field = self.fields.get(town)?;
        let c = self.cell_at(p)?;
        let mut best = (field[c], c);
        for (n, _) in self.neighbors(c) {
            if field[n] < best.0 {
                best = (field[n], n);
            }
        }
        if best.1 == c {
            return None;
        }
        let to = self.center(best.1);
        let (dx, dy) = (to.0 - p.0, to.1 - p.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        Some((dx / len, dy / len))
    }
}

impl Terrain {
    /// The road from `p` to town `town` as a line of cell centers, following the
    /// town's flow field downhill; it ends at the town.
    pub fn path(&self, p: (f32, f32), town: usize) -> Vec<(f32, f32)> {
        let mut out = vec![p];
        let (Some(field), Some(mut c)) = (self.fields.get(town), self.cell_at(p)) else {
            return out;
        };
        while field[c] > 0.0 && field[c].is_finite() {
            let mut best = c;
            for (n, _) in self.neighbors(c) {
                if field[n] < field[best] {
                    best = n;
                }
            }
            if best == c {
                break;
            }
            c = best;
            out.push(self.center(c));
        }
        out
    }
}

/// The point a fraction `f` of the way along a path.
pub fn along(path: &[(f32, f32)], f: f32) -> (f32, f32) {
    let seg = |w: &[(f32, f32)]| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
    let total: f32 = path.windows(2).map(seg).sum();
    let mut left = f.clamp(0.0, 1.0) * total;
    for w in path.windows(2) {
        let l = seg(w);
        if left <= l && l > 0.0 {
            let k = left / l;
            return (
                w[0].0 + (w[1].0 - w[0].0) * k,
                w[0].1 + (w[1].1 - w[0].1) * k,
            );
        }
        left -= l;
    }
    *path.last().unwrap_or(&(0.0, 0.0))
}

impl Default for TerrainSpec {
    fn default() -> Self {
        TerrainSpec {
            cell: 20.0,
            half_size: (0.0, 0.0),
            hill_cost: 0.0,
            hill_scale: 300.0,
            rivers: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RiverSpec, TownSpec};

    fn town(x: f32, y: f32) -> TownSpec {
        TownSpec {
            name: format!("{x}"),
            position: (x, y),
            population: 0,
            nature: Vec::new(),
            buildings: Vec::new(),
            stock: Vec::new(),
        }
    }

    #[test]
    fn roads_go_around_a_river_through_its_ford() {
        let scenario = Scenario {
            seed: 1,
            player: None,
            towns: vec![town(-400.0, 0.0), town(400.0, 0.0)],
            terrain: Some(TerrainSpec {
                rivers: vec![RiverSpec {
                    points: vec![(0.0, -900.0), (0.0, 900.0)],
                    width: 40.0,
                    fords: vec![(0.0, 300.0)],
                    ford_width: 40.0,
                    ford_cost: 2.0,
                }],
                ..Default::default()
            }),
        };
        let t = Terrain::build(&scenario);
        assert!(!t.passable((0.0, 0.0)));
        assert!(t.passable((0.0, 300.0)));
        let road = t.road(0, 1);
        // Straight would be 800; the ford is 300 off the line: at least 2 * 500.
        assert!(road > 1000.0 && road < 1200.0, "road {road}");
        assert!((t.road(1, 0) - road).abs() < 1.0);
        // Following the field from one town leads to the other, over the ford.
        let mut p = (-400.0f32, 0.0f32);
        let mut crossed = false;
        for _ in 0..400 {
            let Some(d) = t.step_toward(1, p) else { break };
            p = (p.0 + d.0 * 5.0, p.1 + d.1 * 5.0);
            assert!(t.passable(p));
            crossed |= p.0.abs() < 20.0;
        }
        assert!(
            crossed && (p.0 - 400.0).abs() < 30.0 && p.1.abs() < 30.0,
            "ended at {p:?}"
        );
    }
}
