//! One step of a battle: refreshing the squads and moving them.

use super::*;

impl SquadBattle {
    /// Advance the battle by one step of `params.dt` seconds.
    #[allow(clippy::needless_range_loop)] // squads are indexed while self is mutated
    pub fn step(&mut self) {
        if self.finished() {
            return;
        }
        let n = self.squads.len();
        self.refresh();
        let alive0: Vec<usize> = self.squads.iter().map(Squad::alive).collect();
        let mut flanked = vec![false; n];

        // 1. Move, by order or by the squad's own judgment: every squad from
        // where all stood as the step began, so the order squads are listed
        // in decides nothing (a line arriving a step before the enemy's
        // would find it standing, braced against its charge).
        self.choose_assaults();
        let start: Vec<(f32, f32)> = self.squads.iter().map(|s| s.spec.pos).collect();
        let mut moved = start.clone();
        for i in 0..n {
            self.move_squad(i);
            moved[i] = std::mem::replace(&mut self.squads[i].spec.pos, start[i]);
        }
        for (s, &to) in self.squads.iter_mut().zip(&moved) {
            s.spec.pos = to;
        }

        // 2. Contacts, and charges where a contact is new.
        let contacts: Vec<Vec<usize>> = (0..n)
            .map(|i| {
                if !self.squads[i].on_field() {
                    return Vec::new();
                }
                self.enemies(i)
                    .filter(|&j| self.squads[j].on_field() && self.gap(i, j) <= self.touch(i, j))
                    .collect()
            })
            .collect();
        for i in 0..n {
            for &j in &contacts[i] {
                if self.squads[i].contacts.contains(&j) {
                    continue;
                }
                // New contact: the faster one charges (both, if they run into
                // each other as fast), the longer weapon gets free blows.
                // Nobody charges over a town's wall.
                if i < j && self.wall_piece_between(i, j).is_some() {
                    self.event(EventKind::Contact { by: i, with: j });
                } else if i < j {
                    self.event(EventKind::Contact { by: i, with: j });
                    let (si, sj) = (self.squads[i].speed, self.squads[j].speed);
                    if (si - sj).abs() < 0.1 {
                        self.charge(i, j);
                        self.charge(j, i);
                    } else if si > sj {
                        self.charge(i, j);
                    } else {
                        self.charge(j, i);
                    }
                }
                let (ri, rj) = (self.squads[i].spec.kit.reach, self.squads[j].spec.kit.reach);
                let t = (ri - rj).abs() / self.params.close_speed.max(1e-3);
                self.squads[i].closing.push((j, t));
            }
        }
        self.land_wounds();
        for i in 0..n {
            let s = &mut self.squads[i];
            s.contacts = contacts[i].clone();
            s.closing.retain(|(j, _)| contacts[i].contains(j));
            let dt = self.params.dt;
            for c in &mut s.closing {
                c.1 -= dt;
            }
            if s.primary.is_none_or(|p| !contacts[i].contains(&p)) {
                s.primary = contacts[i].first().copied();
            }
            if !contacts[i].is_empty() {
                s.engaged = true;
            }
        }
        self.spread_over_wall();

        // 3. Arrows from squads not locked in melee.
        for i in 0..n {
            self.shoot(i);
        }
        self.land_wounds();

        // 4. Melee: each squad strikes the enemy it faces.
        for i in 0..n {
            if !self.squads[i].fighting() {
                continue;
            }
            let Some(p) = self.squads[i].primary else {
                continue;
            };
            if !contacts[i].contains(&p) {
                continue;
            }
            // A squad that lets enemies go strikes nobody who runs; one
            // that takes prisoners takes them instead.
            if !self.squads[i].pursues() && self.squads[p].state == SquadState::Routing {
                if self.squads[i].chases() {
                    self.capture(i, p);
                }
                continue;
            }
            // Blows down over a town's wall come from the front.
            if self.squads[p].primary != Some(i)
                && self.squads[p].fighting()
                && self.wall_piece_between(i, p).is_none()
                && self.breach_between(i, p).is_none()
            {
                flanked[p] = true;
            }
            self.melee(i, p);
        }
        self.land_wounds();
        self.batter_walls();

        // 5. Morale and state.
        let dt = self.params.dt;
        let mut news = Vec::new();
        for (i, &before) in alive0.iter().enumerate() {
            let p = &self.params;
            let s = &mut self.squads[i];
            if !s.on_field() {
                continue;
            }
            let alive = s.alive();
            if alive == 0 {
                s.state = SquadState::Destroyed;
                news.push(EventKind::Destroyed(i));
                continue;
            }
            let lost = before.saturating_sub(alive);
            let steady = 1.0 - 0.5 * s.discipline;
            let n0 = s.spec.members.len().max(1) as f32;
            let mut dm = p.morale_per_loss * lost as f32 / n0 * steady;
            if flanked[i] {
                dm += p.flank_drain * dt * steady;
            }
            if lost == 0 && s.contacts.is_empty() {
                s.quiet += dt;
            } else {
                s.quiet = 0.0;
            }
            s.morale = (s.morale - dm).max(0.0);
            if s.quiet > 10.0 && s.state == SquadState::Fighting {
                s.morale = (s.morale + p.morale_regen * dt).min(s.spec.morale.max(s.morale));
            }
            let (waver, rout) = (p.waver_at, p.rout_at);
            if s.state == SquadState::Fighting {
                if s.morale < rout {
                    s.state = SquadState::Routing;
                    s.primary = None;
                    news.push(EventKind::Routing(i));
                } else if s.morale < waver && !s.wavering {
                    s.wavering = true;
                    news.push(EventKind::Wavering(i));
                } else if s.morale >= waver {
                    s.wavering = false;
                }
            }
        }

        for kind in news {
            self.event(kind);
        }

        // 6. Squads that left the field go home.
        for i in 0..n {
            let (x, y) = self.squads[i].spec.pos;
            let out = x < 0.0 || y < 0.0 || x > self.field.width || y > self.field.height;
            if out && self.squads[i].on_field() {
                self.squads[i].state = SquadState::Fled;
                self.event(EventKind::Fled(i));
            }
        }
        self.t += dt;
    }

