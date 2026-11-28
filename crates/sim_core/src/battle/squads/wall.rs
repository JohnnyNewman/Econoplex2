//! The squad model at a town's wall (`battle/walls.rs`), as near as a disc
//! comes to the soldier model's: attackers form up against the piece they
//! assault and batter it, defenders man it from inside where the enemy
//! comes and strike down over it, and once it is breached the nearest go
//! to plug the breach while the rest fight through it as anywhere.

use super::*;
use crate::battle::walls::{ASSAULT_OFF, COVER, MAN_OFF, NEAR, OVER_WALL, PLUG, RIDERS_OFF};

impl SquadBattle {
    /// Each attacking squad picks the piece it assaults (or keeps its own
    /// while it stands); none while the wall is breached.
    pub(super) fn choose_assaults(&mut self) {
        let Some(ring) = self.ring.take() else {
            return;
        };
        let closed = self.field.wall_closed();
        for i in 0..self.squads.len() {
            let s = &self.squads[i];
            if s.spec.side != 0 || !closed || !s.fighting() {
                self.squads[i].assault = None;
                continue;
            }
            let keep = s
                .assault
                .filter(|&n| self.field.walls[ring.pieces[n].k].hp > 0.0);
            let at = soldiers::V2::new(s.spec.pos.0, s.spec.pos.1);
            self.squads[i].assault = keep.or_else(|| ring.assault(&self.field, at));
        }
        self.ring = Some(ring);
    }

    /// Where squad `i` goes on its own judgment because of the town's wall
    /// (`None`: the wall changes nothing; `Some(None)`: it stands).
    pub(super) fn wall_goal(&self, i: usize) -> Option<Option<(f32, f32, Option<usize>)>> {
        let ring = self.ring.as_ref()?;
        let s = &self.squads[i];
        let r = s.radius(&self.params);
        let at = soldiers::V2::new(s.spec.pos.0, s.spec.pos.1);
        let to = |v: soldiers::V2| Some(Some((v.x, v.y, None)));
        let foe = self.nearest(i, |q| q.fighting());
        if s.spec.side == 0 {
            // Archers come close enough that all of them reach the
            // defenders, who stand still behind the wall or in the breach.
            let near = |f: usize| self.dist(i, f) <= NEAR * s.spec.kit.range;
            if !self.field.wall_closed() {
                let f = foe.filter(|_| s.has_bow())?;
                let q = self.squads[f].spec.pos;
                return Some((!near(f)).then_some((q.0, q.1, None)));
            }
            if s.has_bow() || s.spec.kit.mounted {
                let f = foe?;
                let fc = soldiers::V2::new(self.squads[f].spec.pos.0, self.squads[f].spec.pos.1);
                if s.has_bow() && near(f) {
                    return Some(None);
                }
                let n = ring.nearest(&self.field, fc, false)?;
                let off = if s.spec.kit.mounted {
                    RIDERS_OFF
                } else {
                    0.5 * NEAR * s.spec.kit.range
                };
                return to(ring.pieces[n].beside(off + r));
            }
            let n = s.assault?;
            return to(ring.pieces[n].beside(ASSAULT_OFF + r));
        }
        // Defenders: an enemy inside is fought as anywhere.
        let inside = self.enemies(i).any(|j| {
            let q = self.squads[j].spec.pos;
            self.squads[j].fighting() && ring.inside(soldiers::V2::new(q.0, q.1))
        });
        if inside {
            return None;
        }
        // With nobody left to fight, runners caught inside are run down
        // (or taken), or let go out of the way they came.
        let Some(f) = foe else {
            let runner = self
                .enemies(i)
                .filter(|&j| {
                    let q = self.squads[j].spec.pos;
                    self.squads[j].on_field() && ring.inside(soldiers::V2::new(q.0, q.1))
                })
                .min_by(|&a, &b| self.dist(i, a).total_cmp(&self.dist(i, b)).then(a.cmp(&b)))?;
            let q = self.squads[runner].spec.pos;
            return Some(Some(if s.chases() {
                (q.0, q.1, Some(runner))
            } else {
                (ring.centre.x, ring.centre.y, None)
            }));
        };
        if !self.field.wall_closed() {
            if let Some(n) = ring.nearest(&self.field, at, true) {
                let p = ring.pieces[n];
                if self.plugs(i, p.mid) {
                    return to(p.beside(-(MAN_OFF + r)));
                }
            }
        }
        let fc = soldiers::V2::new(self.squads[f].spec.pos.0, self.squads[f].spec.pos.1);
        to(ring.man_at(fc, MAN_OFF + r))
    }

