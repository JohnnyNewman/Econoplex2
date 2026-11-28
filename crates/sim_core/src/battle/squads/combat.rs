//! Blows, charges and arrows, by the combat rules ([`rules`]): each blow's
//! chance is its attack roll's, and a hit deals the mean damage of the dice.

use super::*;
use crate::battle::rules::{self, Roll};

impl SquadBattle {
    /// A charge on first contact. A braced line with the longer reach turns it
    /// onto its own points; otherwise the charging front strikes with its speed.
    pub(super) fn charge(&mut self, a: usize, b: usize) {
        let p = self.params.clone();
        let speed = self.squads[a].speed;
        if speed < p.paces(false).1 - 0.1 || !self.squads[a].fighting() {
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
            p.charge()
        } else {
            p.charge() * p.foot_charge
        };
        self.event(EventKind::Charge {
            by: a,
            into: b,
            braced,
        });
        if braced {
            // The riders run onto the spears: the spear's blow carries the horse's speed.
            let extra = p.charge() * speed;
            self.strikes(b, a, self.squads[b].front(&p), false, extra, false);
        } else if !self.across_wall(a, b) {
            // A charge rolls the charge dice (a lance's), with the impact on top.
            let extra = per_speed * speed;
            self.strikes(a, b, self.squads[a].front(&p), true, extra, false);
        }
    }