    /// Proficiency, exhaustion and discipline from the members still
    /// standing, and among obstacles the clear ground across each squad's way.
    fn refresh(&mut self) {
        if let Some(nav) = self.nav.clone() {
            for i in 0..self.squads.len() {
                let s = &self.squads[i];
                let at = soldiers::V2::new(s.spec.pos.0, s.spec.pos.1);
                let toward = s
                    .primary
                    .or_else(|| self.nearest(i, |q| q.on_field()))
                    .map(|j| self.squads[j].spec.pos);
                let ahead = soldiers::V2::new(if s.spec.side == 0 { 1.0 } else { -1.0 }, 0.0);
                let dir = toward
                    .map(|q| (soldiers::V2::new(q.0, q.1) - at).norm_or(ahead))
                    .unwrap_or(ahead);
                let max = self.params.frontage + 2.0;
                self.squads[i].room = nav.room(at, dir, s.spec.side, max);
            }
        }
        let p = &self.params;
        for s in &mut self.squads {
            let (mut sum, mut tired, mut exp, mut n) = (0.0, 0.0, 0.0, 0.0);
            for m in s.spec.members.iter().filter(|m| m.fights()) {
                let e = m.experience.max(0.0);
                sum += rules::proficiency(p, m.skill, e);
                tired += rules::exhaustion(m.fatigue) as f32;
                exp += e;
                n += 1.0;
            }
            if n > 0.0 {
                s.eff = sum / n;
                s.exhaustion = tired / n;
                s.experience = exp / n;
                s.agility = p.initiative_experience * (1.0 + exp / n).ln() - s.exhaustion;
                s.health = s
                    .spec
                    .members
                    .iter()
                    .filter(|m| m.fights())
                    .map(|m| m.health)
                    .sum::<f32>()
                    / n;
                let seasoned = ((1.0 + exp / n).ln() / (1.0 + p.experience_full).ln()).min(1.0);
                s.discipline =
                    (0.5 * s.spec.drill.clamp(0.0, 1.0) + 0.5 * seasoned).clamp(0.0, 1.0);
            }
        }
    }