    /// Defenders striking down over the wall share out the attackers below
    /// it, as men along it strike whoever stands before them: each takes
    /// the attacker across the wall the fewest of its side strike already
    /// (the nearest of those), and keeps its own while that is as few.
    pub(super) fn spread_over_wall(&mut self) {
        if self.ring.is_none() {
            return;
        }
        let n = self.squads.len();
        let mut on = vec![0usize; n];
        for i in 0..n {
            let s = &self.squads[i];
            if s.spec.side != 1 || !s.fighting() {
                continue;
            }
            let over: Vec<usize> = s
                .contacts
                .iter()
                .copied()
                .filter(|&j| self.wall_piece_between(i, j).is_some())
                .collect();
            if over.len() < 2 || s.contacts.len() != over.len() {
                if let Some(p) = s.primary {
                    on[p] += 1;
                }
                continue;
            }
            let keep = s.primary.filter(|p| over.contains(p));
            let best = over
                .iter()
                .copied()
                .min_by(|&a, &b| {
                    on[a]
                        .cmp(&on[b])
                        .then(self.dist(i, a).total_cmp(&self.dist(i, b)))
                        .then(a.cmp(&b))
                })
                .expect("two or more");
            let pick = match keep {
                Some(k) if on[k] <= on[best] => k,
                _ => best,
            };
            self.squads[i].primary = Some(pick);
            on[pick] += 1;
        }
    }

    /// The share of defending squad `t` that has the cover of a standing
    /// piece of the town's wall against squad `i`'s arrows: its ranks
    /// within `COVER` of the piece between them.
    pub(super) fn covered_share(&self, i: usize, t: usize) -> f32 {
        let st = &self.squads[t];
        if st.spec.side != 1 {
            return 0.0;
        }
        let Some(n) = self.wall_piece_between(i, t) else {
            return 0.0;
        };
        let w = self.ring.as_ref().expect("a piece is of a ring").pieces[n];
        let r = st.radius(&self.params);
        let at = soldiers::V2::new(st.spec.pos.0, st.spec.pos.1);
        // The squad's front stands this far behind the piece's face.
        let front = (w.dist(at) - r - w.half_width).max(0.0);
        ((COVER - front) / (2.0 * r).max(1.0)).clamp(0.0, 1.0)
    }

    /// Whether defending squad `i` is among the `PLUG` fighting ones of its
    /// side nearest a breach at `at`.
    fn plugs(&self, i: usize, at: soldiers::V2) -> bool {
        let side = self.squads[i].spec.side;
        let d = |j: usize| {
            let q = self.squads[j].spec.pos;
            (soldiers::V2::new(q.0, q.1) - at).len2()
        };
        let mine = d(i);
        let nearer = (0..self.squads.len())
            .filter(|&j| j != i && self.squads[j].spec.side == side && self.squads[j].fighting())
            .filter(|&j| d(j) < mine || (d(j) == mine && j < i))
            .count();
        nearer < PLUG
    }

    /// The standing piece of the town's wall between squads `a` and `b`'s
    /// centres, if any.
    pub(super) fn wall_piece_between(&self, a: usize, b: usize) -> Option<usize> {
        let ring = self.ring.as_ref()?;
        let (p, q) = (self.squads[a].spec.pos, self.squads[b].spec.pos);
        let (p, q) = (soldiers::V2::new(p.0, p.1), soldiers::V2::new(q.0, q.1));
        ring.pieces
            .iter()
            .position(|w| self.field.walls[w.k].hp > 0.0 && w.crosses(p, q))
    }

    /// The breach in the town's wall between squads `a` and `b`'s centres,
    /// if any: they fight in it, front to front.
    pub(super) fn breach_between(&self, a: usize, b: usize) -> Option<usize> {
        let ring = self.ring.as_ref()?;
        let (p, q) = (self.squads[a].spec.pos, self.squads[b].spec.pos);
        let (p, q) = (soldiers::V2::new(p.0, p.1), soldiers::V2::new(q.0, q.1));
        ring.pieces
            .iter()
            .position(|w| self.field.walls[w.k].hp <= 0.0 && w.crosses(p, q))
    }

    /// Front of squad `i` fighting through breach `n`: no wider than
    /// the breach, shared with the other squads of its side fighting there.
    pub(super) fn breach_front(&self, i: usize, n: usize) -> f32 {
        let w = self.ring.as_ref().expect("a piece is of a ring").pieces[n];
        let side = self.squads[i].spec.side;
        let sharing = (0..self.squads.len())
            .filter(|&j| {
                let q = &self.squads[j];
                q.spec.side == side
                    && q.fighting()
                    && q.contacts
                        .iter()
                        .any(|&e| self.breach_between(j, e) == Some(n))
            })
            .count()
            .max(1);
        let s = &self.squads[i];
        s.front(&self.params)
            .min((w.len() / s.spacing(&self.params) / sharing as f32).max(1.0))
    }

