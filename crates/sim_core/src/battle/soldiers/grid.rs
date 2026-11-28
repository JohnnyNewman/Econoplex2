//! A uniform grid over the field, one per side, rebuilt every substep by a
//! counting sort: count the soldiers in each cell, turn the counts into prefix
//! sums, write the soldier indices into one array. O(n), no allocation after
//! the first substep, and each cell lists its soldiers in index order.
//!
//! Only the cells within the box around the side's soldiers are counted, so
//! a small battle on a big field costs little.
//!
//! Soldiers are listed in cells half as wide as the grid's (`cell`), four
//! to a cell, so that a search of a metre reads a quarter of the soldiers
//! a whole cell holds. Searches still visit soldiers in the order of the
//! grid's cells: row by row, cell by cell, and by index within a cell (the
//! soldiers of a cell's four quarters merged by index). Sums over the
//! soldiers a search finds, and the first of equals it picks, come out as
//! they did with whole cells.

use super::V2;

pub(super) struct Grid {
    /// The grid's cells (`cell`): their size's inverse, and the last cell
    /// across and down.
    inv: f32,
    xmax: f32,
    ymax: f32,
    /// The same for the quarter cells soldiers are listed in.
    qinv: f32,
    qxmax: f32,
    qymax: f32,
    /// The box of quarter cells this build covers: first across and down,
    /// and its size. Empty when nobody is listed.
    x0: usize,
    y0: usize,
    bw: usize,
    bh: usize,
    /// `start[c]..start[c + 1]` are box quarter `c`'s entries in `items`.
    start: Vec<u32>,
    items: Vec<u32>,
    /// Where each entry of `items` stood when listed, across and down, in
    /// the same order, so a search reads positions in a row, eight at a time.
    ax: Vec<f32>,
    ay: Vec<f32>,
    /// The soldiers listed for this build, and the box of quarters around them.
    listed: Vec<Listed>,
    lo: (u32, u32),
    hi: (u32, u32),
}

/// A soldier listed for a build: their field quarter, index and place.
#[derive(Clone, Copy)]
struct Listed {
    x: u32,
    y: u32,
    i: u32,
    p: V2,
}

/// The most entries a cell's quarters hold that a search sorts on the
/// stack; a cell holding more sorts on the heap.
const ON_STACK: usize = 64;

