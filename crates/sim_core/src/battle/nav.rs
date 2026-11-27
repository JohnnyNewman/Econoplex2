//! Obstacles on the field, and finding a way round them.
//!
//! A field holds a list of [`Obstacle`]s: shapes (oriented rectangles, and
//! thin wall segments for walls to come), each with the id of the building
//! it belongs to and whether it lets a side through ([`Passage`]: blocked,
//! open, or a gate one side may use). [`Nav`] is built from them once per
//! change of the list and answers what both battle models ask:
//!
//! - **Soldiers** (exact shapes, through a bucket grid of 8 m cells): push a
//!   point out of every obstacle ([`Nav::push_out`]), keep a step from
//!   passing through one ([`Nav::resolve`]) and pick the corner to walk
//!   round when one stands in the way ([`Nav::steer`]).
//! - **Squads** (a 1 m grid with every obstacle grown by [`CLEAR`]): whether
//!   a straight line is clear ([`Nav::clear_line`]), a shortest way round
//!   (A* with the corners pulled tight, [`Nav::path`]), the nearest free spot
//!   to a point ([`Nav::free_spot`]), the width of clear ground across a
//!   heading ([`Nav::room`]), and for runners a flow field to their own edge
//!   of the field ([`Nav::home_dir`]).
//!
//! A squad keeps its way in a [`Route`], planned again only when its goal
//! moves, its line of sight is lost, or the obstacles change: each change
//! builds a new `Nav` with a higher [`Nav::version`], which every route
//! compares. A field without obstacles has no `Nav` at all, so a battle in
//! the open costs and fights exactly as before.

use super::soldiers::V2;
use super::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::{Mutex, OnceLock};

/// Metres every obstacle grows by on the squads' grid: a squad's anchor
/// keeps this far from walls.
pub const CLEAR: f32 = 1.0;
/// Side of a cell of the squads' grid, metres.
const CELL: f32 = 1.0;
/// Side of a bucket of the soldiers' shape lookup, and how far past its
/// cell each shape is listed, metres: a query within `PAD` of a point
/// needs only the point's bucket.
const BUCKET: f32 = 8.0;
const PAD: f32 = 4.0;
/// How far ahead a soldier looks for an obstacle in their way, metres: at
/// most `2 * PAD`, so the buckets at both ends list everything between.
const LOOK: f32 = 8.0;
/// Shapes thinner than this (half width, metres) are swept, so a fast
/// step cannot pass through them.
const THIN: f32 = 1.0;
/// How much the search leans toward the goal: its way is at most this
/// many times the shortest (and shorter once its corners are pulled tight).
const HEURISTIC: f32 = 1.5;
/// The most cells one search may open.
const MAX_OPEN: usize = 400_000;

/// What an obstacle is shaped like, in metres on the field.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// A rectangle: its centre, half its size along `axis` and across it,
    /// and `axis` (unit). A building or a house.
    Rect {
        centre: (f32, f32),
        half: (f32, f32),
        axis: (f32, f32),
    },
    /// A thin wall from `a` to `b`, `half_width` to each side: a stretch of
    /// palisade or of a town wall, or a gate in one.
    Segment {
        a: (f32, f32),
        b: (f32, f32),
        half_width: f32,
    },
}

/// Who may pass through an obstacle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Passage {
    /// Nobody.
    #[default]
    Blocked,
    /// Everybody: a breach, or a gate left open.
    Open,
    /// Only side `0` or `1`: a gate its own side holds.
    Gate(u8),
}

impl Passage {
    /// Whether it stops soldiers of `side`.
    pub fn blocks(self, side: u8) -> bool {
        match self {
            Self::Blocked => true,
            Self::Open => false,
            Self::Gate(s) => s != side,
        }
    }
}

/// One obstacle on the field: a shape, the id of what it belongs to (a
/// building's id, so a view or a siege can find it), and who may pass.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Obstacle {
    pub id: u64,
    pub shape: Shape,
    pub passage: Passage,
}

impl Obstacle {
    /// A rectangle nobody passes.
    pub fn rect(id: u64, centre: (f32, f32), half: (f32, f32), axis: (f32, f32)) -> Self {
        Self {
            id,
            shape: Shape::Rect { centre, half, axis },
            passage: Passage::Blocked,
        }
    }