    /// The gap between `a` and `b` at which they are in contact: a pace,
    /// or across a standing piece of wall its thickness, the two squads'
    /// distances from it and the defender's blow down over it.
    pub(super) fn touch(&self, a: usize, b: usize) -> f32 {
        match self.wall_piece_between(a, b) {
            Some(n) => {
                let w = self.ring.as_ref().expect("a piece is of a ring").pieces[n];
                2.0 * w.half_width + ASSAULT_OFF + MAN_OFF + OVER_WALL
            }
            None => 1.0,
        }
    }

    /// Front of squad `i` at piece `n` of the wall, striking down over it
    /// or battering it: no more than stand along it, the piece shared with
    /// the other squads of its side at it.
    pub(super) fn wall_front(&self, i: usize, n: usize) -> f32 {
        let ring = self.ring.as_ref().expect("a piece is of a ring");
        let w = ring.pieces[n];
        let s = &self.squads[i];
        let side = s.spec.side;
        // Its side's squads along the piece, on its side of it.
        let near = |j: usize| {
            let q = &self.squads[j];
            let at = soldiers::V2::new(q.spec.pos.0, q.spec.pos.1);
            let r = q.radius(&self.params) + q.spec.kit.reach + w.half_width + 1.0 + OVER_WALL;
            j == i
                || (q.fighting()
                    && q.spec.side == side
                    && w.dist(at) <= r
                    && ring.inside(at) == (side == 1))
        };
        let sharing = (0..self.squads.len()).filter(|&j| near(j)).count().max(1);
        let room = (w.len() / s.spacing(&self.params) / sharing as f32).max(1.0);
        (s.alive() as f32).min(room)
    }

    /// Front of defending squad `i` striking down over piece `n` at
    /// attacking squad `t`: its men along the piece, no more than meet the
    /// attackers' front there, shared with the other squads striking at it.
    pub(super) fn over_wall_front(&self, i: usize, t: usize, n: usize) -> f32 {
        let side = self.squads[i].spec.side;
        let striking = (0..self.squads.len())
            .filter(|&j| {
                let q = &self.squads[j];
                q.spec.side == side
                    && q.fighting()
                    && q.contacts.contains(&t)
                    && self.wall_piece_between(j, t).is_some()
            })
            .count()
            .max(1);
        self.wall_front(i, n)
            .min((self.wall_front(t, n) / striking as f32).max(1.0))
    }

    /// The attackers at the wall batter it: those of each squad's front
    /// who stand along the piece it assaults and strike nobody but over
    /// the wall, by their weapon's damage times `wall_taken`. A piece
    /// battered down is a breach, and every way is planned again.
    pub(super) fn batter_walls(&mut self) {
        if self.ring.is_none() {
            return;
        }
        let p = self.params.clone();
        let mut breached = Vec::new();
        for i in 0..self.squads.len() {
            let s = &self.squads[i];
            let Some(n) = s.assault else {
                continue;
            };
            if !s.fighting() {
                continue;
            }
            // Locked with an enemy on its own side of the wall: no hand free.
            if s.contacts
                .iter()
                .any(|&j| self.wall_piece_between(i, j).is_none())
            {
                continue;
            }
            let w = self.ring.as_ref().expect("checked above").pieces[n];
            if self.field.walls[w.k].hp <= 0.0 {
                continue;
            }
            let at = soldiers::V2::new(s.spec.pos.0, s.spec.pos.1);
            let r = s.radius(&p);
            if w.dist(at) > r + s.spec.kit.reach + w.half_width + 1.0 {
                continue;
            }
            // Those who strike a defender over the wall do not batter it.
            let up = s
                .primary
                .filter(|j| s.contacts.contains(j))
                .and_then(|j| self.wall_piece_between(i, j).map(|m| (j, m)))
                .map_or(0.0, |(j, m)| self.over_wall_front(i, j, m) * p.wall_strike_up);
            let front = (self.wall_front(i, n) - up * tmp_k()).max(0.0);
            let per: f32 = (0..3).map(|t| s.spec.kit.damage[t] * p.wall_taken[t]).sum();
            let amount = front * per * s.spec.kit.strike_rate * p.dt;
            self.squads[i].engaged = true;
            if self.field.batter(w.k, amount) {
                breached.push(w.k);
            }
        }
        if breached.is_empty() {
            return;
        }
        for k in breached {
            self.event(EventKind::Breach(k));
        }
        self.nav = renav(&self.field, self.nav.as_ref());
        for s in &mut self.squads {
            s.route.clear();
        }
    }
}

fn tmp_k() -> f32 {
    std::env::var("TMP_K").map_or(1.0, |v| v.parse().unwrap())
}
