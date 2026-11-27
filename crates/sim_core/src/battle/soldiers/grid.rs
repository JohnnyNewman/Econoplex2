//! A uniform grid over the field, one per side, rebuilt every substep by a
//! counting sort: count the soldiers in each cell, turn the counts into prefix
//! sums, write the soldier indices into one array. O(n), no allocation after
//! the first substep, and each cell lists its soldiers in index order.
//!
//! Only the cells within the box around the side's soldiers are counted, so
//! a small battle on a big field costs little.

use super::V2;

pub(super) struct Grid {
    inv: f32,
    /// The last cell across and down.
    xmax: f32,
    ymax: f32,
    /// The box of cells this build covers: first cell across and down, and
    /// its size. Empty when nobody is listed.
    x0: usize,
    y0: usize,
    bw: usize,
    bh: usize,
    /// `start[c]..start[c + 1]` are box cell `c`'s entries in `items`.
    start: Vec<u32>,
    items: Vec<u32>,
    /// Where each entry of `items` stood when listed, in the same order, so
    /// a search reads positions in a row.
    at: Vec<V2>,
    /// The soldiers listed for this build, and the box of cells around them.
    listed: Vec<Listed>,
    lo: (u32, u32),
    hi: (u32, u32),
}

/// A soldier listed for a build: their field cell, index and place.
#[derive(Clone, Copy)]
struct Listed {
    x: u32,
    y: u32,
    i: u32,
    p: V2,
}

impl Grid {
    pub(super) fn new(width: f32, height: f32, cell: f32) -> Self {
        let w = (width / cell).ceil().max(1.0) as usize;
        let h = (height / cell).ceil().max(1.0) as usize;
        Self {
            inv: 1.0 / cell,
            xmax: (w - 1) as f32,
            ymax: (h - 1) as f32,
            x0: 0,
            y0: 0,
            bw: 0,
            bh: 0,
            start: vec![0],
            items: Vec::new(),
            at: Vec::new(),
            listed: Vec::new(),
            lo: (u32::MAX, u32::MAX),
            hi: (0, 0),
        }
    }

    /// The field cell a point falls in; points off the field fall in its edge cells.
    #[inline]
    pub(super) fn cell(&self, p: V2) -> (usize, usize) {
        // Clamped first, so the cast needs no checks: `max` takes the 0
        // when the product is not a number.
        let x = (p.x * self.inv).max(0.0).min(self.xmax) as u32;
        let y = (p.y * self.inv).max(0.0).min(self.ymax) as u32;
        (x as usize, y as usize)
    }

    /// Start a build: list soldiers with [`Grid::push`] in index order,
    /// then [`Grid::finish`].
    pub(super) fn clear(&mut self) {
        self.listed.clear();
        (self.lo, self.hi) = ((u32::MAX, u32::MAX), (0, 0));
    }

    /// List soldier `i` at `p`.
    #[inline]
    pub(super) fn push(&mut self, i: usize, p: V2) {
        let (x, y) = self.cell(p);
        let (x, y) = (x as u32, y as u32);
        self.lo = (self.lo.0.min(x), self.lo.1.min(y));
        self.hi = (self.hi.0.max(x), self.hi.1.max(y));
        self.listed.push(Listed {
            x,
            y,
            i: i as u32,
            p,
        });
    }

