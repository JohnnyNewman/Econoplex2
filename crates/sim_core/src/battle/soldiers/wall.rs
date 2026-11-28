//! The soldier model at a town's wall (`battle/walls.rs`): where squads go
//! while it stands or once it is breached, and the blows that batter it.

use super::squad::Goal;
use super::*;
use crate::battle::walls::{Ring, ASSAULT_OFF, MAN_OFF, NEAR, PLUG, RIDERS_OFF};

impl SoldierBattle {
    /// Where squad `c` goes because of the town's wall, on its own
    /// judgment (`None`: the wall does not change what it does).
    pub(super) fn wall_goal(&mut self, c: usize) -> Option<Goal> {
        let ring = self.ring.take()?;
        let goal = self.wall_goal_in(&ring, c);
        self.ring = Some(ring);
        self.squads[c].at_wall = goal.is_some();
        goal
    }

    fn wall_goal_in(&mut self, ring: &Ring, c: usize) -> Option<Goal> {
        let s = &self.squads[c];
        if s.order() != Order::Auto {
            return None;
        }
        let walk = if s.mounted() {
            self.params.horse_walk_speed
        } else {
            self.params.walk_speed
        };
        let closed = self.field.wall_closed();
        let foe = self.nearest_foe(c);
        if s.spec.side == 0 {
            let goal = |to: V2, face: V2| Goal {
                to: Some(to),
                face: Some(face),
                target: None,
                pace: walk,
                backing: false,
            };
            let stand = |face: V2| Goal {
                to: None,
                face: Some(face),
                target: None,
                pace: 0.0,
                backing: false,
            };
            // Archers come close enough that all of them reach the
            // defenders, who stand still behind the wall or in the breach.
            // Archers whose arrows are all with their fallen fight on foot.
            let bow = s.has_bow()
                && (s.first..s.first + s.spec.members.len())
                    .any(|i| self.soldier[i].arrows > 0 && self.soldier[i].state.in_line());
            let (centre, range, quiver) = (s.centre, s.spec.kit.range, s.has_bow());
            let near = |fc: V2| (fc - centre).len() <= NEAR * range;
            if !closed {
                self.squads[c].assault = None;
                let f = foe?;
                let fc = self.squads[f].centre;
                return if bow {
                    Some(if near(fc) { stand(fc) } else { goal(fc, fc) })
                } else if quiver {
                    Some(Goal {
                        to: Some(fc),
                        face: None,
                        target: Some(f),
                        pace: walk,
                        backing: false,
                    })
                } else {
                    None
                };
            }
            let s = &self.squads[c];
            // Archers shoot the men on the wall from outside it; riders
            // wait for a breach.
            if bow || s.mounted() {
                let f = foe?;
                let fc = self.squads[f].centre;
                if bow && near(fc) {
                    return Some(stand(fc));
                }
                let n = ring.nearest(&self.field, fc, false)?;
                let p = ring.pieces[n];
                let off = if s.mounted() {
                    RIDERS_OFF
                } else {
                    0.5 * NEAR * s.spec.kit.range
                };
                return Some(goal(p.beside(off), fc));
            }
            let keep = s
                .assault
                .filter(|&n| self.field.walls[ring.pieces[n].k].hp > 0.0);
            let n = keep.or_else(|| ring.assault(&self.field, s.anchor))?;
            let p = ring.pieces[n];
            self.squads[c].assault = Some(n);
            return Some(goal(p.beside(ASSAULT_OFF), p.mid));
        }
        // Defenders: an enemy inside is fought as anywhere.
        let inside = self
            .enemies(c)
            .any(|j| self.squads[j].fighting() && ring.inside(self.squads[j].centre));
        if inside {
            return None;
        }
        // With nobody left to fight, runners caught inside are run down
        // (or taken), or let go out of the way they came.
        let Some(f) = foe else {
            let runner = self
                .enemies(c)
                .filter(|&j| self.squads[j].on_field() && ring.inside(self.squads[j].centre))
                .min_by(|&a, &b| {
                    let d = |j: usize| (self.squads[j].centre - s.centre).len2();
                    d(a).total_cmp(&d(b)).then(a.cmp(&b))
                })?;
            let rc = self.squads[runner].centre;
            return Some(if s.chases() {
                Goal {
                    to: Some(rc),
                    face: Some(rc),
                    target: Some(runner),
                    pace: walk,
                    backing: false,
                }
            } else {
                Goal {
                    to: Some(ring.centre),
                    face: Some(rc),
                    target: None,
                    pace: walk,
                    backing: false,
                }
            });
        };
        if !closed {
            if let Some(n) = ring.nearest(&self.field, s.anchor, true) {
                let p = ring.pieces[n];
                if self.plugs(c, p.mid) {
                    return Some(Goal {
                        to: Some(p.beside(-MAN_OFF)),
                        face: Some(p.mid + p.out * 10.0),
                        target: None,
                        pace: walk,
                        backing: false,
                    });
                }
            }
        }
        // Man the wall where the nearest enemy is.
        let fc = self.squads[f].centre;
        Some(Goal {
            to: Some(ring.man(fc)),
            face: Some(fc),
            target: None,
            pace: walk,
            backing: false,
        })
    }