impl Grid {
    pub(super) fn new(width: f32, height: f32, cell: f32) -> Self {
        let w = (width / cell).ceil().max(1.0) as usize;
        let h = (height / cell).ceil().max(1.0) as usize;
        let q = cell * 0.5;
        let qw = (width / q).ceil().max(1.0) as usize;
        let qh = (height / q).ceil().max(1.0) as usize;
        Self {
            inv: 1.0 / cell,
            xmax: (w - 1) as f32,
            ymax: (h - 1) as f32,
            qinv: 1.0 / q,
            qxmax: (qw - 1) as f32,
            qymax: (qh - 1) as f32,
            x0: 0,
            y0: 0,
            bw: 0,
            bh: 0,
            start: vec![0],
            items: Vec::new(),
            ax: Vec::new(),
            ay: Vec::new(),
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

    /// The field quarter a point falls in, the same way. A quarter's cell
    /// is half its coordinates: the sizes are a power of two apart, so the
    /// products are too, and the field's edge cells hold its edge quarters.
    #[inline]
    fn quarter(&self, p: V2) -> (usize, usize) {
        let x = (p.x * self.qinv).max(0.0).min(self.qxmax) as u32;
        let y = (p.y * self.qinv).max(0.0).min(self.qymax) as u32;
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
        let (x, y) = self.quarter(p);
        debug_assert_eq!((x / 2, y / 2), self.cell(p));
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

    /// Sort the listed soldiers into their quarters.
    pub(super) fn finish(&mut self) {
        let n = self.listed.len();
        if n == 0 {
            (self.bw, self.bh) = (0, 0);
            self.start.clear();
            self.start.push(0);
            self.items.clear();
            self.ax.clear();
            self.ay.clear();
            self.ax.resize(8, f32::INFINITY);
            self.ay.resize(8, f32::INFINITY);
            return;
        }
        let (x0, y0) = (self.lo.0 as usize, self.lo.1 as usize);
        (self.x0, self.y0) = (x0, y0);
        let bw = self.hi.0 as usize - x0 + 1;
        (self.bw, self.bh) = (bw, self.hi.1 as usize - y0 + 1);
        let cells = bw * self.bh;
        let box_cell = |e: &Listed| (e.y as usize - y0) * bw + (e.x as usize - x0);
        // Count quarter c's soldiers in start[c + 2] and sum, so that
        // start[c + 1] is where quarter c begins; writing moves it on to
        // where c ends, which is where c + 1 begins.
        self.start.clear();
        self.start.resize(cells + 2, 0);
        // Each entry's box quarter, worked out once, takes the place of its
        // field quarter across.
        for e in &mut self.listed {
            let c = box_cell(e);
            e.x = c as u32;
            self.start[c + 2] += 1;
        }
        let mut sum = 0;
        for s in &mut self.start[2..] {
            sum += *s;
            *s = sum;
        }
        // Every entry is written below, so the old ones need no clearing.
        self.items.resize(n, 0);
        // Eight past the end, so a search reads every quarter eight at a
        // time (the padding is never in a quarter).
        self.ax.resize(n + 8, f32::INFINITY);
        self.ay.resize(n + 8, f32::INFINITY);
        for e in &self.listed {
            let at = &mut self.start[e.x as usize + 1];
            let to = *at as usize;
            *at += 1;
            self.items[to] = e.i;
            self.ax[to] = e.p.x;
            self.ay[to] = e.p.y;
        }
        self.start.pop();
    }

    /// Whether anybody stands in the cells around `(x, y)`, itself included.
    #[inline]
    pub(super) fn anyone_around(&self, at: (usize, usize)) -> bool {
        self.anyone_around_box(at, at)
    }

    /// Whether anybody stands in the cells from `(xa, ya)` to `(xb, yb)`, or
    /// in the cells around them.
    #[inline]
    pub(super) fn anyone_around_box(
        &self,
        (xa, ya): (usize, usize),
        (xb, yb): (usize, usize),
    ) -> bool {
        // Their quarters, and the quarters of the cells around them.
        let (xa, ya) = (2 * xa.saturating_sub(1), 2 * ya.saturating_sub(1));
        let (xb, yb) = (2 * xb + 3, 2 * yb + 3);
        if self.bw == 0 {
            return false;
        }
        let (bx1, by1) = (self.x0 + self.bw - 1, self.y0 + self.bh - 1);
        let (xa, ya, xb, yb) = (xa.max(self.x0), ya.max(self.y0), xb.min(bx1), yb.min(by1));
        if xa > xb || ya > yb {
            return false;
        }
        let (xa, xb) = (xa - self.x0, xb - self.x0);
        (ya - self.y0..=yb - self.y0).any(|y| {
            let r = y * self.bw;
            self.start[r + xb + 1] > self.start[r + xa]
        })
    }

    /// Call `f` with every soldier listed in the cells within `r` of `p`
    /// (and so some a little further), in the grid's order (see the
    /// module's comment), until it returns false.
    #[inline]
    pub(super) fn each_near(&self, p: V2, r: f32, mut f: impl FnMut(usize) -> bool) {
        self.walk::<false>(p, r, f32::INFINITY, |j, _| f(j).then_some(f32::INFINITY));
    }

    /// Whether `f` holds for any soldier [`Grid::each_near`] calls for.
    #[inline]
    pub(super) fn any_near(&self, p: V2, r: f32, mut f: impl FnMut(usize) -> bool) -> bool {
        let mut found = false;
        self.each_near(p, r, |j| {
            found = f(j);
            !found
        });
        found
    }

    /// Call `f` with every soldier in the cells within `r` of `p` whose
    /// squared distance from it is at most `r2`, and where they stood when
    /// listed, in the grid's order. `f` returns the bound for the soldiers
    /// after it, `r2` or less: a search that wants only the nearest need not
    /// hear of anyone further than the nearest so far. (A soldier past the
    /// bound may still be called for: `f` must check.)
    ///
    /// Only the quarters that can hold a soldier within `r2` are read. In a
    /// melee most of the soldiers in them still stand further than the
    /// bound, in no order a branch could guess. So the distances are worked
    /// out eight at a time into a mask, without a branch, and `f` is called
    /// only for the soldiers the mask keeps.
    #[inline]
    pub(super) fn each_within(&self, p: V2, r: f32, r2: f32, mut f: impl FnMut(usize, V2) -> f32) {
        self.walk::<true>(p, r, r2, |j, q| Some(f(j, q)));
    }

    /// The walk behind both searches: the cells within `r` of `p`, in the
    /// grid's order, and in each the soldiers of its quarters merged by
    /// index. With `NEAR`, only the quarters within `r2` of `p` and the
    /// soldiers within the bound, which starts at `r2`; `f` returns the next
    /// bound, or `None` to stop.
    #[inline(always)]
    fn walk<const NEAR: bool>(
        &self,
        p: V2,
        r: f32,
        r2: f32,
        mut f: impl FnMut(usize, V2) -> Option<f32>,
    ) {
        if self.bw == 0 {
            return;
        }
        let (cxa, cya) = self.cell(V2::new(p.x - r, p.y - r));
        let (cxb, cyb) = self.cell(V2::new(p.x + r, p.y + r));
        // The quarters to read: all of the cells', or (searching by
        // distance) those within `r2`, with a little to spare for rounding.
        let (mut qxa, mut qya, mut qxb, mut qyb) = (2 * cxa, 2 * cya, 2 * cxb + 1, 2 * cyb + 1);
        if NEAR {
            let reach = r2.sqrt() * (1.0 + 1e-4) + 1e-3;
            let (xa, ya) = self.quarter(V2::new(p.x - reach, p.y - reach));
            let (xb, yb) = self.quarter(V2::new(p.x + reach, p.y + reach));
            (qxa, qya, qxb, qyb) = (qxa.max(xa), qya.max(ya), qxb.min(xb), qyb.min(yb));
        }
        // Within the box.
        let (bx1, by1) = (self.x0 + self.bw - 1, self.y0 + self.bh - 1);
        let (qxa, qya) = (qxa.max(self.x0), qya.max(self.y0));
        let (qxb, qyb) = (qxb.min(bx1), qyb.min(by1));
        if qxa > qxb || qya > qyb {
            return;
        }
        let mut bound = r2;
        let mut stack = [0u64; ON_STACK];
        let mut heap: Vec<u64> = Vec::new();
        for cy in qya / 2..=qyb / 2 {
            let (ya, yb) = ((2 * cy).max(qya), (2 * cy + 1).min(qyb));
            for cx in qxa / 2..=qxb / 2 {
                let (xa, xb) = ((2 * cx).max(qxa), (2 * cx + 1).min(qxb));
                // The cell's quarters to read that hold anybody.
                let mut quarters = [(0usize, 0usize); 4];
                let (mut held, mut used) = (0, 0);
                for y in ya..=yb {
                    let row = (y - self.y0) * self.bw - self.x0;
                    for x in xa..=xb {
                        let (s, e) = (
                            self.start[row + x] as usize,
                            self.start[row + x + 1] as usize,
                        );
                        if e > s {
                            quarters[used] = (s, e);
                            used += 1;
                            held += e - s;
                        }
                    }
                }
                // One quarter: its soldiers are in index order already.
                if used <= 1 {
                    if used == 1 {
                        let (s, e) = quarters[0];
                        if !self.span::<NEAR>(p, s, e, &mut bound, &mut f) {
                            return;
                        }
                    }
                    continue;
                }
                // Several: the soldiers each keeps, merged by index, as
                // index and entry in one number.
                let keep: &mut [u64] = if held <= ON_STACK {
                    &mut stack[..held]
                } else {
                    heap.clear();
                    heap.resize(held, 0);
                    &mut heap[..]
                };
                let mut n = 0;
                for &(s, e) in &quarters[..used] {
                    self.gather::<NEAR>(p, (s, e), bound, keep, &mut n);
                }
                let keep = &mut keep[..n];
                // Few, and in runs already in order: by insertion.
                for k in 1..keep.len() {
                    let e = keep[k];
                    let mut m = k;
                    while m > 0 && keep[m - 1] > e {
                        keep[m] = keep[m - 1];
                        m -= 1;
                    }
                    keep[m] = e;
                }
                for &e in keep.iter() {
                    let e = e as u32 as usize;
                    match f(self.items[e] as usize, V2::new(self.ax[e], self.ay[e])) {
                        Some(b) => bound = b,
                        None => return,
                    }
                }
            }
        }
    }

    /// The entries `a..b` within the bound (all, without `NEAR`), eight at
    /// a time, each into `keep[n]` as its soldier's index and its entry.
    #[inline(always)]
    fn gather<const NEAR: bool>(
        &self,
        p: V2,
        (a, b): (usize, usize),
        bound: f32,
        keep: &mut [u64],
        n: &mut usize,
    ) {
        let key = |e: usize| (u64::from(self.items[e]) << 32) | e as u64;
        if !NEAR {
            for e in a..b {
                keep[*n] = key(e);
                *n += 1;
            }
            return;
        }
        let mut k = a;
        while k < b {
            let mut mask = self.mask(p, k, bound);
            if b - k < 8 {
                mask &= (1 << (b - k)) - 1;
            }
            while mask != 0 {
                let l = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                keep[*n] = key(k + l);
                *n += 1;
            }
            k += 8;
        }
    }

    /// Call `f` for the entries `a..b` in order, within the bound (all,
    /// without `NEAR`); false once `f` asks to stop.
    #[inline(always)]
    fn span<const NEAR: bool>(
        &self,
        p: V2,
        a: usize,
        b: usize,
        bound: &mut f32,
        f: &mut impl FnMut(usize, V2) -> Option<f32>,
    ) -> bool {
        if !NEAR {
            for e in a..b {
                match f(self.items[e] as usize, V2::new(self.ax[e], self.ay[e])) {
                    Some(x) => *bound = x,
                    None => return false,
                }
            }
            return true;
        }
        let mut k = a;
        while k < b {
            let mut mask = self.mask(p, k, *bound);
            if b - k < 8 {
                mask &= (1 << (b - k)) - 1;
            }
            while mask != 0 {
                let l = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let e = k + l;
                match f(self.items[e] as usize, V2::new(self.ax[e], self.ay[e])) {
                    Some(x) => *bound = x,
                    None => return false,
                }
            }
            k += 8;
        }
        true
    }

    /// Which of the eight entries from `k` stand within `bound` of `p`, as
    /// bits: the same sums as `(p - q).len2()`, so the same answers.
    #[inline(always)]
    fn mask(&self, p: V2, k: usize, bound: f32) -> u32 {
        let (bx, by): (&[f32; 8], &[f32; 8]) = (
            self.ax[k..k + 8].try_into().expect("eight"),
            self.ay[k..k + 8].try_into().expect("eight"),
        );
        let mut mask = 0u32;
        for l in 0..8 {
            let (dx, dy) = (p.x - bx[l], p.y - by[l]);
            mask |= u32::from(dx * dx + dy * dy <= bound) << l;
        }
        mask
    }
}
