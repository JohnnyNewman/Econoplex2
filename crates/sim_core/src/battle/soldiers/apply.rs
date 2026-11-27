//! Phases 5 and 6: blows land and soldiers move, in soldier order, then each
//! squad is recounted and tells the report what happened.

use super::*;

impl SoldierBattle {
    /// Phase 5: damage, movement with collision, morale, fatigue and state.
    #[inline(never)]
    pub(super) fn apply(&mut self, plans: &[Plan], blows: &[(u32, Blow)], dt: f32) {
        let n = self.pos.len();
        let ns = self.squads.len();
        let turn = self.params.soldiers.soldier_turn * dt;
        let (turn_c, turn_s) = (turn.cos(), turn.sin());
        self.land(blows);
        let lands = std::mem::take(&mut self.lands);
        for ((i, b), &landed) in blows.iter().zip(&lands) {
            if !landed {
                continue;
            }
            let (i, t) = (*i as usize, b.target as usize);
            let (si, st) = (self.squad[i] as usize, self.squad[t] as usize);
            if b.capture {
                if self.state[t] == SoldierState::Routing {
                    self.captured.push(t);
                }
            } else if b.counter_by != NONE {
                // A charge onto braced spears.
                self.charge_event(si, st, true);
            } else {
                if b.angle > 0 && self.side[i] != self.side[t] {
                    self.hurt[t].side_blows += 1;
                }
                if plans[i].act == Act::Charge {
                    self.charge_event(si, st, false);
                }
            }
            self.squads[si].engaged = true;
            self.squads[st].engaged = true;
            if !matches!(plans[i].act, Act::Shoot | Act::Capture) && self.side[i] != self.side[t] {
                let (a, b) = (si.min(st), si.max(st));
                if !self.met[a * ns + b] {
                    self.met[a * ns + b] = true;
                    self.event(EventKind::Contact { by: a, with: b });
                }
            }
        }

        self.lands = lands;

        // Runners laid hands on are taken: off the field, unhurt.
        let captured = std::mem::take(&mut self.captured);
        for &t in &captured {
            if self.state[t] == SoldierState::Routing {
                self.state[t] = SoldierState::Fled;
                self.vel[t] = V2::ZERO;
                let s = &mut self.squads[self.squad[t] as usize];
                s.spec.members[t - s.first].captured = true;
            }
        }
        self.captured = captured;
        self.captured.clear();

        self.fallen.clear();
        self.broke.clear();
        // The soldiers' columns as slices of one length, so the loop below
        // indexes them without a check each time.
        let Self {
            params: p,
            field,
            squads,
            kits,
            hurt: hurts,
            state: states,
            squad,
            kit,
            restrained,
            health,
            pos,
            vel,
            facing,
            target,
            fatigue,
            morale: morales,
            cooldown,
            arrows,
            quiet: quiets,
            fallen,
            broke,
            ..
        } = self;
        let sp = &p.soldiers;
        let hurts = &mut hurts[..n];
        let states = &mut states[..n];
        let (squad, kit, plans) = (&squad[..n], &kit[..n], &plans[..n]);
        let (restrained, health) = (&mut restrained[..n], &mut health[..n]);
        let (pos, vel, facing) = (&mut pos[..n], &mut vel[..n], &mut facing[..n]);
        let (target, fatigues) = (&mut target[..n], &mut fatigue[..n]);
        let (morales, cooldown) = (&mut morales[..n], &mut cooldown[..n]);
        let (arrows, quiets) = (&mut arrows[..n], &mut quiets[..n]);
        for i in 0..n {
            let hurt = std::mem::take(&mut hurts[i]);
            let state = states[i];
            if !state.present() {
                continue;
            }
            let c = squad[i] as usize;
            let plan = &plans[i];
            let k = &kits[kit[i] as usize];
            restrained[i] = plan.crush >= sp.crush_bodies;
            if hurt.damage > 0.0 {
                health[i] -= hurt.damage;
                let s = &mut squads[c];
                s.spec.members[i - s.first].health = health[i];
            }
            if health[i] <= 0.0 {
                states[i] = SoldierState::Down;
                vel[i] = V2::ZERO;
                target[i] = NONE;
                squads[c].losses += 1;
                if state.in_line() {
                    squads[c].left += 1;
                }
                fallen.push(i);
                continue;
            }
            // Fatigue and morale are read once and written once below.
            let (fatigue, morale) = (fatigues[i], morales[i]);
            // Move, as far as the wall lets them.
            let from = pos[i];
            let mut to = from + plan.vel * dt + plan.push;
            if let Some(w) = field.wall_x {
                if squads[c].spec.behind_wall && state != SoldierState::Routing {
                    to.x = to.x.max(w + WALL_HALF + k.body);
                } else if wall_between(field, from, to, p.breach_width) {
                    to.x = if from.x < w {
                        to.x.min(w - WALL_HALF - 0.01)
                    } else {
                        to.x.max(w + WALL_HALF + 0.01)
                    };
                }
                // In the breach, between its sides.
                if (to.x - w).abs() < WALL_HALF && !in_breach(field, p, to.y) {
                    let half = p.breach_width * 0.5;
                    let mid = field.height * 0.5;
                    to.y = to.y.clamp(mid - half, mid + half);
                }
            }
            pos[i] = to;
            vel[i] = plan.vel;
            facing[i] = facing[i].turned(plan.facing, turn_c, turn_s);
            // Morale: wounds and blows from the side shake a soldier, more
            // so a tired one.
            let steady = squads[c].steady * (1.0 + fatigue);
            let shock = (hurt.wounds as f32 * sp.hit_morale
                + hurt.side_blows as f32 * sp.flank_drain)
                * steady;
            let morale = (morale + plan.morale - shock).clamp(0.0, 1.0);
            morales[i] = morale;
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
            fatigues[i] = fatigue;
            // The next blow or arrow, at the pace (`vigour`) they have left.
            let pace = 1.0 - sp.fatigue_effect * fatigue;
            match plan.act {
                Act::Strike | Act::Charge => {
                    cooldown[i] = 1.0 / (k.kit.strike_rate * pace).max(1e-3);
                }
                Act::Capture => {
                    let pace = pace * nerve(p, morale);
                    cooldown[i] = 1.0 / (k.kit.strike_rate * pace).max(1e-3);
                }
                Act::Shoot => {
                    cooldown[i] = 1.0 / (k.kit.shot_rate * pace).max(1e-3);
                    arrows[i] = arrows[i].saturating_sub(1);
                    let s = &mut squads[c];
                    s.arrows = s.arrows.saturating_sub(1);
                }
                Act::None => cooldown[i] -= dt,
            }
            let quiet = plan.act == Act::None && plan.target == NONE && hurt.wounds == 0;
            quiets[i] = if quiet { quiets[i] + dt } else { 0.0 };
            target[i] = plan.target;
            // State.
            let gone = off_field(field, to);
            states[i] = if gone {
                if state.in_line() {
                    squads[c].left += 1;
                }
                SoldierState::Fled
            } else if state == SoldierState::Routing {
                SoldierState::Routing
            } else if morale < p.rout_at {
                squads[c].left += 1;
                broke.push(i);
                SoldierState::Routing
            } else if morale < p.waver_at {
                SoldierState::Wavering
            } else if plan.target != NONE {
                SoldierState::Engaged
            } else {
                SoldierState::Formed
            };
        }
        let p = &self.params;
        let sp = &p.soldiers;

        // Friends near the fallen and the broken lose heart.
        let fallen = std::mem::take(&mut self.fallen);
        let broke = std::mem::take(&mut self.broke);
        for (list, loss) in [(&fallen, sp.near_fall), (&broke, sp.near_rout)] {
            for &f in list.iter() {
                let side = self.side[f] as usize;
                let r2 = sp.near_radius * sp.near_radius;
                let pf = self.pos[f];
                for j in self.grids[side].near(pf, sp.near_radius) {
                    if j != f && (self.pos[j] - pf).len2() <= r2 && self.state[j].present() {
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
            let pf = self.pos[f];
            for j in self.grids[foes].near(pf, sp.near_radius) {
                if (self.pos[j] - pf).len2() <= r2 && self.state[j].in_line() {
                    self.morale[j] = (self.morale[j] + sp.foe_breaks).min(1.0);
                }
            }
        }
        self.fallen = fallen;
        self.broke = broke;
    }

    /// Which of the substep's blows land, into `lands`, with their damage
    /// booked in `hurt`. Blows land by initiative, highest first; blows of
    /// the same initiative land together. A blow lands only if its striker
    /// and its target are still up when its turn comes: a man downed first
    /// does not strike, and nobody strikes a man already down. So damage
    /// past zero comes from the blows that downed him, and the order the
    /// blows were listed in does not matter.
    #[inline(never)]
    pub(super) fn land(&mut self, blows: &[(u32, Blow)]) {
        let n = blows.len();
        self.lands.clear();
        self.lands.resize(n, false);
        // Who a blow hurts, and who strikes it (who must be up).
        let who = |b: &Blow, i: u32| -> (usize, usize) {
            if b.counter_by != NONE {
                (i as usize, b.counter_by as usize)
            } else {
                (b.target as usize, i as usize)
            }
        };
        let down = |hurt: &[Hurt], health: &[f32], k: usize| health[k] - hurt[k].damage <= 0.0;
        let order = &mut self.order;
        order.clear();
        order.extend(0..n as u32);
        order.sort_unstable_by(|&a, &b| {
            blows[b as usize]
                .1
                .init
                .total_cmp(&blows[a as usize].1.init)
                .then(a.cmp(&b))
        });
        let mut g = 0;
        while g < n {
            let init = blows[order[g] as usize].1.init;
            let mut e = g;
            while e < n && blows[order[e] as usize].1.init == init {
                e += 1;
            }
            // Who is still up is read before any blow of the group lands.
            for &k in &order[g..e] {
                let (i, b) = &blows[k as usize];
                let (hurts, by) = who(b, *i);
                let up = !down(&self.hurt, &self.health, hurts)
                    && (b.capture || b.counter_by != NONE || !down(&self.hurt, &self.health, by));
                self.lands[k as usize] = up;
            }
            for &k in &order[g..e] {
                let (i, b) = &blows[k as usize];
                if !self.lands[k as usize] {
                    continue;
                }
                let (hurts, _) = who(b, *i);
                let damage = if b.counter_by != NONE {
                    b.counter
                } else if b.hit && !b.capture {
                    b.damage
                } else {
                    0.0
                };
                if damage > 0.0 {
                    self.hurt[hurts].damage += damage;
                    self.hurt[hurts].wounds += 1;
                }
            }
            g = e;
        }
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
    #[inline(never)]
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
                let r = first..first + n;
                for (m, st) in self.morale[r.clone()].iter_mut().zip(&self.state[r]) {
                    if st.present() {
                        *m = (*m - dm).max(0.0);
                    }
                }
            }
            let left = s.left;
            let (mut in_line, mut routing, mut standing, mut contact) = (0, 0, 0, 0);
            let (mut centre, mut front, mut fronts) = (V2::ZERO, V2::ZERO, 0);
            let (mut morale, mut vigour) = (0.0, 0.0);
            let mut runners = V2::ZERO;
            let members = &self.squads[c].spec.members;
            let r = first..first + n;
            let fatigue_effect = self.params.soldiers.fatigue_effect;
            let (healths, states, at) = (
                &self.health[r.clone()],
                &self.state[r.clone()],
                &self.pos[r.clone()],
            );
            let (morales, ranks, targets) = (
                &self.morale[r.clone()],
                &self.rank[r.clone()],
                &self.target[r.clone()],
            );
            let fatigues = &self.fatigue[r];
            for (k, m) in members.iter().enumerate() {
                if healths[k] > 0.0 && !m.captured {
                    standing += 1;
                }
                match states[k] {
                    st if st.in_line() => {
                        in_line += 1;
                        centre += at[k];
                        morale += morales[k];
                        vigour += 1.0 - fatigue_effect * fatigues[k];
                        if ranks[k] == 0 {
                            front += at[k];
                            fronts += 1;
                        }
                        if targets[k] != NONE {
                            contact += 1;
                            self.tally[self.squad[targets[k] as usize] as usize] += 1;
                        }
                    }
                    SoldierState::Routing => {
                        routing += 1;
                        runners += at[k];
                        morale += morales[k];
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