    /// Whether `p` lies inside its shape grown by `r`.
    pub fn contains(&self, p: (f32, f32), r: f32) -> bool {
        Box2::of(self).inside(V2::new(p.0, p.1), r)
    }
}

/// A shape as a rectangle: centre, unit axes, half sizes, and which sides
/// it stops (bit `s` for side `s`).
#[derive(Clone, Copy, Debug)]
struct Box2 {
    c: V2,
    u: V2,
    v: V2,
    hx: f32,
    hy: f32,
    stops: u8,
}

impl Box2 {
    fn of(o: &Obstacle) -> Self {
        let stops = (o.passage.blocks(0) as u8) | ((o.passage.blocks(1) as u8) << 1);
        match o.shape {
            Shape::Rect { centre, half, axis } => {
                let u = V2::new(axis.0, axis.1).norm_or(V2::new(1.0, 0.0));
                Self {
                    c: V2::new(centre.0, centre.1),
                    u,
                    v: V2::new(-u.y, u.x),
                    hx: half.0.max(0.0),
                    hy: half.1.max(0.0),
                    stops,
                }
            }
            Shape::Segment { a, b, half_width } => {
                let (a, b) = (V2::new(a.0, a.1), V2::new(b.0, b.1));
                let u = (b - a).norm_or(V2::new(1.0, 0.0));
                Self {
                    c: (a + b) * 0.5,
                    u,
                    v: V2::new(-u.y, u.x),
                    hx: (b - a).len() * 0.5 + half_width,
                    hy: half_width.max(0.0),
                    stops,
                }
            }
        }
    }

    #[inline]
    fn stops(&self, side: u8) -> bool {
        self.stops & (1 << side.min(1)) != 0
    }

    #[inline]
    fn local(&self, p: V2) -> (f32, f32) {
        let d = p - self.c;
        (d.dot(self.u), d.dot(self.v))
    }

    #[inline]
    fn at(&self, lx: f32, ly: f32) -> V2 {
        self.c + self.u * lx + self.v * ly
    }

    #[inline]
    fn inside(&self, p: V2, r: f32) -> bool {
        let (lx, ly) = self.local(p);
        lx.abs() < self.hx + r && ly.abs() < self.hy + r
    }

    /// Half the size of the box around it on the field's axes.
    fn reach(&self) -> (f32, f32) {
        (
            self.u.x.abs() * self.hx + self.v.x.abs() * self.hy,
            self.u.y.abs() * self.hx + self.v.y.abs() * self.hy,
        )
    }

    /// Where along `a`..`b` (0 to 1) the segment enters the box grown by
    /// `r`, if it does and `a` is outside.
    fn enters(&self, a: V2, b: V2, r: f32) -> Option<f32> {
        let (ax, ay) = self.local(a);
        let (bx, by) = self.local(b);
        let (ex, ey) = (self.hx + r, self.hy + r);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for (p, d, e) in [(ax, bx - ax, ex), (ay, by - ay, ey)] {
            if d.abs() < 1e-9 {
                if p.abs() >= e {
                    return None;
                }
                continue;
            }
            let (mut lo, mut hi) = ((-e - p) / d, (e - p) / d);
            if lo > hi {
                std::mem::swap(&mut lo, &mut hi);
            }
            t0 = t0.max(lo);
            t1 = t1.min(hi);
            if t0 > t1 {
                return None;
            }
        }
        (t0 > 0.0).then_some(t0)
    }
}

/// Shapes ready for questions about bodies near them: a bucket grid over a
/// rectangle from `origin`, `size` big, each bucket listing the shapes within
/// `PAD` of it. A battle's `Nav` holds one in field metres; a town's walkers
/// use one in map units (`layout::TownLayout::shapes`).
#[derive(Clone)]
pub struct Shapes {
    boxes: Vec<Box2>,
    origin: V2,
    width: f32,
    height: f32,
    /// Buckets across and down; bucket `k`'s boxes are
    /// `items[start[k]..start[k + 1]]`.
    bw: usize,
    bh: usize,
    start: Vec<u32>,
    items: Vec<u32>,
    /// Whether any shape is thin enough to need sweeping (`THIN`).
    thin: bool,
}

