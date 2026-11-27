//! Phases 5 and 6: blows land and soldiers move, in soldier order, then each
//! squad is recounted and tells the report what happened.

use super::*;

impl SoldierBattle {
    /// Phase 5: damage, movement with collision, morale, fatigue and state.
    pub(super) fn apply(&mut self, plans: &[Plan], blows: &[(u32, Blow)], dt: f32) {
        let n = self.pos.len();
        let ns = self.squads.len();
        let turn = self.params.soldiers.soldier_turn * dt;
        let (turn_c, turn_s) = (turn.cos(), turn.sin());
        // Blows add up per soldier, in soldier order, so two on one soldier
        // give the same result whoever struck first.
        for (i, b) in blows {
            let (i, t) = (*i as usize, b.target as usize);
            let (si, st) = (self.squad[i] as usize, self.squad[t] as usize);
            if b.counter_by != NONE {
                // A charge onto braced spears.
                if b.counter > 0.0 {
                    self.hurt[i].damage += b.counter;
                    self.hurt[i].wounds += 1;
                }
                self.charge_event(si, st, true);
            } else {
                if b.hit {
                    self.hurt[t].damage += b.damage;
                    self.hurt[t].wounds += 1;
                }
                if b.angle > 0 && self.side[i] != self.side[t] {
                    self.hurt[t].side_blows += 1;
                }
                if plans[i].act == Act::Charge {
                    self.charge_event(si, st, false);
                }
            }
            self.squads[si].engaged = true;
            self.squads[st].engaged = true;
            if plans[i].act != Act::Shoot && self.side[i] != self.side[t] {
                let (a, b) = (si.min(st), si.max(st));
                if !self.met[a * ns + b] {
                    self.met[a * ns + b] = true;
                    self.event(EventKind::Contact { by: a, with: b });
                }
            }
        }

        let p = &self.params;
        let sp = &p.soldiers;
        self.fallen.clear();
        self.broke.clear();
        for i in 0..n {
            let hurt = std::mem::take(&mut self.hurt[i]);
            let state = self.state[i];
            if !state.present() {
                continue;
            }
            let c = self.squad[i] as usize;
            let plan = &plans[i];
            let k = &self.kits[self.kit[i] as usize];
            if hurt.damage > 0.0 {
                self.health[i] -= hurt.damage;
                let s = &mut self.squads[c];
                s.spec.members[i - s.first].health = self.health[i];
            }
            if self.health[i] <= 0.0 {
                self.state[i] = SoldierState::Down;
                self.vel[i] = V2::ZERO;
                self.target[i] = NONE;
                self.squads[c].losses += 1;
                if state.in_line() {
                    self.squads[c].left += 1;
                }
                self.fallen.push(i);
                continue;
            }
            // Fatigue and morale are read once and written once below.
            let (fatigue, morale) = (self.fatigue[i], self.morale[i]);
            // Move, as far as the wall lets them.
            let from = self.pos[i];
            let mut to = from + plan.vel * dt + plan.push;
            if let Some(w) = self.field.wall_x {
                if self.squads[c].spec.behind_wall && state != SoldierState::Routing {
                    to.x = to.x.max(w + WALL_HALF + k.body);
                } else if self.wall_between(from, to, self.params.breach_width) {
                    to.x = if from.x < w {
                        to.x.min(w - WALL_HALF - 0.01)
                    } else {
                        to.x.max(w + WALL_HALF + 0.01)
                    };
                }
                // In the breach, between its sides.
                if (to.x - w).abs() < WALL_HALF && !self.in_breach(to.y) {
                    let half = self.params.breach_width * 0.5;
                    let mid = self.field.height * 0.5;
                    to.y = to.y.clamp(mid - half, mid + half);
                }
            }
            self.pos[i] = to;
            self.vel[i] = plan.vel;
            self.facing[i] = self.facing[i].turned(plan.facing, turn_c, turn_s);
            // Morale: wounds and blows from the side shake a soldier, more
            // so a tired one.
            let steady = self.squads[c].steady * (1.0 + fatigue);
            let shock = (hurt.wounds as f32 * sp.hit_morale
                + hurt.side_blows as f32 * sp.flank_drain)
                * steady;
            let morale = (morale + plan.morale - shock).clamp(0.0, 1.0);
            self.morale[i] = morale;
            // Fatigue: running, fighting and marching tire; standing rests.
            let speed = plan.vel.len();
            let mut rise = 0.0;
            if speed > k.walk + 0.2 {
                let horse = if k.kit.mounted { sp.horse_tire } else { 1.0 };
                rise += sp.fatigue_run * (1.0 + k.kit.weight / sp.carry_kg.max(1.0)) * horse;
            } else if speed > 0.2 {
                rise += sp.fatigue_walk;
            }
            if plan.target != NONE {
                rise += sp.fatigue_fight;
            }
            let fatigue = if rise > 0.0 {
                (fatigue + rise * dt).min(1.0)
            } else {
                (fatigue - sp.fatigue_rest * dt).max(0.0)
            };
            self.fatigue[i] = fatigue;
            // The next blow or arrow.
            match plan.act {
                Act::Strike | Act::Charge => {
                    let pace = self.vigour(i) * self.nerve(i);
                    self.cooldown[i] = 1.0 / (k.kit.strike_rate * pace).max(1e-3);
                }
                Act::Shoot => {
                    let pace = self.vigour(i) * self.nerve(i);
                    self.cooldown[i] = 1.0 / (k.kit.shot_rate * pace).max(1e-3);
                    self.arrows[i] = self.arrows[i].saturating_sub(1);
                    let s = &mut self.squads[c];
                    s.arrows = s.arrows.saturating_sub(1);
                }
                Act::None => self.cooldown[i] -= dt,
            }
            let quiet = plan.act == Act::None && plan.target == NONE && hurt.wounds == 0;
            self.quiet[i] = if quiet { self.quiet[i] + dt } else { 0.0 };
            self.target[i] = plan.target;
            // State.
            let gone = self.off_field(to);
            self.state[i] = if gone {
                if state.in_line() {
                    self.squads[c].left += 1;
                }
                SoldierState::Fled
            } else if state == SoldierState::Routing {
                SoldierState::Routing
            } else if morale < p.rout_at {
                self.squads[c].left += 1;
                self.broke.push(i);
                SoldierState::Routing
            } else if morale < p.waver_at {
                SoldierState::Wavering
            } else if plan.target != NONE {
                SoldierState::Engaged
            } else {
                SoldierState::Formed
            };
        }

        // Friends near the fallen and the broken lose heart.
        let fallen = std::mem::take(&mut self.fallen);
        let broke = std::mem::take(&mut self.broke);
        for (list, loss) in [(&fallen, sp.near_fall), (&broke, sp.near_rout)] {
            for &f in list.iter() {
                let side = self.side[f] as usize;
                let r2 = sp.near_radius * sp.near_radius;
                let near: Vec<usize> = self.grids[side]
                    .near(self.pos[f], sp.near_radius)
                    .filter(|&j| j != f && (self.pos[j] - self.pos[f]).len2() <= r2)
                    .collect();
                for j in near {
                    if self.state[j].present() {
                        let steady = self.squads[self.squad[j] as usize].steady;
                        self.morale[j] = (self.morale[j] - loss * steady).max(0.0);
                        self.quiet[j] = 0.0;
                    }
                }
            }
        }
        // Foes near the broken take heart.
        for &f in broke.iter() {
            let foes = 1 - (self.side[f] as usize).min(1);
            let r2 = sp.near_radius * sp.near_radius;
            let near: Vec<usize> = self.grids[foes]
                .near(self.pos[f], sp.near_radius)
                .filter(|&j| (self.pos[j] - self.pos[f]).len2() <= r2)
                .collect();
            for j in near {
                if self.state[j].in_line() {
                    self.morale[j] = (self.morale[j] + sp.foe_breaks).min(1.0);
                }
            }
        }
        self.fallen = fallen;
        self.broke = broke;
    }

