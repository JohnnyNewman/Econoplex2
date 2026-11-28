//! Phases 5 and 6: blows land and soldiers move, in soldier order, then each
//! squad is recounted and tells the report what happened.

use super::*;

impl SoldierBattle {
    /// Phase 5: damage, movement with collision, morale, fatigue and state.
    #[inline(never)]
    pub(super) fn apply(&mut self, plans: &[Plan], blows: &[(u32, Blow)], dt: f32) {
        let n = self.soldier.len();
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
            let (si, st) = (
                self.soldier[i].squad as usize,
                self.soldier[t].squad as usize,
            );
            if b.capture {
                if self.soldier[t].state == SoldierState::Routing {
                    self.captured.push(t);
                }
            } else if b.counter_by != NONE {
                // A charge onto braced spears.
                self.charge_event(si, st, true);
            } else {
                if b.angle > 0 && self.soldier[i].side != self.soldier[t].side {
                    self.hurt[t].side_blows += 1;
                }
                if plans[i].act == Act::Charge {
                    self.charge_event(si, st, false);
                }
            }
            self.squads[si].engaged = true;
            self.squads[st].engaged = true;
            if !matches!(plans[i].act, Act::Shoot | Act::Capture)
                && self.soldier[i].side != self.soldier[t].side
            {
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
            if self.soldier[t].state == SoldierState::Routing {
                self.soldier[t].state = SoldierState::Fled;
                self.soldier[t].vel = V2::ZERO;
                let s = &mut self.squads[self.soldier[t].squad as usize];
                s.spec.members[t - s.first].captured = true;
            }
        }
        self.captured = captured;
        self.captured.clear();

        self.fallen.clear();
        self.broke.clear();
        // The soldiers and their other columns as slices of one length, so
        // the loop below indexes them without a check each time.
        let Self {
            params: p,
            field,
            squads,
            kits,
            hurt: hurts,
            soldier,
            restrained,
            health,
            target,
            nav,
            fallen,
            broke,
            ..
        } = self;
        let sp = &p.soldiers;
        let (soldier, hurts, plans) = (&mut soldier[..n], &mut hurts[..n], &plans[..n]);
        let (restrained, health) = (&mut restrained[..n], &mut health[..n]);
        let target = &mut target[..n];
        for i in 0..n {
            let hurt = std::mem::take(&mut hurts[i]);
            let me = &mut soldier[i];
            let state = me.state;
            if !state.present() {
                continue;
            }
            let c = me.squad as usize;
            let plan = &plans[i];
            let k = &kits[me.kit as usize];
            restrained[i] = plan.crush >= sp.crush_bodies;
            if hurt.damage > 0.0 {
                health[i] -= hurt.damage;
                let s = &mut squads[c];
                s.spec.members[i - s.first].health = health[i];
            }
            if health[i] <= 0.0 {
                me.state = SoldierState::Down;
                me.vel = V2::ZERO;
                target[i] = NONE;
                squads[c].losses += 1;
                if state.in_line() {
                    squads[c].left += 1;
                }
                fallen.push(i);
                continue;
            }
            // Fatigue and morale are read once and written once below.
            let (fatigue, morale) = (me.fatigue, me.morale);
            // Move, as far as the wall lets them.
            let from = me.pos;
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
            // Nobody walks into a building: out of it, along its wall.
            if let Some(nav) = nav.as_deref() {
                if nav.any_near(to) || nav.any_near(from) {
                    to = nav.resolve(from, to, k.body, me.side);
                }
            }
            me.pos = to;
            me.vel = plan.vel;
            me.facing = me.facing.turned(plan.facing, turn_c, turn_s);
            // Morale: wounds and blows from the side shake a soldier, more
            // so a tired one.
            let morale = if hurt.wounds == 0 && hurt.side_blows == 0 {
                (morale + plan.morale).clamp(0.0, 1.0)
            } else {
                let steady = squads[c].steady * (1.0 + fatigue);
                let shock = (hurt.wounds as f32 * sp.hit_morale
                    + hurt.side_blows as f32 * sp.flank_drain)
                    * steady;
                (morale + plan.morale - shock).clamp(0.0, 1.0)
            };
            me.morale = morale;
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
            me.fatigue = fatigue;
            // The next blow or arrow, at the pace (`vigour`) they have left.
            let pace = 1.0 - sp.fatigue_effect * fatigue;
            match plan.act {
                Act::Strike | Act::Charge => {
                    me.cooldown = 1.0 / (k.kit.strike_rate * pace).max(1e-3);
                }
                Act::Capture => {
                    let pace = pace * nerve(p, morale);
                    me.cooldown = 1.0 / (k.kit.strike_rate * pace).max(1e-3);
                }
                Act::Shoot => {
                    me.cooldown = 1.0 / (k.kit.shot_rate * pace).max(1e-3);
                    me.arrows = me.arrows.saturating_sub(1);
                    let s = &mut squads[c];
                    s.arrows = s.arrows.saturating_sub(1);
                }
                Act::None => me.cooldown -= dt,
            }
            let quiet = plan.act == Act::None && plan.target == NONE && hurt.wounds == 0;
            me.quiet = if quiet { me.quiet + dt } else { 0.0 };
            target[i] = plan.target;
            // State.
            let gone = off_field(field, to);
            me.state = if gone {
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
        let (grids, men, squads) = (&self.grids, &mut self.soldier, &self.squads);
        for (list, loss) in [(&fallen, sp.near_fall), (&broke, sp.near_rout)] {
            for &f in list.iter() {
                let side = men[f].side as usize;
                let r2 = sp.near_radius * sp.near_radius;
                let pf = men[f].pos;
                grids[side].each_near(pf, sp.near_radius, |j| {
                    if j != f && (men[j].pos - pf).len2() <= r2 && men[j].state.present() {
                        let steady = squads[men[j].squad as usize].steady;
                        men[j].morale = (men[j].morale - loss * steady).max(0.0);
                        men[j].quiet = 0.0;
                    }
                    true
                });
            }
        }
        // Foes near the broken take heart.
        for &f in broke.iter() {
            let foes = 1 - (men[f].side as usize).min(1);
            let r2 = sp.near_radius * sp.near_radius;
            let pf = men[f].pos;
            grids[foes].each_near(pf, sp.near_radius, |j| {
                if (men[j].pos - pf).len2() <= r2 && men[j].state.in_line() {
                    men[j].morale = (men[j].morale + sp.foe_breaks).min(1.0);
                }
                true
            });
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
                for s in &mut self.soldier[r] {
                    if s.state.present() {
                        s.morale = (s.morale - dm).max(0.0);
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
            let (healths, men, targets) = (
                &self.health[r.clone()],
                &self.soldier[r.clone()],
                &self.target[r],
            );
            let (healths, members) = (&healths[..n], &members[..n]);
            let (men, targets) = (&men[..n], &targets[..n]);
            for (k, me) in men.iter().enumerate() {
                // Everyone on their feet is unhurt enough and not taken: only
                // those who left the field need asking.
                match me.state {
                    st if st.in_line() => {
                        standing += 1;
                        in_line += 1;
                        centre += me.pos;
                        morale += me.morale;
                        vigour += 1.0 - fatigue_effect * me.fatigue;
                        if me.rank == 0 {
                            front += me.pos;
                            fronts += 1;
                        }
                        if targets[k] != NONE {
                            contact += 1;
                            self.tally[self.soldier[targets[k] as usize].squad as usize] += 1;
                        }
                    }
                    SoldierState::Routing => {
                        standing += 1;
                        routing += 1;
                        runners += me.pos;
                        morale += me.morale;
                    }
                    SoldierState::Fled if healths[k] > 0.0 && !members[k].captured => {
                        standing += 1;
                    }
                    // Down is at or below no health.
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
            if s.leader.is_some_and(|l| !self.soldier[l].state.present()) {
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
                    for s in &mut self.soldier[first..first + n] {
                        if s.state.in_line() {
                            s.state = SoldierState::Routing;
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