/// The obstacles of a field, ready for the soldiers' and the squads' questions.
pub struct Nav {
    obstacles: Vec<Obstacle>,
    version: u64,
    shapes: Shapes,
    /// The squads' grid: cells across and down, and per cell bit `s` set if
    /// it is blocked for side `s` (obstacles grown by `CLEAR`).
    gw: usize,
    gh: usize,
    blocked: Vec<u8>,
    /// Each side's flow field to its own edge of the field, made the first
    /// time someone of that side runs.
    home: [OnceLock<Vec<f32>>; 2],
    search: Mutex<Search>,
}

impl std::fmt::Debug for Nav {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nav")
            .field("version", &self.version)
            .field("obstacles", &self.obstacles.len())
            .finish()
    }
}

impl PartialEq for Nav {
    fn eq(&self, o: &Self) -> bool {
        self.version == o.version && self.obstacles == o.obstacles
    }
}

/// A search's working memory, kept between searches.
#[derive(Default)]
struct Search {
    cost: Vec<f32>,
    from: Vec<u32>,
    seen: Vec<u32>,
    stamp: u32,
    heap: BinaryHeap<Reverse<(u32, u32)>>,
}

const SQRT2: f32 = std::f32::consts::SQRT_2;
const STEPS: [(i32, i32, f32); 8] = [
    (1, 0, 1.0),
    (-1, 0, 1.0),
    (0, 1, 1.0),
    (0, -1, 1.0),
    (1, 1, SQRT2),
    (1, -1, SQRT2),
    (-1, 1, SQRT2),
    (-1, -1, SQRT2),
];