    fn charge_event(&mut self, by: usize, into: usize, braced: bool) {
        let ns = self.squads.len();
        if !self.charged[by * ns + into] {
            self.charged[by * ns + into] = true;
            self.event(EventKind::Charge { by, into, braced });
        }
    }

    /// Phase 6: recount each squad's soldiers and morale, and tell the report
    /// about contact, wavering, routing, running out of arrows and fleeing.
    pub(super) fn recount(&mut self) {
        let p = &self.params;
        let (rout, waver, per_loss) = (p.rout_at, p.waver_at, p.morale_per_loss);
        let mut news = Vec::new();
        for c in 0..self.squads.len() {
            let s = &self.squads[c];
            let (first, n) = (s.first, s.spec.members.len());
            if !s.on_field() {
                continue;
            }
            // Each loss shakes the whole squad, as on the squad model.
            if s.losses > 0 {
                let dm = per_loss * s.losses as f32 / n.max(1) as f32 * s.steady;
                for i in first..first + n {
                    if self.state[i].present() {
                        self.morale[i] = (self.morale[i] - dm).max(0.0);
                    }
                }
            }
            let left = s.left;
            let (mut in_line, mut routing, mut standing, mut contact) = (0, 0, 0, 0);
            let (mut centre, mut front, mut fronts) = (V2::ZERO, V2::ZERO, 0);
            let (mut morale, mut vigour) = (0.0, 0.0);
            let mut runners = V2::ZERO;
            for i in first..first + n {
                if self.health[i] > 0.0 {
                    standing += 1;
                }
                match self.state[i] {
                    st if st.in_line() => {
                        in_line += 1;
                        centre += self.pos[i];
                        morale += self.morale[i];
                        vigour += self.vigour(i);
                        if self.rank[i] == 0 {
                            front += self.pos[i];
                            fronts += 1;
                        }
                        if self.target[i] != NONE {
                            contact += 1;
                            self.tally[self.squad[self.target[i] as usize] as usize] += 1;
                        }
                    }
                    SoldierState::Routing => {
                        routing += 1;
                        runners += self.pos[i];
                        morale += self.morale[i];
                    }
                    _ => {}
                }
            }
            // `tally` is all zeros between squads: only a squad in contact
            // wrote to it.
            let fighting_with = if contact > 0 {
                let f = (0..self.squads.len())
                    .filter(|&j| self.tally[j] > 0)
                    .max_by_key(|&j| (self.tally[j], std::cmp::Reverse(j)));
                self.tally.fill(0);
                f
            } else {
                None
            };
            let s = &mut self.squads[c];
            s.losses = 0;
            s.left = 0;
            s.in_line = in_line;
            s.routing = routing;
            s.in_contact = contact;
            s.fighting_with = fighting_with;
            if in_line > 0 {
                s.centre = centre * (1.0 / in_line as f32);
                s.vigour = vigour / in_line as f32;
            } else if routing > 0 {
                s.centre = runners * (1.0 / routing as f32);
            }
            s.front = if fronts > 0 {
                front * (1.0 / fronts as f32)
            } else {
                s.anchor
            };
            if in_line + routing > 0 {
                s.morale = morale / (in_line + routing) as f32;
            }
            if s.leader.is_some_and(|l| !self.state[l].present()) {
                s.leader = None;
            }
            // State, as on the squad model.
            match s.state {
                SquadState::Fighting if standing == 0 => {
                    s.state = SquadState::Destroyed;
                    news.push(EventKind::Destroyed(c));
                }
                SquadState::Fighting if in_line == 0 && routing == 0 => {
                    s.state = SquadState::Fled;
                    news.push(EventKind::Fled(c));
                }
                SquadState::Fighting if in_line == 0 || s.morale < rout => {
                    s.state = SquadState::Routing;
                    s.target = None;
                    news.push(EventKind::Routing(c));
                    for st in &mut self.state[first..first + n] {
                        if st.in_line() {
                            *st = SoldierState::Routing;
                        }
                    }
                }
                SquadState::Fighting => {
                    if s.morale < waver && !s.wavering {
                        s.wavering = true;
                        news.push(EventKind::Wavering(c));
                    } else if s.morale >= waver {
                        s.wavering = false;
                    }
                }
                SquadState::Routing if routing == 0 => {
                    s.state = if standing == 0 {
                        news.push(EventKind::Destroyed(c));
                        SquadState::Destroyed
                    } else {
                        news.push(EventKind::Fled(c));
                        SquadState::Fled
                    };
                }
                _ => {}
            }
            if s.spec.kit.range > 0.0 && s.arrows == 0 && !s.told_arrows {
                s.told_arrows = true;
                news.push(EventKind::OutOfArrows(c));
            }
            if left > 0 && s.fighting() {
                self.refill(c);
            }
        }
        for kind in news {
            self.event(kind);
        }
    }
}