    /// Squad `i`'s melee on squad `t` this step, before the dice: blows
    /// expected, whether they are flank blows, their chance to hit and the
    /// mean hit points of a hit.
    fn exchange(&self, i: usize, t: usize) -> (f32, bool, f32, f32) {
        let p = &self.params;
        let (si, st) = (&self.squads[i], &self.squads[t]);
        let mut front = si.front(p);
        if self.across_wall(i, t) {
            let breach = p.breach_width / si.spacing(p);
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
        // On a wall nobody gets inside a defender's reach.
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
        let flank = st.primary != Some(i) || st.state == SquadState::Routing;
        let blows = front * si.spec.kit.strike_rate * p.dt * reach_factor;
        let (chance, damage) = self.blow_odds(i, t, false, 0.0, flank);
        (blows, flank, chance, damage)
    }

    /// Squad `i`'s initiative before the d20 against squad `t`: agility,
    /// the weapons' speed, and the longer weapon's edge on men coming into
    /// its reach.
    fn initiative(&self, i: usize, t: usize) -> f32 {
        let (si, st) = (&self.squads[i], &self.squads[t]);
        let outreach = if si.spec.kit.reach > st.spec.kit.reach + 0.5 {
            self.params.reach_initiative
        } else {
            0.0
        };
        si.agility + si.spec.kit.speed + outreach
    }

    /// Melee blows of squad `i` on squad `t` for one step. When `t` strikes
    /// back in the same step, the blows meet: each man of `i` whose
    /// opponent wins the initiative and downs him first does not strike.
    /// The share of `i`'s blows lost is the chance `t` strikes first times
    /// the share of `i`'s front its blows down this step.
    pub(super) fn melee(&mut self, i: usize, t: usize) {
        let (blows, flank, _, _) = self.exchange(i, t);
        let st = &self.squads[t];
        let back = st.fighting() && st.primary == Some(i) && st.contacts.contains(&i);
        let lost = if back {
            let (tb, _, tc, td) = self.exchange(t, i);
            let si = &self.squads[i];
            let hp = rules::max_hp(&self.params, si.experience) * si.health;
            let downed = (tb * tc * td / (si.front(&self.params) * hp).max(1e-3)).min(1.0);
            rules::first_chance(self.initiative(t, i), self.initiative(i, t)) * downed
        } else {
            0.0
        };
        let n = self.count(blows * (1.0 - lost));
        self.strikes(i, t, n as f32, false, 0.0, flank);
    }

    /// Frightened: a wavering squad attacks with disadvantage.
    fn frightened(&self, i: usize) -> bool {
        self.squads[i].morale < self.params.waver_at
    }

    /// Squad `i`'s attack bonus: proficiency and weapon quality, less exhaustion.
    fn bonus(&self, i: usize) -> f32 {
        let s = &self.squads[i];
        s.eff + s.spec.kit.quality - self.params.exhaustion_penalty * s.exhaustion
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

    /// `n` blows from squad `i` on squad `t`, a `charge` with `extra` hit
    /// points of impact or not. `flank` blows meet no shield and have
    /// advantage; a frightened squad strikes with disadvantage. A man
    /// climbing the wall or pushing through the breach can't hold up his
    /// shield, and the defender on it has the higher ground (`wall_ground`).
    fn strikes(&mut self, i: usize, t: usize, n: f32, charge: bool, extra: f32, flank: bool) {
        let (chance, damage) = self.blow_odds(i, t, charge, extra, flank);
        let n = n.round() as u32;
        for _ in 0..n {
            if self.rng.random::<f32>() < chance {
                self.hit(t, damage);
            }
        }
    }

    /// The chance a blow of squad `i` on squad `t` hits, and the mean hit
    /// points of a hit (see [`Self::strikes`]).
    fn blow_odds(&self, i: usize, t: usize, charge: bool, extra: f32, flank: bool) -> (f32, f32) {
        let (si, st) = (&self.squads[i], &self.squads[t]);
        let on_wall = si.spec.behind_wall && self.across_wall(i, t);
        let ground = if on_wall {
            self.params.wall_ground
        } else {
            0.0
        };
        let roll = Roll::of(flank, self.frightened(i));
        let ac = st.spec.kit.armour_class(!flank && !on_wall);
        let chance = rules::hit_chance(self.bonus(i) + ground, ac, roll);
        let crit = rules::crit_chance(roll).min(chance) / chance.max(1e-6);
        (
            chance,
            si.spec.kit.blow(charge, extra, crit, &st.spec.kit.taken),
        )
    }

    /// One hit of `damage` hit points on squad `t`, landed with the rest of
    /// the phase's by [`Self::land_wounds`].
    fn hit(&mut self, t: usize, damage: f32) {
        self.wounds.push((t, damage));
    }

    /// The phase's hits land, each on a random fighter of its squad still
    /// standing: every squad struck with the fighters it had when the phase
    /// began, so blows and arrows are simultaneous.
    pub(super) fn land_wounds(&mut self) {
        let wounds = std::mem::take(&mut self.wounds);
        for (t, damage) in wounds {
            self.wound(t, damage);
        }
    }

    /// `damage` hit points land on a random fighter of `t` still standing.
    fn wound(&mut self, t: usize, damage: f32) {
        let standing: Vec<usize> = self.squads[t]
            .spec
            .members
            .iter()
            .enumerate()
            .filter(|(_, m)| m.fights())
            .map(|(k, _)| k)
            .collect();
        if standing.is_empty() {
            return;
        }
        let k = standing[self.rng.random_range(0..standing.len())];
        let m = &mut self.squads[t].spec.members[k];
        m.health -= damage / rules::max_hp(&self.params, m.experience);
    }

    /// Squad `i` takes runners of routing squad `t` prisoner: as many as
    /// its front would strike blows this step.
    pub(super) fn capture(&mut self, i: usize, t: usize) {
        let p = self.params.clone();
        let si = &self.squads[i];
        let reach = si.front(&p) * si.spec.kit.strike_rate * p.dt * self.morale_factor(i);
        let n = self.count(reach);
        for _ in 0..n {
            let free: Vec<usize> = self.squads[t]
                .spec
                .members
                .iter()
                .enumerate()
                .filter(|(_, m)| m.fights())
                .map(|(k, _)| k)
                .collect();
            if free.is_empty() {
                return;
            }
            let k = free[self.rng.random_range(0..free.len())];
            self.squads[t].spec.members[k].captured = true;
        }
        self.squads[i].engaged = true;
    }

    /// Arrows: archers not locked in melee shoot the nearest enemy in range
    /// (or their ordered target), never into a melee where friends fight.
    pub(super) fn shoot(&mut self, i: usize) {
        let p = self.params.clone();
        let s = &self.squads[i];
        if !s.fighting() || !s.has_bow() || !s.contacts.is_empty() || s.stance.fire == Fire::Hold {
            return;
        }
        let range = s.range(&p);
        let side = s.spec.side;
        let ok = |j: usize| {
            let q = &self.squads[j];
            q.on_field()
                && self.dist(i, j) <= range
                && !q.contacts.iter().any(|&c| self.squads[c].spec.side == side)
        };
        let target = match order_now(&s.orders) {
            Order::Shoot(j) | Order::Attack(j) if j < self.squads.len() && ok(j) => Some(j),
            _ => self
                .enemies(i)
                .filter(|&j| ok(j))
                .min_by(|&a, &b| self.dist(i, a).total_cmp(&self.dist(i, b)).then(a.cmp(&b))),
        };
        let Some(t) = target else {
            return;
        };
        let shots = s.alive() as f32 * p.shot_rate(s.spec.kit.shot_rate) * p.dt;
        let n = self.count(shots).min(self.squads[i].arrows);
        let s = &self.squads[i];
        let st = &self.squads[t];
        let d = self.dist(i, t);
        // A rider is a bigger target; an arrow loses its aim with range.
        let rider = if st.spec.kit.mounted {
            p.rider_target
        } else {
            0.0
        };
        let bonus = self.bonus(i) - p.range_penalty * d / range + rider + p.shot_bonus();
        // Cover on the wall; a shield turns arrows unless its bearer runs.
        let cover = if self.across_wall(i, t) && st.spec.behind_wall {
            p.wall_cover
        } else {
            0.0
        };
        let ac = st.spec.kit.armour_class(st.state != SquadState::Routing) + cover;
        let roll = Roll::of(false, self.frightened(i) || p.shot_disadvantage(d));
        let hit = rules::hit_chance(bonus, ac, roll);
        let crit = rules::crit_chance(roll).min(hit) / hit.max(1e-6);
        // An arrow comes down on the man aimed at `arrow_aim` of the time, as
        // on the soldier model; one that misses him lands among the others,
        // on a man as often as men cover the ground of the formation (each
        // within `hit_radius`, a rider's more).
        let r = p.soldiers.hit_radius
            * if st.spec.kit.mounted {
                p.mounted_size
            } else {
                1.0
            };
        let crowd = (std::f32::consts::PI * r * r / st.spacing(&p).powi(2)).min(1.0);
        let lands = p.arrow_aim + (1.0 - p.arrow_aim) * crowd;
        let chance = lands * hit;
        let damage = s.spec.kit.shot(crit, &st.spec.kit.taken);
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