    /// Where a squad goes this step, by its order or its own judgment.
    fn move_squad(&mut self, i: usize) {
        let p = self.params.clone();
        self.squads[i].speed = 0.0;
        let s = &self.squads[i];
        if !s.on_field() {
            return;
        }
        let mounted = s.spec.kit.mounted;
        let (walk, run) = s.paces(&p);

        if s.state == SquadState::Routing {
            // Away from the nearest enemy, toward its own edge of the field.
            let home_x = if s.spec.side == 0 {
                -10.0
            } else {
                self.field.width + 10.0
            };
            let target = (home_x, s.spec.pos.1);
            self.step_toward(i, target, run, None);
            return;
        }
        // Locked in melee. Two foot squads stay pressed together as their
        // ranks thin, so the fight goes on instead of breaking off and
        // starting again as a new charge. Riders, and men on foot facing
        // them, hold their ground: when the melee opens, the riders ride in
        // again, a new charge.
        if !s.contacts.is_empty() {
            let pressed = |j: &usize| {
                !mounted
                    && !self.squads[*j].spec.kit.mounted
                    && s.contacts.contains(j)
                    && self.wall_piece_between(i, *j).is_none()
            };
            if let Some(j) = s.primary.filter(pressed) {
                self.step_toward(i, self.squads[j].spec.pos, walk, Some(j));
                self.squads[i].speed = 0.0;
            }
            return;
        }
        // A move reached gives way to the next order.
        if let Order::MoveTo { anchor, .. } = order_now(&s.orders) {
            let at = s.spec.pos;
            if (anchor.0 - at.0).hypot(anchor.1 - at.1) <= ARRIVED && s.orders.len() > 1 {
                self.squads[i].orders.remove(0);
            }
        }
        let s = &self.squads[i];
        let order = order_now(&s.orders);
        // Skirmishers back away from enemies who come close, unless told
        // to close with one or to go somewhere.
        if s.stance.fire == Fire::Skirmish
            && s.has_bow()
            && !matches!(
                order,
                Order::Attack(_) | Order::MoveTo { .. } | Order::Withdraw
            )
        {
            if let Some(j) = self
                .nearest(i, |q| q.fighting())
                .filter(|&j| self.gap(i, j) < p.skirmish_distance)
            {
                let (a, b) = (s.spec.pos, self.squads[j].spec.pos);
                let away = (a.0 - b.0, a.1 - b.1);
                let l = away.0.hypot(away.1).max(1e-3);
                let to = (a.0 + away.0 / l * 20.0, a.1 + away.1 / l * 20.0);
                self.step_toward(i, to, run, None);
                return;
            }
        }
        let goal: Option<(f32, f32, Option<usize>)> = match order {
            Order::Hold => None,
            Order::MoveTo { anchor, run: r, .. } => {
                let at = s.spec.pos;
                if (anchor.0 - at.0).hypot(anchor.1 - at.1) <= ARRIVED {
                    return;
                }
                self.step_toward(i, anchor, if r { run } else { walk }, None);
                return;
            }
            Order::Withdraw => {
                let home_x = if s.spec.side == 0 {
                    -10.0
                } else {
                    self.field.width + 10.0
                };
                self.step_toward(i, (home_x, s.spec.pos.1), walk, None);
                return;
            }
            Order::Attack(j) if j < self.squads.len() && self.squads[j].on_field() => {
                let q = self.squads[j].spec.pos;
                Some((q.0, q.1, Some(j)))
            }
            Order::Shoot(j) if j < self.squads.len() && self.squads[j].on_field() => {
                if s.has_bow() && self.dist(i, j) > 0.9 * s.spec.kit.range {
                    let q = self.squads[j].spec.pos;
                    Some((q.0, q.1, Some(j)))
                } else {
                    None
                }
            }
            _ => self.auto_goal(i),
        };
        let Some((x, y, target)) = goal else {
            return;
        };
        // An aggressive side's squads run in from further out.
        let eager = order == Order::Auto
            && self.doctrine[(s.spec.side as usize).min(1)].posture == Posture::Aggressive;
        let charge = p.charge_distance * if eager { 2.0 } else { 1.0 };
        let near = target.is_some_and(|j| self.gap(i, j) < charge);
        self.step_toward(i, (x, y), if near { run } else { walk }, target);
    }

