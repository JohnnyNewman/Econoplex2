//! Phase 2, the squad controllers: each squad turns its order into an anchor
//! (the centre of its front rank), a facing and a formation, and gives every
//! soldier a slot: anchor + rotate(facing) * (file * spacing, -rank * spacing).
//! The squad's judgment on `Auto` is the squad model's.

use super::*;

/// Where a squad goes this substep.
struct Goal {
    /// The point it heads for, if it moves.
    to: Option<V2>,
    /// Where it looks while it stands.
    face: Option<V2>,
    /// The enemy squad it closes with.
    target: Option<usize>,
    /// Marching or running pace, metres per second.
    pace: f32,
    /// Backing away: it keeps facing `face` as it goes.
    backing: bool,
}

impl SoldierBattle {
    #[inline(never)]
    pub(super) fn command(&mut self) {
        let dt = self.params.soldiers.substep;
        self.find_near_foes();
        for c in 0..self.squads.len() {
            if !self.squads[c].fighting() || self.squads[c].in_line == 0 {
                continue;
            }
            let goal = self.goal(c);
            let s = &self.squads[c];
            let near = goal
                .target
                .is_some_and(|j| self.ahead(c, j) < self.params.charge_distance + 50.0);
            let formation = self.formation_for(c, goal.to, near);
            let sp = &self.params.soldiers;
            // Turn toward the goal, or toward what it faces while it stands.
            let want = goal
                .to
                .filter(|_| !goal.backing)
                .map(|g| g - s.anchor)
                .filter(|d| d.len2() > 1.0)
                .or(goal.face.map(|f| f - s.centre))
                .map(|d| d.norm_or(s.facing))
                .unwrap_or(s.facing);
            let max = sp.turn_rate * dt;
            let facing = s.facing.turned(want, max.cos(), max.sin());
            // Step toward the goal. Once in contact, no further ahead of the
            // front rank than `lead`: the line presses, it does not run on.
            // Riders charge home through the enemy.
            let mut anchor = s.anchor;
            if let Some(g) = goal.to {
                let d = g - anchor;
                let step = (goal.pace * s.vigour * dt).min(d.len());
                anchor += d.norm_or(facing) * step;
                let ahead = (anchor - s.front).dot(facing);
                if s.in_contact > 0 && !s.mounted() && ahead > sp.lead {
                    anchor -= facing * (ahead - sp.lead);
                }
            }
            if let Some(w) = self.field.wall_x {
                if s.spec.behind_wall {
                    anchor.x = anchor.x.max(w + WALL_HALF + 0.5);
                } else if s.anchor.x <= w {
                    anchor.x = anchor.x.min(w - WALL_HALF - 0.5);
                }
            }
            let shoot_at = self.shoot_target(c);
            let s = &mut self.squads[c];
            s.anchor = anchor;
            s.facing = facing;
            s.target = goal.target;
            s.shoot_at = shoot_at;
            if formation != s.formation || std::mem::take(&mut s.reform_due) {
                self.reform(c, formation);
            }
        }
    }

    /// Each commanded squad's nearest fighting enemy into `near_foe`, as
    /// [`Self::nearest`] would find it, from one list of the squads fighting.
    fn find_near_foes(&mut self) {
        let fighting: Vec<(u8, usize, V2)> = (0..self.squads.len())
            .filter(|&j| self.squads[j].fighting())
            .map(|j| (self.squads[j].spec.side, j, self.squads[j].centre))
            .collect();
        let mut near_foe = std::mem::take(&mut self.near_foe);
        near_foe.clear();
        near_foe.extend(self.squads.iter().map(|s| {
            if !s.fighting() || s.in_line == 0 {
                return None;
            }
            let (side, at) = (s.spec.side, s.centre);
            closest(
                fighting
                    .iter()
                    .filter(|e| e.0 != side)
                    .map(|&(_, j, q)| ((at - q).len(), j)),
            )
        }));
        self.near_foe = near_foe;
    }

    fn dist(&self, a: usize, b: usize) -> f32 {
        (self.squads[a].centre - self.squads[b].centre).len()
    }