    /// Sort the listed soldiers into their cells.
    pub(super) fn finish(&mut self) {
        let n = self.listed.len();
        if n == 0 {
            (self.bw, self.bh) = (0, 0);
            self.start.clear();
            self.start.push(0);
            self.items.clear();
            self.at.clear();
            return;
        }
        let (x0, y0) = (self.lo.0 as usize, self.lo.1 as usize);
        (self.x0, self.y0) = (x0, y0);
        let bw = self.hi.0 as usize - x0 + 1;
        (self.bw, self.bh) = (bw, self.hi.1 as usize - y0 + 1);
        let cells = bw * self.bh;
        let box_cell = |e: &Listed| (e.y as usize - y0) * bw + (e.x as usize - x0);
        // Count cell c's soldiers in start[c + 2] and sum, so that start[c + 1]
        // is where cell c begins; writing moves it on to where c ends, which
        // is where c + 1 begins.
        self.start.clear();
        self.start.resize(cells + 2, 0);
        for e in &self.listed {
            self.start[box_cell(e) + 2] += 1;
        }
        let mut sum = 0;
        for s in &mut self.start[2..] {
            sum += *s;
            *s = sum;
        }
        // Every entry is written below, so the old ones need no clearing.
        self.items.resize(n, 0);
        self.at.resize(n, V2::ZERO);
        for e in &self.listed {
            let at = &mut self.start[box_cell(e) + 1];
            let to = *at as usize;
            *at += 1;
            self.items[to] = e.i;
            self.at[to] = e.p;
        }
        self.start.pop();
    }

    /// The box cells from field cells `(xa, ya)` to `(xb, yb)`, or `None`
    /// when they miss the box.
    #[inline]
    fn clip(&self, xa: usize, ya: usize, xb: usize, yb: usize) -> Option<[usize; 4]> {
        if self.bw == 0 {
            return None;
        }
        let (bx1, by1) = (self.x0 + self.bw - 1, self.y0 + self.bh - 1);
        let (xa, ya, xb, yb) = (xa.max(self.x0), ya.max(self.y0), xb.min(bx1), yb.min(by1));
        (xa <= xb && ya <= yb).then(|| [xa - self.x0, ya - self.y0, xb - self.x0, yb - self.y0])
    }

    /// The entries of box row `y`, cells `xa..=xb`.
    #[inline]
    fn row(&self, y: usize, xa: usize, xb: usize) -> &[u32] {
        let r = y * self.bw;
        &self.items[self.start[r + xa] as usize..self.start[r + xb + 1] as usize]
    }

    /// The entries of box row `y`, cells `xa..=xb`, with where they stood.
    #[inline]
    fn row_at(&self, y: usize, xa: usize, xb: usize) -> (&[u32], &[V2]) {
        let r = y * self.bw;
        let span = self.start[r + xa] as usize..self.start[r + xb + 1] as usize;
        (&self.items[span.clone()], &self.at[span])
    }

    /// Whether anybody stands in the cells around `(x, y)`, itself included.
    #[inline]
    pub(super) fn anyone_around(&self, (x, y): (usize, usize)) -> bool {
        let Some([xa, ya, xb, yb]) =
            self.clip(x.saturating_sub(1), y.saturating_sub(1), x + 1, y + 1)
        else {
            return false;
        };
        (ya..=yb).any(|y| {
            let r = y * self.bw;
            self.start[r + xb + 1] > self.start[r + xa]
        })
    }

    /// Every soldier within `r` of `p` (and some a little further), cell by
    /// cell in a fixed order.
    #[inline]
    pub(super) fn near(&self, p: V2, r: f32) -> impl Iterator<Item = usize> + '_ {
        let (xa, ya) = self.cell(V2::new(p.x - r, p.y - r));
        let (xb, yb) = self.cell(V2::new(p.x + r, p.y + r));
        let [xa, ya, xb, yb] = self.clip(xa, ya, xb, yb).unwrap_or([1, 1, 0, 0]);
        (ya..=yb).flat_map(move |y| self.row(y, xa, xb).iter().map(|&i| i as usize))
    }

    /// Call `f` with every soldier within `r` of `p` (and some a little
    /// further) and where they stood when listed, in the order of
    /// [`Grid::near`].
    #[inline]
    pub(super) fn each_near(&self, p: V2, r: f32, mut f: impl FnMut(usize, V2)) {
        let (xa, ya) = self.cell(V2::new(p.x - r, p.y - r));
        let (xb, yb) = self.cell(V2::new(p.x + r, p.y + r));
        let Some([xa, ya, xb, yb]) = self.clip(xa, ya, xb, yb) else {
            return;
        };
        for y in ya..=yb {
            let (items, at) = self.row_at(y, xa, xb);
            for (&i, &q) in items.iter().zip(at) {
                f(i as usize, q);
            }
        }
    }
}