impl Nav {
    /// The obstacles of `field`, or `None` for a field without any.
    pub fn new(field: &Field, version: u64) -> Option<Self> {
        if field.obstacles.is_empty() {
            return None;
        }
        let boxes: Vec<Box2> = field.obstacles.iter().map(Box2::of).collect();
        let shapes = Shapes::of(boxes.clone(), V2::ZERO, (field.width, field.height));
        let gw = (field.width / CELL).ceil().max(1.0) as usize;
        let gh = (field.height / CELL).ceil().max(1.0) as usize;
        let mut blocked = vec![0u8; gw * gh];
        for b in &boxes {
            if b.stops == 0 {
                continue;
            }
            let (rx, ry) = b.reach();
            let (rx, ry) = (rx + CLEAR + CELL, ry + CLEAR + CELL);
            let x0 = ((b.c.x - rx) / CELL).floor().max(0.0) as usize;
            let y0 = ((b.c.y - ry) / CELL).floor().max(0.0) as usize;
            let x1 = (((b.c.x + rx) / CELL).ceil().max(0.0) as usize).min(gw);
            let y1 = (((b.c.y + ry) / CELL).ceil().max(0.0) as usize).min(gh);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = V2::new((x as f32 + 0.5) * CELL, (y as f32 + 0.5) * CELL);
                    if b.inside(p, CLEAR) {
                        blocked[y * gw + x] |= b.stops;
                    }
                }
            }
        }
        Some(Self {
            obstacles: field.obstacles.clone(),
            version,
            shapes,
            gw,
            gh,
            blocked,
            home: [OnceLock::new(), OnceLock::new()],
            search: Mutex::new(Search::default()),
        })
    }

    /// Goes up by one with every change to the obstacles: a route planned
    /// on an older one is planned again.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn obstacles(&self) -> &[Obstacle] {
        &self.obstacles
    }

    // --- The soldiers' questions: exact shapes (`Shapes`). ---

    #[inline]
    pub fn any_near(&self, p: V2) -> bool {
        self.shapes.any_near(p)
    }
    pub fn inside(&self, p: V2, r: f32, side: u8) -> bool {
        self.shapes.inside(p, r, side)
    }
    pub fn push_out(&self, p: V2, r: f32, side: u8) -> V2 {
        self.shapes.push_out(p, r, side)
    }
    pub fn resolve(&self, from: V2, to: V2, r: f32, side: u8) -> V2 {
        self.shapes.resolve(from, to, r, side)
    }
    pub fn steer(&self, me: V2, target: V2, r: f32, side: u8) -> Option<V2> {
        self.shapes.steer(me, target, r, side)
    }

    // --- The squads' questions: the grid. ---

    #[inline]
    fn cell_of(&self, p: V2) -> Option<usize> {
        if p.x < 0.0 || p.y < 0.0 {
            return None;
        }
        let (x, y) = ((p.x / CELL) as usize, (p.y / CELL) as usize);
        (x < self.gw && y < self.gh).then_some(y * self.gw + x)
    }

    #[inline]
    fn centre(&self, c: usize) -> V2 {
        V2::new(
            ((c % self.gw) as f32 + 0.5) * CELL,
            ((c / self.gw) as f32 + 0.5) * CELL,
        )
    }

    #[inline]
    fn free_cell(&self, c: usize, side: u8) -> bool {
        self.blocked[c] & (1 << side.min(1)) == 0
    }

    /// Whether a squad of side `side` stands clear at `p` (off the field is clear).
    pub fn open(&self, p: V2, side: u8) -> bool {
        self.cell_of(p).is_none_or(|c| self.free_cell(c, side))
    }

    /// Whether the straight way from `a` to `b` keeps `CLEAR` from every
    /// obstacle that stops `side`.
    pub fn clear_line(&self, a: V2, b: V2, side: u8) -> bool {
        let d = b - a;
        let n = (d.len() / CELL).ceil() as usize;
        if n == 0 {
            return self.open(a, side);
        }
        let step = d * (1.0 / n as f32);
        (0..=n).all(|k| self.open(a + step * k as f32, side))
    }

    /// The free cell nearest to `p`, within `limit` metres.
    fn nearest_free(&self, p: V2, side: u8, limit: f32) -> Option<usize> {
        let x = ((p.x / CELL).floor() as i64).clamp(0, self.gw as i64 - 1);
        let y = ((p.y / CELL).floor() as i64).clamp(0, self.gh as i64 - 1);
        let mut best: Option<(f32, usize)> = None;
        let rings = (limit / CELL).ceil() as i64;
        for ring in 0..=rings {
            if best.is_some_and(|b| b.0.sqrt() < (ring as f32 - 1.0) * CELL) {
                break;
            }
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (cx, cy) = (x + dx, y + dy);
                    if cx < 0 || cy < 0 || cx >= self.gw as i64 || cy >= self.gh as i64 {
                        continue;
                    }
                    let c = cy as usize * self.gw + cx as usize;
                    if !self.free_cell(c, side) {
                        continue;
                    }
                    let d2 = (self.centre(c) - p).len2();
                    if best.is_none_or(|b| d2 < b.0 || (d2 == b.0 && c < b.1)) {
                        best = Some((d2, c));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// `p` if a squad of side `side` can stand there, else the nearest spot
    /// where it can (within 100 m; `p` itself if there is none).
    pub fn free_spot(&self, p: V2, side: u8) -> V2 {
        if self.open(p, side) {
            return p;
        }
        self.nearest_free(p, side, 100.0)
            .map_or(p, |c| self.centre(c))
    }

    /// The width of clear ground across heading `dir` at `p`, metres: the
    /// narrowest of three looks, at `p` and a few metres on. Up to `max`.
    pub fn room(&self, p: V2, dir: V2, side: u8, max: f32) -> f32 {
        let across = dir.right();
        let half = 0.5 * max;
        [0.0, 3.0, 6.0]
            .into_iter()
            .map(|ahead| {
                let at = p + dir * ahead;
                if !self.open(at, side) {
                    return 0.0;
                }
                let reach = |s: f32| {
                    let mut d = 0.0;
                    while d < half && self.open(at + across * (s * (d + CELL)), side) {
                        d += CELL;
                    }
                    d
                };
                let w = reach(1.0) + reach(-1.0) + 2.0 * CLEAR;
                w.min(max)
            })
            .fold(max, f32::min)
    }

    /// A shortest way for a squad of side `side` from `from` to `to` that
    /// keeps `CLEAR` from every obstacle: its corners, ending at `to` (or at
    /// the nearest free spot to it). `None` if there is no way.
    pub fn path(&self, from: V2, to: V2, side: u8) -> Option<Vec<V2>> {
        let end_free = self.open(to, side) && self.cell_of(to).is_some();
        if end_free && self.clear_line(from, to, side) {
            return Some(vec![to]);
        }
        let s = self.nearest_free(from, side, 30.0)?;
        let g = self.nearest_free(to, side, 100.0)?;
        let goal = if end_free { to } else { self.centre(g) };
        if s == g {
            return Some(vec![goal]);
        }
        let cells = self.search(s, g, side)?;
        // Pull the corners tight: from each corner, on to the furthest cell
        // still in sight.
        let mut way = Vec::new();
        let mut at = from;
        let mut k = 0;
        while k + 1 < cells.len() {
            let mut j = k + 1;
            while j + 1 < cells.len() && self.clear_line(at, self.centre(cells[j + 1]), side) {
                j += 1;
            }
            if j + 1 == cells.len() {
                break;
            }
            let c = self.centre(cells[j]);
            way.push(c);
            at = c;
            k = j;
        }
        way.push(goal);
        Some(way)
    }

    /// The length of the way from `from` along `way`.
    pub fn length(from: V2, way: &[V2]) -> f32 {
        let mut at = from;
        let mut l = 0.0;
        for &p in way {
            l += (p - at).len();
            at = p;
        }
        l
    }

    /// A* from cell `s` to cell `g`: the cells on the way, both included.
    fn search(&self, s: usize, g: usize, side: u8) -> Option<Vec<usize>> {
        let mut guard = self.search.lock().unwrap_or_else(|e| e.into_inner());
        let w = &mut *guard;
        let n = self.gw * self.gh;
        if w.cost.len() != n {
            w.cost = vec![0.0; n];
            w.from = vec![0; n];
            w.seen = vec![0; n];
            w.stamp = 0;
        }
        w.stamp = w.stamp.wrapping_add(1);
        if w.stamp == 0 {
            w.seen.fill(0);
            w.stamp = 1;
        }
        let stamp = w.stamp;
        w.heap.clear();
        let (gx, gy) = ((g % self.gw) as f32, (g / self.gw) as f32);
        let h = |c: usize| {
            let (dx, dy) = (
                ((c % self.gw) as f32 - gx).abs(),
                ((c / self.gw) as f32 - gy).abs(),
            );
            // Weighted: a way at most this much longer than the shortest,
            // found opening far fewer cells.
            HEURISTIC * (dx.max(dy) + (SQRT2 - 1.0) * dx.min(dy))
        };
        w.cost[s] = 0.0;
        w.from[s] = s as u32;
        w.seen[s] = stamp;
        w.heap.push(Reverse((h(s).to_bits(), s as u32)));
        let mut opened = 0;
        while let Some(Reverse((f, c))) = w.heap.pop() {
            let c = c as usize;
            if c == g {
                let mut cells = vec![g];
                let mut k = g;
                while k != s {
                    k = w.from[k] as usize;
                    cells.push(k);
                }
                cells.reverse();
                return Some(cells);
            }
            // A stale entry: the cell was reached more cheaply since.
            if f32::from_bits(f) > w.cost[c] + h(c) + 1e-4 {
                continue;
            }
            opened += 1;
            if opened > MAX_OPEN {
                return None;
            }
            let (cx, cy) = ((c % self.gw) as i32, (c / self.gw) as i32);
            for &(dx, dy, step) in &STEPS {
                let (nx, ny) = (cx + dx, cy + dy);
                if nx < 0 || ny < 0 || nx >= self.gw as i32 || ny >= self.gh as i32 {
                    continue;
                }
                let nc = ny as usize * self.gw + nx as usize;
                if !self.free_cell(nc, side) {
                    continue;
                }
                // No cutting a corner between two blocked cells.
                if dx != 0
                    && dy != 0
                    && (!self.free_cell(cy as usize * self.gw + nx as usize, side)
                        || !self.free_cell(ny as usize * self.gw + cx as usize, side))
                {
                    continue;
                }
                let cost = w.cost[c] + step;
                if w.seen[nc] == stamp && cost >= w.cost[nc] {
                    continue;
                }
                w.seen[nc] = stamp;
                w.cost[nc] = cost;
                w.from[nc] = c as u32;
                w.heap.push(Reverse(((cost + h(nc)).to_bits(), nc as u32)));
            }
        }
        None
    }

    /// Side `side`'s flow field: the length of the way from each cell to its
    /// own edge of the field (x = 0 for the attackers, the far edge for the
    /// defenders), made once.
    fn home_field(&self, side: u8) -> &[f32] {
        let side = side.min(1);
        self.home[side as usize].get_or_init(|| {
            let n = self.gw * self.gh;
            let mut cost = vec![f32::INFINITY; n];
            let mut heap = BinaryHeap::new();
            let x = if side == 0 { 0 } else { self.gw - 1 };
            for y in 0..self.gh {
                let c = y * self.gw + x;
                if self.free_cell(c, side) {
                    cost[c] = 0.0;
                    heap.push(Reverse((0.0f32.to_bits(), c as u32)));
                }
            }
            while let Some(Reverse((f, c))) = heap.pop() {
                let c = c as usize;
                if f32::from_bits(f) > cost[c] {
                    continue;
                }
                let (cx, cy) = ((c % self.gw) as i32, (c / self.gw) as i32);
                for &(dx, dy, step) in &STEPS {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= self.gw as i32 || ny >= self.gh as i32 {
                        continue;
                    }
                    let nc = ny as usize * self.gw + nx as usize;
                    if !self.free_cell(nc, side) {
                        continue;
                    }
                    if dx != 0
                        && dy != 0
                        && (!self.free_cell(cy as usize * self.gw + nx as usize, side)
                            || !self.free_cell(ny as usize * self.gw + cx as usize, side))
                    {
                        continue;
                    }
                    let k = cost[c] + step;
                    if k < cost[nc] {
                        cost[nc] = k;
                        heap.push(Reverse((k.to_bits(), nc as u32)));
                    }
                }
            }
            cost
        })
    }

    /// The way a runner of side `side` at `p` goes for their own edge of the
    /// field, round the obstacles: down the flow field. `None` where it
    /// gives no way (off the field, or shut in).
    pub fn home_dir(&self, p: V2, side: u8) -> Option<V2> {
        let c = self.cell_of(p)?;
        let field = self.home_field(side);
        let (cx, cy) = ((c % self.gw) as i32, (c / self.gw) as i32);
        let mut best = (field[c], c);
        for &(dx, dy, _) in &STEPS {
            let (nx, ny) = (cx + dx, cy + dy);
            if nx < 0 || ny < 0 || nx >= self.gw as i32 || ny >= self.gh as i32 {
                continue;
            }
            let nc = ny as usize * self.gw + nx as usize;
            if field[nc] < best.0 {
                best = (field[nc], nc);
            }
        }
        (best.1 != c && best.0.is_finite()).then(|| (self.centre(best.1) - p).norm_or(V2::ZERO))
    }
}

impl Shapes {
    /// Every obstacle in `obstacles`, for questions within the rectangle
    /// from `origin`, `size` big (outside it nothing is in the way).
    pub fn new(obstacles: &[Obstacle], origin: (f32, f32), size: (f32, f32)) -> Self {
        let boxes = obstacles.iter().map(Box2::of).collect();
        Self::of(boxes, V2::new(origin.0, origin.1), size)
    }

    fn of(boxes: Vec<Box2>, origin: V2, size: (f32, f32)) -> Self {
        let bw = (size.0 / BUCKET).ceil().max(1.0) as usize;
        let bh = (size.1 / BUCKET).ceil().max(1.0) as usize;
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); bw * bh];
        let bucket_range = |lo: f32, hi: f32, n: usize| {
            let a = ((lo / BUCKET).floor().max(0.0) as usize).min(n - 1);
            let b = ((hi / BUCKET).floor().max(0.0) as usize).min(n - 1);
            a..=b
        };
        for (k, b) in boxes.iter().enumerate() {
            let (rx, ry) = b.reach();
            let c = b.c - origin;
            if c.x + rx + PAD < 0.0
                || c.y + ry + PAD < 0.0
                || c.x - rx - PAD > size.0
                || c.y - ry - PAD > size.1
            {
                continue;
            }
            for y in bucket_range(c.y - ry - PAD, c.y + ry + PAD, bh) {
                for x in bucket_range(c.x - rx - PAD, c.x + rx + PAD, bw) {
                    lists[y * bw + x].push(k as u32);
                }
            }
        }
        let mut start = Vec::with_capacity(lists.len() + 1);
        let mut items = Vec::new();
        start.push(0);
        for l in &lists {
            items.extend_from_slice(l);
            start.push(items.len() as u32);
        }
        Self {
            thin: boxes.iter().any(|b| b.hx.min(b.hy) < THIN),
            boxes,
            origin,
            width: size.0,
            height: size.1,
            bw,
            bh,
            start,
            items,
        }
    }

    /// The boxes listed in `p`'s bucket.
    #[inline]
    fn near(&self, p: V2) -> &[u32] {
        let p = p - self.origin;
        if p.x < -PAD || p.y < -PAD || p.x > self.width + PAD || p.y > self.height + PAD {
            return &[];
        }
        let x = ((p.x / BUCKET).max(0.0) as usize).min(self.bw - 1);
        let y = ((p.y / BUCKET).max(0.0) as usize).min(self.bh - 1);
        let k = y * self.bw + x;
        &self.items[self.start[k] as usize..self.start[k + 1] as usize]
    }

    /// Whether any obstacle is listed near `p`: if not, nothing needs asking.
    #[inline]
    pub fn any_near(&self, p: V2) -> bool {
        !self.near(p).is_empty()
    }

    /// Whether `p` stands inside an obstacle that stops `side`, grown by `r`.
    pub fn inside(&self, p: V2, r: f32, side: u8) -> bool {
        self.near(p).iter().any(|&k| {
            let b = &self.boxes[k as usize];
            b.stops(side) && b.inside(p, r)
        })
    }

    /// `p` moved out of every obstacle that stops `side`, so that a body of
    /// radius `r` (at most `PAD`) stands clear of it: out by the shortest way.
    pub fn push_out(&self, mut p: V2, r: f32, side: u8) -> V2 {
        for _ in 0..2 {
            let mut moved = false;
            for &k in self.near(p) {
                let b = &self.boxes[k as usize];
                if !b.stops(side) {
                    continue;
                }
                let (lx, ly) = b.local(p);
                let (ex, ey) = (b.hx + r, b.hy + r);
                let (px, py) = (ex - lx.abs(), ey - ly.abs());
                if px <= 0.0 || py <= 0.0 {
                    continue;
                }
                let sign = |v: f32| if v < 0.0 { -1.0 } else { 1.0 };
                p = if px < py {
                    b.at(sign(lx) * (ex + 1e-3), ly)
                } else {
                    b.at(lx, sign(ly) * (ey + 1e-3))
                };
                moved = true;
            }
            if !moved {
                break;
            }
        }
        p
    }

    /// Where a body of radius `r` of side `side` stepping from `from` to
    /// `to` ends: stopped short of a thin wall it would pass through, and
    /// out of every obstacle (sliding along it).
    pub fn resolve(&self, from: V2, to: V2, r: f32, side: u8) -> V2 {
        if !self.thin {
            return self.push_out(to, r, side);
        }
        let mut to = to;
        for list in [self.near(from), self.near(to)] {
            for &k in list {
                let b = &self.boxes[k as usize];
                if !b.stops(side) || b.hx.min(b.hy) >= THIN || b.inside(from, r) {
                    continue;
                }
                if let Some(t) = b.enters(from, to, r) {
                    to = from + (to - from) * (t - 1e-3).max(0.0);
                }
            }
        }
        self.push_out(to, r, side)
    }

    /// If an obstacle that stops `side` stands within `LOOK` metres on the
    /// straight way from `me` to `target`, the corner to head for to walk
    /// round it (for a body of radius `r`): the one of its two outermost
    /// corners, as seen from `me`, that makes the shorter way.
    pub fn steer(&self, me: V2, target: V2, r: f32, side: u8) -> Option<V2> {
        let d = target - me;
        let len = d.len();
        if len < 0.05 {
            return None;
        }
        let dir = d * (1.0 / len);
        let look = len.min(LOOK);
        let end = me + dir * look;
        let mut hit: Option<(f32, usize)> = None;
        let (a, b) = (self.near(me), self.near(end));
        let both = if std::ptr::eq(a, b) {
            [a, &[][..]]
        } else {
            [a, b]
        };
        for list in both {
            for &k in list {
                let b = &self.boxes[k as usize];
                if !b.stops(side) || b.inside(me, r) {
                    continue;
                }
                if let Some(t) = b.enters(me, end, r) {
                    if hit.is_none_or(|h| t < h.0 || (t == h.0 && (k as usize) < h.1)) {
                        hit = Some((t, k as usize));
                    }
                }
            }
        }
        let (_, k) = hit?;
        let b = &self.boxes[k];
        let m = r + 0.6;
        let corners = [
            b.at(b.hx + m, b.hy + m),
            b.at(-b.hx - m, b.hy + m),
            b.at(b.hx + m, -b.hy - m),
            b.at(-b.hx - m, -b.hy - m),
        ];
        // The outermost corners to the left and to the right of the way.
        let (mut left, mut right): (Option<(f32, V2)>, Option<(f32, V2)>) = (None, None);
        for c in corners {
            let to = c - me;
            let l = to.len();
            if l < 0.3 {
                continue;
            }
            let s = dir.cross(to) / l;
            let a = if to.dot(dir) >= 0.0 {
                s
            } else {
                2.0 * s.signum() - s
            };
            if a >= 0.0 {
                if left.is_none_or(|x| a > x.0) {
                    left = Some((a, c));
                }
            } else if right.is_none_or(|x| a < x.0) {
                right = Some((a, c));
            }
        }
        let way = |c: V2| (c - me).len() + (target - c).len();
        match (left, right) {
            (Some((_, l)), Some((_, r))) => Some(if way(l) <= way(r) { l } else { r }),
            (Some((_, c)), None) | (None, Some((_, c))) => Some(c),
            (None, None) => None,
        }
    }
}