    /// Distance from a squad's front to the edge of squad `b`, as a disc.
    /// It does not change when the squad changes formation.
    fn ahead(&self, a: usize, b: usize) -> f32 {
        (self.squads[a].anchor - self.squads[b].centre).len() - self.squads[b].radius()
    }

    fn enemies(&self, c: usize) -> impl Iterator<Item = usize> + '_ {
        let side = self.squads[c].spec.side;
        (0..self.squads.len()).filter(move |&j| self.squads[j].spec.side != side)
    }

    fn nearest(&self, c: usize, keep: impl Fn(&SquadCtl) -> bool) -> Option<usize> {
        closest(
            self.enemies(c)
                .filter(|&j| keep(&self.squads[j]))
                .map(|j| (self.dist(c, j), j)),
        )
    }

    fn home_x(&self, c: usize) -> f32 {
        if self.squads[c].spec.side == 0 {
            -10.0
        } else {
            self.field.width + 10.0
        }
    }

    /// A move the squad has reached gives way to its next order, if any.
    fn advance(&mut self, c: usize) {
        let s = &mut self.squads[c];
        while let Order::MoveTo { anchor, .. } = s.order() {
            if s.orders.len() < 2 || (V2::new(anchor.0, anchor.1) - s.anchor).len() > ARRIVED {
                break;
            }
            s.orders.remove(0);
            s.start_order();
        }
    }

    /// Where a squad goes, by its order or its own judgment.
    fn goal(&mut self, c: usize) -> Goal {
        self.advance(c);
        let p = &self.params;
        let s = &self.squads[c];
        let (walk, run) = if s.mounted() {
            (p.horse_walk_speed, p.horse_charge_speed)
        } else {
            (p.walk_speed, p.run_speed)
        };
        let hold = |face: Option<V2>| Goal {
            to: None,
            face,
            target: None,
            pace: 0.0,
            backing: false,
        };
        let nearest_face = self.near_foe[c].map(|j| self.squads[j].centre);
        // Riders that ran onto a spear wall ride back, then charge again.
        if let Some(r) = s.rally {
            if (r - s.anchor).len() > 2.0 {
                return Goal {
                    to: Some(r),
                    face: nearest_face,
                    target: None,
                    pace: walk,
                    backing: false,
                };
            }
            self.squads[c].rally = None;
            self.squads[c].contact_time = 0.0;
            return hold(nearest_face);
        }
        // Locked in melee. Two foot squads stay pressed together as their
        // ranks thin; riders, and men on foot facing them, hold their ground.
        if s.in_contact > 0 {
            if let Some(j) = s.fighting_with {
                let q = &self.squads[j];
                if s.mounted() && !q.mounted() && q.spec.kit.reach + 0.01 >= s.spec.kit.reach {
                    let dt = p.soldiers.substep;
                    let s = &mut self.squads[c];
                    s.contact_time += dt;
                    if s.contact_time >= p.soldiers.rally_after {
                        s.rally = Some(s.anchor - s.facing * p.soldiers.rally_distance);
                    }
                    return hold(Some(self.squads[j].centre));
                }
                if !s.mounted() && !q.mounted() {
                    return Goal {
                        to: Some(q.centre),
                        face: None,
                        target: Some(j),
                        pace: walk,
                        backing: false,
                    };
                }
                return hold(Some(q.centre));
            }
        }
        let valid = |j: usize| j < self.squads.len() && self.squads[j].on_field();
        let order = s.order();
        // Skirmishers back away from enemies who come close, facing them,
        // unless told to close with one or to go somewhere.
        if s.stance.fire == Fire::Skirmish
            && s.has_bow()
            && !matches!(
                order,
                Order::Attack(_) | Order::MoveTo { .. } | Order::Withdraw
            )
        {
            if let Some(j) = self.near_foe[c].filter(|&j| self.ahead(c, j) < p.skirmish_distance) {
                let e = self.squads[j].centre;
                return Goal {
                    to: Some(s.anchor + (s.anchor - e).norm_or(-s.facing) * 20.0),
                    face: Some(e),
                    target: None,
                    pace: run,
                    backing: true,
                };
            }
        }
        let mut pace = None;
        let goal: Option<(V2, Option<usize>)> = match order {
            Order::Hold => {
                // Spears face the riders they brace against.
                let riders = self
                    .nearest(c, |q| q.fighting() && q.mounted())
                    .map(|j| self.squads[j].centre);
                return hold(riders.or(nearest_face));
            }
            Order::MoveTo {
                anchor,
                facing,
                run: r,
                ..
            } => {
                let to = V2::new(anchor.0, anchor.1);
                if (to - s.anchor).len() <= ARRIVED {
                    // There: stand, turned the way it was told.
                    return hold(facing.map(|(x, y)| s.centre + V2::new(x, y) * 100.0));
                }
                pace = Some(if r { run } else { walk });
                Some((to, None))
            }
            Order::Withdraw => Some((V2::new(self.home_x(c), s.centre.y), None)),
            Order::Attack(j) if valid(j) => Some((self.squads[j].centre, Some(j))),
            Order::Shoot(j) if valid(j) => {
                if s.has_bow() && self.dist(c, j) > 0.9 * s.spec.kit.range {
                    Some((self.squads[j].centre, Some(j)))
                } else {
                    return hold(Some(self.squads[j].centre));
                }
            }
            _ => self.auto_goal(c),
        };
        let Some((to, target)) = goal else {
            let riders = self
                .nearest(c, |q| q.fighting() && q.mounted())
                .filter(|&j| self.dist(c, j) < p.soldiers.brace_riders)
                .map(|j| self.squads[j].centre);
            return hold(riders.or(nearest_face));
        };
        // An aggressive side's squads run in from further out.
        let eager = order == Order::Auto && self.doctrine_of(c).posture == Posture::Aggressive;
        let charge = p.charge_distance * if eager { 2.0 } else { 1.0 };
        let near = target.is_some_and(|j| self.ahead(c, j) < charge);
        Goal {
            to: Some(to),
            face: None,
            target,
            pace: pace.unwrap_or(if near { run } else { walk }),
            backing: false,
        }
    }

    fn doctrine_of(&self, c: usize) -> Doctrine {
        self.doctrine[(self.squads[c].spec.side as usize).min(1)]
    }

    /// A squad's own judgment, as on the squad model: where to go, and whom
    /// to close with.
    fn auto_goal(&self, c: usize) -> Option<(V2, Option<usize>)> {
        let s = &self.squads[c];
        if s.spec.behind_wall {
            return None;
        }
        let kit = &s.spec.kit;
        // A cautious side's squads hold until an enemy comes close.
        if self.doctrine_of(c).posture == Posture::Cautious
            && self.near_foe[c].is_none_or(|j| self.ahead(c, j) > self.params.cautious_wait)
        {
            return None;
        }
        let target = if kit.mounted {
            // Riders look for archers and fleeing squads (if they chase
            // them), and steer clear of a spear wall while softer targets
            // are about.
            let pursue = s.chases();
            let score = |j: usize| {
                let q = &self.squads[j];
                let mut d = self.dist(c, j);
                if q.state == SquadState::Routing && d > 150.0 {
                    d += 1000.0;
                }
                if !q.mounted() && q.spec.kit.reach >= kit.reach && q.fighting() {
                    d += 250.0;
                }
                if q.has_bow() {
                    d -= 50.0;
                }
                d
            };
            closest(
                self.enemies(c)
                    .filter(|&j| {
                        let q = &self.squads[j];
                        q.fighting() || (pursue && q.on_field())
                    })
                    .map(|j| (score(j), j)),
            )
        } else {
            self.near_foe[c]
        }?;
        // Archers stand and shoot once in range.
        if s.has_bow() && self.dist(c, target) <= 0.9 * kit.range {
            return None;
        }
        // Spearmen brace when riders are coming.
        if !kit.mounted && kit.reach >= 2.5 {
            let riders = self.enemies(c).any(|j| {
                let r = &self.squads[j];
                r.fighting() && r.mounted() && self.dist(c, j) < self.params.soldiers.brace_riders
            });
            if riders {
                return None;
            }
        }
        Some((self.squads[target].centre, Some(target)))
    }

    /// The squad its archers shoot: the ordered one in range, else the nearest.
    fn shoot_target(&self, c: usize) -> Option<usize> {
        let s = &self.squads[c];
        if !s.has_bow() || s.stance.fire == Fire::Hold {
            return None;
        }
        let range = s.spec.kit.range;
        let ok = |j: usize| {
            let q = &self.squads[j];
            (q.fighting() || (s.pursues() && q.on_field())) && self.dist(c, j) <= range
        };
        match s.order() {
            Order::Shoot(j) | Order::Attack(j) if j < self.squads.len() && ok(j) => Some(j),
            _ => closest(
                self.enemies(c)
                    .filter(|&j| ok(j))
                    .map(|j| (self.dist(c, j), j)),
            ),
        }
    }

    /// The formation a squad takes for its goal on its own judgment.
    pub(super) fn formation_for(&self, c: usize, to: Option<V2>, near: bool) -> Formation {
        let s = &self.squads[c];
        let kit = &s.spec.kit;
        let sp = &self.params.soldiers;
        let loose = match s.stance.spacing {
            Spacing::Own => kit.range > 0.0 && !kit.mounted,
            Spacing::Tight => false,
            Spacing::Loose => true,
        };
        let spread = if loose {
            Formation::Loose
        } else {
            Formation::Line
        };
        if kit.mounted {
            return if near { Formation::Wedge } else { spread };
        }
        if kit.reach >= 2.5 {
            let riders = self.enemies(c).any(|j| {
                let r = &self.squads[j];
                r.fighting() && r.mounted() && self.dist(c, j) < sp.brace_riders
            });
            if s.order() == Order::Hold || riders {
                return Formation::Braced;
            }
        }
        // A squad whose front the player set keeps it on the march.
        let marching = s.width.is_none()
            && matches!(s.order(), Order::MoveTo { .. } | Order::Withdraw)
            && to.is_some_and(|g| (g - s.anchor).len() > sp.column_beyond);
        if marching {
            return Formation::Column;
        }
        spread
    }

    /// The soldiers of a squad standing in its line.
    fn line_of(&self, c: usize) -> Vec<usize> {
        let s = &self.squads[c];
        (s.first..s.first + s.spec.members.len())
            .filter(|&i| self.state[i].in_line())
            .collect()
    }

    fn max_files(&self, c: usize, formation: Formation) -> usize {
        let p = &self.params;
        let s = &self.squads[c];
        let base = if s.mounted() {
            p.horse_spacing
        } else {
            p.foot_spacing
        };
        let spacing = if formation == Formation::Loose {
            p.soldiers.loose_spacing
        } else {
            base
        };
        match (formation, s.width) {
            (Formation::Column, _) => p.soldiers.column_files as usize,
            (Formation::Wedge, _) | (_, None) => (p.frontage / spacing) as usize,
            // The front the player set.
            (_, Some(w)) => (w / spacing).round() as usize,
        }
        .max(1)
    }

    /// Take a new formation: every soldier in line gets a new slot, the ones
    /// in front the front slots, so nobody crosses the squad.
    pub(super) fn reform(&mut self, c: usize, formation: Formation) {
        let p = &self.params;
        let base = if self.squads[c].mounted() {
            p.horse_spacing
        } else {
            p.foot_spacing
        };
        let spacing = if formation == Formation::Loose {
            p.soldiers.loose_spacing
        } else {
            base
        };
        let max_files = self.max_files(c, formation);
        let mut line = self.line_of(c);
        let n = line.len();
        let s = &mut self.squads[c];
        s.formation = formation;
        s.spacing = spacing;
        if n == 0 {
            return;
        }
        // The slots, front rank first and left to right in each rank.
        let mut slots: Vec<(i16, u16)> = Vec::with_capacity(n);
        let files = max_files.min(n);
        if formation == Formation::Wedge {
            let mut r = 0u16;
            while slots.len() < n {
                let m = (2 * r as usize + 1).min(n - slots.len());
                let first = -((m as i16 - 1) / 2);
                slots.extend((0..m as i16).map(|f| (first + f, r)));
                r += 1;
            }
        } else {
            let ranks = n.div_ceil(files);
            for r in 0..ranks {
                let m = files.min(n - r * files);
                let first = ((files - m) / 2) as i16;
                slots.extend((0..m as i16).map(|f| (first + f, r as u16)));
            }
        }
        s.files = files as u16;
        // Soldiers by how far back they stand, then rank by rank left to right.
        let (anchor, facing) = (s.anchor, s.facing);
        let back = |i: usize| -(self.pos[i] - anchor).dot(facing);
        let lat = |i: usize| (self.pos[i] - anchor).dot(facing.right());
        line.sort_by(|&a, &b| back(a).total_cmp(&back(b)).then(a.cmp(&b)));
        let mut k = 0;
        while k < n {
            let r = slots[k].1;
            let end = k + slots[k..].iter().take_while(|s| s.1 == r).count();
            line[k..end].sort_by(|&a, &b| lat(a).total_cmp(&lat(b)).then(a.cmp(&b)));
            for j in k..end {
                self.file[line[j]] = slots[j].0;
                self.rank[line[j]] = slots[j].1;
            }
            k = end;
        }
        // Who stands ahead of whom: the same file one rank on (in a wedge,
        // the nearest file of the narrower rank).
        let mut placed: Vec<((u16, i16), usize)> = line
            .iter()
            .map(|&i| ((self.rank[i], self.file[i]), i))
            .collect();
        placed.sort_unstable();
        for &((r, f), i) in &placed {
            self.ahead[i] = NONE;
            if r == 0 {
                continue;
            }
            let f = if formation == Formation::Wedge {
                f.clamp(-(r as i16 - 1), r as i16 - 1)
            } else {
                f
            };
            if let Ok(k) = placed.binary_search_by(|p| p.0.cmp(&(r - 1, f))) {
                self.ahead[i] = placed[k].1 as u32;
            }
        }
    }

    /// Fill the gaps losses left: in a line each file closes up from the rear,
    /// and the rear of the deepest files fills the shallowest, so the front
    /// stays whole until the squad is too small for it. Other formations
    /// take their slots again.
    pub(super) fn refill(&mut self, c: usize) {
        let formation = self.squads[c].formation;
        if matches!(formation, Formation::Wedge | Formation::Column) {
            self.reform(c, formation);
            return;
        }
        let line = self.line_of(c);
        let n = line.len();
        if n == 0 {
            return;
        }
        let w = self.max_files(c, formation).min(n);
        let files = (self.squads[c].files as usize).max(1);
        let mut cols: Vec<Vec<usize>> = vec![Vec::new(); files];
        for &i in &line {
            let f = (self.file[i].max(0) as usize).min(files - 1);
            cols[f].push(i);
        }
        for col in &mut cols {
            col.sort_by_key(|&i| (self.rank[i], i));
        }
        // Too few soldiers for the width: drop empty files, outside first.
        while cols.len() > w {
            if cols[0].is_empty() {
                cols.remove(0);
            } else if cols.last().is_some_and(Vec::is_empty) {
                cols.pop();
            } else if let Some(e) = cols.iter().position(Vec::is_empty) {
                cols.remove(e);
            } else {
                break;
            }
        }
        // The rear of the deepest file nearest the shallowest fills it.
        loop {
            let (lo, lo_len) = cols
                .iter()
                .enumerate()
                .map(|(f, c)| (f, c.len()))
                .min_by_key(|&(f, l)| (l, f))
                .unwrap_or((0, 0));
            let hi_len = cols.iter().map(Vec::len).max().unwrap_or(0);
            if hi_len <= lo_len + 1 {
                break;
            }
            let hi = (0..cols.len())
                .filter(|&f| cols[f].len() == hi_len)
                .min_by_key(|&f| (f.abs_diff(lo), f))
                .unwrap_or(0);
            if let Some(i) = cols[hi].pop() {
                cols[lo].push(i);
            }
        }
        for (f, col) in cols.iter().enumerate() {
            for (r, &i) in col.iter().enumerate() {
                self.file[i] = f as i16;
                self.rank[i] = r as u16;
                self.ahead[i] = if r == 0 { NONE } else { col[r - 1] as u32 };
            }
        }
        self.squads[c].files = cols.len() as u16;
    }
}

/// The squad of the least score, the lower index on a tie: each score taken once.
fn closest(scored: impl Iterator<Item = (f32, usize)>) -> Option<usize> {
    scored
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(_, j)| j)
}
