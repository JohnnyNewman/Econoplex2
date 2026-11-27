//! A uniform grid over the field, one per side, rebuilt every substep by a
//! counting sort: count the soldiers in each cell, turn the counts into prefix
//! sums, write the soldier indices into one array. O(n), no allocation after
//! the first substep, and each cell lists its soldiers in index order.
//!
//! Only the cells within the box around the side's soldiers are counted, so
//! a small battle on a big field costs little.

use super::V2;

pub(super) struct Grid {
    /// Cells across and down the field.
    w: usize,
    h: usize,
    inv: f32,
    /// The box of cells this build covers: first cell across and down, and
    /// its size. Empty when nobody is listed.
    x0: usize,
    y0: usize,
    bw: usize,
    bh: usize,
    /// `start[c]..start[c + 1]` are box cell `c`'s entries in `items`.
    start: Vec<u32>,
    items: Vec<u32>,
    /// Each listed soldier's field cell, the next free entry per cell, and
    /// the sorted list, while building.
    cells: Vec<(u32, u32)>,
    cursor: Vec<u32>,
    sorted: Vec<u32>,
}

impl Grid {
    pub(super) fn new(width: f32, height: f32, cell: f32) -> Self {
        let w = (width / cell).ceil().max(1.0) as usize;
        let h = (height / cell).ceil().max(1.0) as usize;
        Self {
            w,
            h,
            inv: 1.0 / cell,
            x0: 0,
            y0: 0,
            bw: 0,
            bh: 0,
            start: vec![0],
            items: Vec::new(),
            cells: Vec::new(),
            cursor: Vec::new(),
            sorted: Vec::new(),
        }
    }

    /// The field cell a point falls in; points off the field fall in its edge cells.
    #[inline]
    pub(super) fn cell(&self, p: V2) -> (usize, usize) {
        // Clamped as floats, so the cast needs no range checks.
        let x = (p.x * self.inv).clamp(0.0, (self.w - 1) as f32) as i32;
        let y = (p.y * self.inv).clamp(0.0, (self.h - 1) as f32) as i32;
        (x as usize, y as usize)
    }

    /// Start a build: list soldiers with [`Grid::push`] in index order,
    /// then [`Grid::finish`].
    pub(super) fn clear(&mut self) {
        self.cells.clear();
        self.items.clear();
    }

    /// List soldier `i` at `p`.
    #[inline]
    pub(super) fn push(&mut self, i: usize, p: V2) {
        let (x, y) = self.cell(p);
        self.cells.push((x as u32, y as u32));
        self.items.push(i as u32);
    }

    /// Sort the listed soldiers into their cells.
    pub(super) fn finish(&mut self) {
        if self.items.is_empty() {
            (self.bw, self.bh) = (0, 0);
            self.start.clear();
            self.start.push(0);
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for &(x, y) in &self.cells {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let (x0, y0) = (x0 as usize, y0 as usize);
        (self.x0, self.y0) = (x0, y0);
        (self.bw, self.bh) = (x1 as usize - x0 + 1, y1 as usize - y0 + 1);
        let cells = self.bw * self.bh;
        self.start.clear();
        self.start.resize(cells + 1, 0);
        for &(x, y) in &self.cells {
            let c = (y as usize - y0) * self.bw + (x as usize - x0);
            self.start[c + 1] += 1;
        }
        for c in 0..cells {
            self.start[c + 1] += self.start[c];
        }
        // Write in index order: `cursor` is each cell's next free entry.
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.start[..cells]);
        self.sorted.clear();
        self.sorted.resize(self.items.len(), 0);
        for (&(x, y), &i) in self.cells.iter().zip(&self.items) {
            let c = (y as usize - y0) * self.bw + (x as usize - x0);
            let at = &mut self.cursor[c];
            self.sorted[*at as usize] = i;
            *at += 1;
        }
        std::mem::swap(&mut self.items, &mut self.sorted);
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

    /// How many soldiers stand in the cells around `(x, y)`, itself included.
    #[inline]
    pub(super) fn around(&self, (x, y): (usize, usize)) -> u32 {
        let Some([xa, ya, xb, yb]) =
            self.clip(x.saturating_sub(1), y.saturating_sub(1), x + 1, y + 1)
        else {
            return 0;
        };
        (ya..=yb).map(|y| self.row(y, xa, xb).len() as u32).sum()
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
}