    /// A squad's own judgment: where to go, and whom to close with.
    fn auto_goal(&self, i: usize) -> Option<(f32, f32, Option<usize>)> {
        let s = &self.squads[i];
        if s.spec.behind_wall {
            return None;
        }
        if let Some(goal) = self.wall_goal(i) {
            return goal;
        }
        let kit = &s.spec.kit;
        let fighting = |q: &Squad| q.fighting();
        // A cautious side's squads hold until an enemy comes close.
        if self.doctrine[(s.spec.side as usize).min(1)].posture == Posture::Cautious
            && self
                .nearest(i, fighting)
                .is_none_or(|j| self.gap(i, j) > self.params.cautious_wait)
        {
            return None;
        }
        let target = if kit.mounted {
            // Riders look for archers and fleeing squads (if they chase
            // them), and steer clear of a spear wall while softer targets
            // are about.
            let pursue = s.chases();
            self.enemies(i)
                .filter(|&j| {
                    let q = &self.squads[j];
                    q.fighting() || (pursue && q.on_field())
                })
                .min_by(|&a, &b| {
                    let score = |j: usize| {
                        let q = &self.squads[j];
                        let mut d = self.dist(i, j);
                        if q.state == SquadState::Routing && d > 150.0 {
                            d += 1000.0;
                        }
                        if !q.spec.kit.mounted && q.spec.kit.reach >= kit.reach && q.fighting() {
                            d += 250.0;
                        }
                        if q.has_bow() {
                            d -= 50.0;
                        }
                        d
                    };
                    score(a).total_cmp(&score(b)).then(a.cmp(&b))
                })
        } else {
            self.nearest(i, fighting)
        }?;
        let q = self.squads[target].spec.pos;
        // Archers stand and shoot once in range.
        if s.has_bow() && self.dist(i, target) <= 0.9 * kit.range {
            return None;
        }
        // Spearmen brace when riders are coming.
        if !kit.mounted && kit.reach >= 2.5 {
            let riders = self.enemies(i).any(|j| {
                let r = &self.squads[j];
                r.fighting() && r.spec.kit.mounted && self.dist(i, j) < 200.0
            });
            if riders {
                return None;
            }
        }
        Some((q.0, q.1, Some(target)))
    }

    fn step_toward(&mut self, i: usize, to: (f32, f32), speed: f32, target: Option<usize>) {
        let from = self.squads[i].spec.pos;
        // Round the obstacles: toward the next corner of the way.
        let (to, corner) = match self.nav.as_deref() {
            Some(nav) => {
                let s = &mut self.squads[i];
                let at = soldiers::V2::new(from.0, from.1);
                let goal = soldiers::V2::new(to.0, to.1);
                let way = s.route.next(nav, at, goal, s.spec.side, self.params.dt);
                if (way - goal).len() > 0.01 {
                    ((way.x, way.y), Some((way - at).len()))
                } else {
                    (to, None)
                }
            }
            None => (to, None),
        };
        self.step_along(i, from, to, corner, speed, target)
    }

    /// One step from `from` toward `to`, which is a corner of the way
    /// `corner` metres off (`None`: the goal itself).
    fn step_along(
        &mut self,
        i: usize,
        from: (f32, f32),
        to: (f32, f32),
        corner: Option<f32>,
        speed: f32,
        target: Option<usize>,
    ) {
        let dt = self.params.dt;
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let d = (dx * dx + dy * dy).sqrt();
        let mut step = speed * dt;
        if let Some(j) = target {
            // Stop at the enemy's edge.
            step = step.min((self.gap(i, j) - 0.5).max(0.0));
            if let Some(c) = corner {
                step = step.min(c);
            }
        } else {
            step = step.min(d);
        }
        if d < 1e-3 {
            return;
        }
        if step <= 0.0 {
            // Already at the enemy's edge, because the enemy closed the gap
            // first this step: it was still advancing, not standing braced.
            if target.is_some() {
                self.squads[i].speed = speed;
            }
            return;
        }
        let mut pos = (from.0 + dx / d * step, from.1 + dy / d * step);
        // Nobody walks through the wall: attackers stop at it, defenders stay behind it.
        if let Some(w) = self.field.wall_x {
            let r = self.squads[i].radius(&self.params);
            if self.squads[i].state != SquadState::Routing {
                if self.squads[i].spec.behind_wall {
                    pos.0 = pos.0.max(w + r);
                } else {
                    pos.0 = pos.0.min(w - r);
                }
            }
        }
        let moved = ((pos.0 - from.0).powi(2) + (pos.1 - from.1).powi(2)).sqrt();
        self.squads[i].spec.pos = pos;
        // A squad that moved at all hits at the pace it was moving, and so
        // does one that found the enemy at its edge after a last sliver.
        let at_edge = target.is_some_and(|j| self.gap(i, j) <= 0.51);
        self.squads[i].speed = if moved > 1e-3 || at_edge { speed } else { 0.0 };
    }

    /// True if the wall stands between two squads.
    pub(super) fn across_wall(&self, a: usize, b: usize) -> bool {
        self.field.wall_x.is_some()
            && self.squads[a].spec.behind_wall != self.squads[b].spec.behind_wall
    }
}
