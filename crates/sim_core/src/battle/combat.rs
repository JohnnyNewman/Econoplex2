//! Blows, charges and arrows.

use super::*;

impl Battle {
    /// A charge on first contact. A braced line with the longer reach turns it
    /// onto its own points; otherwise the charging front strikes with its speed.
    pub(super) fn charge(&mut self, a: usize, b: usize) {
        let p = self.params.clone();
        let speed = self.squads[a].speed;
        if speed < p.run_speed - 0.1 || !self.squads[a].fighting() {
            return;
        }
        let (sa, sb) = (&self.squads[a], &self.squads[b]);
        let braced = sb.fighting()
            && sb.speed < 0.1
            && !sb.spec.kit.mounted
            && sb.spec.kit.reach + 0.01 >= sa.spec.kit.reach
            && sb.morale >= p.waver_at
            && !self.across_wall(a, b);
        let per_speed = if sa.spec.kit.mounted {
            p.charge_damage
        } else {
            p.charge_damage * p.foot_charge
        };
        self.event(EventKind::Charge {
            by: a,
            into: b,
            braced,
        });
        if braced {
            // The riders run onto the spears: the spear's blow carries the horse's speed.
            let extra = p.charge_damage * speed;
            self.strikes(b, a, self.squads[b].front(&p), extra, false);
        } else if !self.across_wall(a, b) {
            let extra = per_speed * speed;
            self.strikes(a, b, self.squads[a].front(&p), extra, false);
        }
    }

    /// Melee blows of squad `i` on squad `t` for one step.
    pub(super) fn melee(&mut self, i: usize, t: usize) {
        let p = self.params.clone();
        let (si, st) = (&self.squads[i], &self.squads[t]);
        let mut front = si.front(&p);
        if self.across_wall(i, t) {
            let breach = p.breach_width / si.spacing(&p);
            front = front.min(if si.spec.behind_wall {
                2.0 * breach
            } else {
                breach
            });
        }
        // Reach: while a shorter weapon closes in, only the longer one strikes;
        // once inside, the longer one is clumsy.
        let (ri, rt) = (si.spec.kit.reach, st.spec.kit.reach);
        let closing = si.closing.iter().find(|c| c.0 == t).map_or(0.0, |c| c.1);
        // On a wall nobody gets inside a defender's reach, and a man climbing
        // or pushing through the breach can't parry or hold up his shield.
        let on_wall = si.spec.behind_wall && self.across_wall(i, t);
        let reach_factor = if on_wall {
            1.0
        } else if closing > 0.0 {
            if ri + 0.01 >= rt {
                1.0
            } else {
                0.0
            }
        } else if ri > rt + 1.0 {
            1.0 - si.spec.kit.close_penalty
        } else {
            1.0
        };
        let flank = st.primary != Some(i) || st.state == SquadState::Routing || on_wall;
        let blows = front * si.spec.kit.strike_rate * p.dt * reach_factor * self.morale_factor(i);
        let n = self.count(blows);
        self.strikes(i, t, n as f32, 0.0, flank);
    }

    fn morale_factor(&self, i: usize) -> f32 {
        let s = &self.squads[i];
        let w = self.params.waver_at.max(1e-3);
        if s.morale >= w {
            1.0
        } else {
            0.5 + 0.5 * s.morale / w
        }
    }

    /// A random whole number with the given expectation.
    fn count(&mut self, x: f32) -> u32 {
        let base = x.max(0.0).floor();
        base as u32 + u32::from(self.rng.random::<f32>() < x - base)
    }

    /// `n` blows from squad `i` on squad `t`; `flank` blows meet no parry and no shield.
    fn strikes(&mut self, i: usize, t: usize, n: f32, extra: f32, flank: bool) {
        let p = &self.params;
        let (si, st) = (&self.squads[i], &self.squads[t]);
        let def = if flank { 0.0 } else { st.eff };
        let shield = if flank {
            0.0
        } else {
            st.spec.kit.shield * p.shield_parry
        };
        let x = p.skill_slope * (si.eff - def);
        let chance = (2.0 * p.base_hit / (1.0 + (-x).exp())).min(0.95) * (1.0 - shield);
        let damage = (si.spec.kit.damage + extra) * (1.0 - st.spec.kit.armour);
        let n = n.round() as u32;
        for _ in 0..n {
            if self.rng.random::<f32>() < chance {
                self.hit(t, damage);
            }
        }
    }

    /// One hit lands on a random fighter still standing.
    fn hit(&mut self, t: usize, damage: f32) {
        let standing: Vec<usize> = self.squads[t]
            .spec
            .members
            .iter()
            .enumerate()
            .filter(|(_, m)| m.health > 0.0)
            .map(|(k, _)| k)
            .collect();
        if standing.is_empty() {
            return;
        }
        let k = standing[self.rng.random_range(0..standing.len())];
        self.squads[t].spec.members[k].health -= damage;
    }

    /// Arrows: archers not locked in melee shoot the nearest enemy in range
    /// (or their ordered target), never into a melee where friends fight.
    pub(super) fn shoot(&mut self, i: usize) {
        let p = self.params.clone();
        let s = &self.squads[i];
        if !s.fighting() || !s.has_bow() || !s.contacts.is_empty() {
            return;
        }
        let range = s.spec.kit.range;
        let side = s.spec.side;
        let ok = |j: usize| {
            let q = &self.squads[j];
            q.on_field()
                && self.dist(i, j) <= range
                && !q.contacts.iter().any(|&c| self.squads[c].spec.side == side)
        };
        let target = match s.order {
            Order::Shoot(j) | Order::Attack(j) if j < self.squads.len() && ok(j) => Some(j),
            _ => self
                .enemies(i)
                .filter(|&j| ok(j))
                .min_by(|&a, &b| self.dist(i, a).total_cmp(&self.dist(i, b)).then(a.cmp(&b))),
        };
        let Some(t) = target else {
            return;
        };
        let shots = s.alive() as f32 * s.spec.kit.shot_rate * p.dt * self.morale_factor(i);
        let n = self.count(shots).min(self.squads[i].arrows);
        let s = &self.squads[i];
        let st = &self.squads[t];
        let d = self.dist(i, t);
        let size = if st.spec.kit.mounted {
            p.mounted_size
        } else {
            1.0
        };
        let cover = if self.across_wall(i, t) && st.spec.behind_wall {
            p.wall_cover
        } else {
            0.0
        };
        let skill = (0.5 + s.eff).clamp(0.2, 1.5);
        let chance = (p.shot_accuracy * (1.0 - p.range_falloff * d / range) * skill * size)
            .clamp(0.0, 0.95)
            * (1.0 - st.spec.kit.shield)
            * (1.0 - cover);
        let damage = s.spec.kit.shot_damage * (1.0 - st.spec.kit.armour);
        self.squads[i].arrows -= n;
        self.squads[i].engaged = true;
        for _ in 0..n {
            if self.rng.random::<f32>() < chance {
                self.hit(t, damage);
            }
        }
        if self.squads[i].arrows == 0 {
            self.event(EventKind::OutOfArrows(i));
        }
    }
}