    fn nearest_foe(&self, c: usize) -> Option<usize> {
        let side = self.squads[c].spec.side;
        let at = self.squads[c].centre;
        (0..self.squads.len())
            .filter(|&j| self.squads[j].spec.side != side && self.squads[j].fighting())
            .min_by(|&a, &b| {
                let d = |j: usize| (self.squads[j].centre - at).len2();
                d(a).total_cmp(&d(b)).then(a.cmp(&b))
            })
    }

    /// Whether defending squad `c` is among the `PLUG` fighting ones of its
    /// side nearest to a breach at `at`.
    fn plugs(&self, c: usize, at: V2) -> bool {
        let side = self.squads[c].spec.side;
        let mine = (self.squads[c].centre - at).len2();
        let nearer = (0..self.squads.len())
            .filter(|&j| j != c && self.squads[j].spec.side == side && self.squads[j].fighting())
            .filter(|&j| {
                let d = (self.squads[j].centre - at).len2();
                d < mine || (d == mine && j < c)
            })
            .count();
        nearer < PLUG
    }

    /// The attackers at the wall batter it: each soldier in line within his
    /// reach of the piece his squad assaults who strikes nobody strikes it,
    /// by his weapon's damage times `wall_taken`. A piece battered down is a
    /// breach, and every way is planned again.
    pub(super) fn batter_walls(&mut self) {
        let Some(ring) = self.ring.take() else {
            return;
        };
        self.batter_with(&ring);
        self.ring = Some(ring);
    }

    fn batter_with(&mut self, ring: &Ring) {
        let dt = self.params.soldiers.substep;
        let taken = self.params.wall_taken;
        let mut breached = Vec::new();
        for c in 0..self.squads.len() {
            let s = &self.squads[c];
            let Some(n) = s.assault else {
                continue;
            };
            if !s.fighting() {
                continue;
            }
            let p = ring.pieces[n];
            if self.field.walls[p.k].hp <= 0.0 {
                continue;
            }
            let mut amount = 0.0;
            for i in s.first..s.first + s.spec.members.len() {
                if !self.soldier[i].state.in_line() || self.target[i] != NONE {
                    continue;
                }
                let k = &self.kits[self.soldier[i].kit as usize];
                if p.dist(self.soldier[i].pos) > k.kit.reach + k.body + p.half_width + 0.3 {
                    continue;
                }
                let per: f32 = (0..3).map(|t| k.kit.damage[t] * taken[t]).sum();
                amount += per * k.kit.strike_rate * dt;
            }
            if amount > 0.0 {
                self.squads[c].engaged = true;
            }
            if self.field.batter(p.k, amount) {
                breached.push(p.k);
            }
        }
        if breached.is_empty() {
            return;
        }
        for k in breached {
            self.events.push(Event {
                t: self.t,
                kind: EventKind::Breach(k),
            });
        }
        self.nav = renav(&self.field, self.nav.as_ref());
        for s in &mut self.squads {
            s.route.clear();
        }
    }
}