/// A squad's way to its goal, kept between substeps.
#[derive(Clone, Debug, Default)]
pub(crate) struct Route {
    /// The corners left, the last at the goal.
    way: Vec<V2>,
    /// The goal it was planned for, on which obstacles, and whether it
    /// has been planned at all.
    goal: V2,
    version: u64,
    planned: bool,
    /// Battle seconds since sight lines were last checked: they are once
    /// every `SIGHT` seconds.
    since: f32,
}

/// How often a route checks its sight lines, battle seconds.
const SIGHT: f32 = 1.0;

impl Route {
    /// The point to head for now, on the way from `at` to `goal`, `dt`
    /// battle seconds after the last call.
    pub(crate) fn next(&mut self, nav: &Nav, at: V2, goal: V2, side: u8, dt: f32) -> V2 {
        let moved = (goal - self.goal).len();
        self.since += dt;
        let look = self.since >= SIGHT;
        if look {
            self.since = 0.0;
        }
        if !self.planned || self.version != nav.version() {
            self.plan(nav, at, goal, side);
        } else if moved > 0.01 && nav.open(goal, side) {
            // The goal moved (an enemy squad walks): the last leg follows
            // it, as long as it can see it from the corner before (checked
            // once a second; the line to a goal straight ahead is checked
            // below).
            let n = self.way.len();
            if n < 2 {
                self.way = vec![goal];
                self.goal = goal;
            } else if look {
                if nav.clear_line(self.way[n - 2], goal, side) {
                    self.way[n - 1] = goal;
                    self.goal = goal;
                } else {
                    self.plan(nav, at, goal, side);
                }
            }
        }
        while self.way.len() > 1
            && ((self.way[0] - at).len() < 1.5 || (look && nav.clear_line(at, self.way[1], side)))
        {
            self.way.remove(0);
        }
        // Heading straight for the goal: plan again once the way closes
        // (the goal walked behind a house).
        if self.way.len() == 1 && look && !nav.clear_line(at, self.way[0], side) {
            self.plan(nav, at, goal, side);
        }
        self.way.first().copied().unwrap_or(goal)
    }

    fn plan(&mut self, nav: &Nav, at: V2, goal: V2, side: u8) {
        self.planned = true;
        self.goal = goal;
        self.version = nav.version();
        self.since = 0.0;
        self.way = if nav.open(goal, side) && nav.clear_line(at, goal, side) {
            vec![goal]
        } else {
            nav.path(at, goal, side).unwrap_or_else(|| vec![goal])
        };
    }

    /// Forget the way: the next call plans anew.
    pub(crate) fn clear(&mut self) {
        self.planned = false;
        self.way.clear();
    }
}
