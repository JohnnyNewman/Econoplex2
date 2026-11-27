//! Phases 3 and 4: each soldier picks where to step and whom to strike or
//! shoot, then each strike and shot is rolled. Both only read the last
//! substep's state, so they can run in any order and on any thread.

use super::*;
use crate::battle::rules::{self, Roll};

/// cos 135°: a blow from further round than this comes from behind.
const REAR: f32 = -std::f32::consts::FRAC_1_SQRT_2;

impl SoldierBattle {
    /// Phase 3: what soldier `i` means to do.
    pub(super) fn plan(&self, i: usize) -> Plan {
        let state = self.state[i];
        let mut plan = Plan {
            facing: self.facing[i],
            ..Plan::default()
        };
        if !state.present() {
            return plan;
        }
        let p = &self.params;
        let sp = &p.soldiers;
        let dt = sp.substep;
        let k = &self.kits[self.kit[i] as usize];
        let c = &self.squads[self.squad[i] as usize];
        let top = k.top * self.vigour(i);
        let me = self.pos[i];
        let side = self.side[i] as usize;
        let (friends, foes) = (&self.grids[side], &self.grids[1 - side]);
        let foes_near = foes.around(foes.cell(me)) > 0;

        if state == SoldierState::Routing {
            // Away from the nearest enemies, toward their own edge of the
            // field; round the enemy when they stand in the way.
            let edge = V2::new(if side == 0 { -1.0 } else { 1.0 }, 0.0);
            // Among obstacles, down the side's flow field to its edge.
            let home = self
                .nav
                .as_deref()
                .and_then(|n| n.home_dir(me, side as u8))
                .unwrap_or(edge);
            let mut away = V2::ZERO;
            if foes_near {
                let cell2 = sp.cell * sp.cell;
                foes.each_near(me, sp.cell, |j, pj| {
                    let d = me - pj;
                    let d2 = d.len2();
                    if d2 >= cell2 {
                        return;
                    }
                    if d2 > 1e-6 {
                        away += d * (2.0 / d2);
                    }
                    plan.push += self.shove(i, j, d, d2, 0.5);
                });
            }
            let mut dir = home + away;
            if dir.dot(home) < 0.3 {
                let round = if away.y != 0.0 {
                    away.y.signum()
                } else if me.y < self.field.height * 0.5 {
                    -1.0
                } else {
                    1.0
                };
                dir = if home == edge {
                    V2::new(home.x * 0.3, round)
                } else {
                    // Among obstacles: round them along the way home, each
                    // runner on its own side of it.
                    let side = if i.is_multiple_of(2) { 1.0 } else { -1.0 };
                    home * 0.3 + V2::new(-home.y, home.x) * side
                };
            }
            let mut dir = dir.norm_or(home);
            // Among obstacles the way home turns from cell to cell, and a
            // runner deciding afresh each substep could swing back and forth
            // between two of them for ever: it keeps some of the way it ran.
            if self.nav.is_some() {
                dir = (dir + self.vel[i].norm_or(V2::ZERO) * 1.5).norm_or(home);
            }
            plan.vel = dir * top;
            plan.facing = dir;
            return plan;
        }

        // The nearest enemy within reach, or near the slot; and bodies to push off.
        let mut slot = c.slot(self.file[i], self.rank[i]);
        // In contact, the ranks behind keep a pace behind the man in front:
        // the front presses, the rear does not crowd it.
        let ahead = self.ahead[i] as usize;
        if c.in_contact > 0 && ahead != NONE as usize && self.state[ahead].in_line() {
            let lim = self.pos[ahead] - c.facing * c.spacing;
            let over = (slot - lim).dot(c.facing);
            if over > 0.0 {
                slot -= c.facing * over;
            }
        }
        // Where a slot falls on an obstacle, the nearest place beside it.
        if self.nav.as_deref().is_some_and(|n| n.any_near(slot)) {
            slot = self.clear_slot(slot, i);
        }
        let front = ahead == NONE as usize || !self.state[ahead].in_line();
        let reach = k.kit.reach;
        let slack2 = sp.engage_slack * sp.engage_slack;
        let breach = if c.spec.behind_wall {
            2.0 * p.breach_width
        } else {
            p.breach_width
        };
        let mut best = NONE;
        let mut best_d2 = f32::INFINITY;
        if foes_near {
            // Only the front rank looks past its reach, to its slot.
            let look = if front {
                reach.max(sp.engage_slack + (slot - me).len())
            } else {
                reach
            };
            let r = look.max(k.body) + self.widest[1 - side];
            foes.each_near(me, r.min(sp.cell), |j, pj| {
                let d = me - pj;
                let d2 = d.len2();
                // Nobody further than `r` is pushed against or picked.
                if d2 > r * r {
                    return;
                }
                let bj = self.body[j];
                let shove = self.shove(i, j, d, d2, 0.5);
                plan.crush += u8::from(shove != V2::ZERO);
                plan.push += shove;
                // A squad that lets enemies go strikes nobody who runs (one
                // that takes prisoners goes for them, to take them).
                if !c.chases() && self.state[j] == SoldierState::Routing {
                    return;
                }
                if d2 < best_d2 {
                    let rj = reach + bj;
                    // In reach, or (for the front rank) near the slot.
                    let close = d2 <= rj * rj || (front && (pj - slot).len2() <= slack2);
                    if close && !self.wall_between(me, pj, breach) {
                        best = j as u32;
                        best_d2 = d2;
                    }
                }
            });
        }

        if best != NONE {
            let t = best as usize;
            let kt = &self.kits[self.kit[t] as usize];
            let d = best_d2.sqrt();
            let u = (self.pos[t] - me).norm_or(c.facing);
            plan.target = best;
            plan.facing = u;
            // Close to a little inside their reach; pushing past a longer
            // weapon's point is slow.
            let want = (reach * 0.8).max(k.body + kt.body);
            let mut speed = top;
            if kt.kit.reach > reach + 0.01 && d < kt.kit.reach + k.body {
                speed = speed.min(p.close_speed);
            }
            let mut to = me + u * (d - want).clamp(0.0, speed * dt);
            // Stray no further than `engage_slack` from the slot (but a slot
            // the line has pushed past the enemy does not drag them on).
            let off = to - slot;
            if off.len2() > slack2 && off.len2() > (me - slot).len2() {
                to = slot + off.norm_or(u) * sp.engage_slack.max((me - slot).len());
            }
            plan.vel = ((to - me) * (1.0 / dt)).clamp_len(top);
            if d <= reach + kt.body {
                // Arriving at a run is a charge, for any steady soldier.
                let running = self.vel[i].len() >= p.run_speed - 0.1;
                let steady = matches!(state, SoldierState::Formed | SoldierState::Engaged);
                plan.act = if steady && running {
                    Act::Charge
                } else if self.cooldown[i] <= 0.0 {
                    Act::Strike
                } else {
                    Act::None
                };
                if plan.act != Act::None && self.blocked(i, me, u, d) {
                    plan.act = Act::None;
                }
                // A runner within reach is taken, not struck.
                if plan.act != Act::None && !c.pursues() && self.state[t] == SoldierState::Routing {
                    plan.act = Act::Capture;
                }
            }
        } else {
            plan.facing = c.facing;
            // Shoot at a soldier of the squad's mark, if they are in range
            // and no enemy is close enough to make them draw the knife.
            if let Some(q) = c.shoot_at {
                let free = !foes_near && self.arrows[i] > 0 && self.cooldown[i] <= 0.0;
                if k.kit.range > 0.0 && free {
                    let q = &self.squads[q];
                    let mut d = Draw::new(self.seed, self.substep, i, Purpose::Aim);
                    let n = q.spec.members.len().max(1);
                    let aim = (0..3)
                        .map(|_| q.first + d.below(n))
                        .find(|&j| self.state[j].present());
                    if let Some(a) = aim {
                        let to = self.pos[a] - me;
                        if to.len2() <= k.kit.range * k.kit.range {
                            plan.act = Act::Shoot;
                            plan.aim = a as u32;
                            plan.facing = to.norm_or(c.facing);
                        }
                    }
                }
            }
            // Round an obstacle between them and their slot, by its nearer corner.
            let mut to = slot;
            if let Some(nav) = self.nav.as_deref() {
                if (slot - me).len2() > 1.0 && nav.any_near(me) {
                    if let Some(corner) = nav.steer(me, slot, k.body, side as u8) {
                        to = corner;
                    }
                }
            }
            plan.vel = ((to - me) * (1.0 / dt)).clamp_len(top);
        }

        // Friends give way to the front rank. A soldier standing in their
        // slot with nobody to fight need not look, nor need one keeping
        // within half a pace of it out of contact: the formation keeps them apart.
        let off2 = (slot - me).len2();
        let half = 0.5 * c.spacing;
        let settled = state == SoldierState::Formed
            && best == NONE
            && (off2 < 0.09 || (!foes_near && off2 < half * half));
        if !settled {
            let r = k.body + self.widest[side];
            let my_squad = self.squad[i];
            friends.each_near(me, r, |j, pj| {
                let d = me - pj;
                let d2 = d.len2();
                // Only bodies that overlap push.
                if d2 >= r * r || j == i || self.state[j] == SoldierState::Routing {
                    return;
                }
                let share = if self.squad[j] != my_squad || self.rank[j] == self.rank[i] {
                    0.5
                } else if self.rank[j] < self.rank[i] {
                    1.0
                } else {
                    0.0
                };
                let min = self.body[i] + self.body[j];
                plan.crush += u8::from(d2 < min * min);
                if share > 0.0 {
                    plan.push += self.shove(i, j, d, d2, share);
                }
            });
        }

        plan.morale = self.drift(i, c, best != NONE);
        plan
    }

    /// How far soldier `i` is pushed off soldier `j`, `d` away (squared
    /// `d2`), taking `share` of the overlap.
    #[inline]
    fn shove(&self, i: usize, j: usize, d: V2, d2: f32, share: f32) -> V2 {
        let min = self.body[i] + self.body[j];
        if d2 >= min * min {
            return V2::ZERO;
        }
        let l = d2.sqrt();
        // Two on one spot step apart along the field's x, the lower index left.
        let n = if l > 1e-4 {
            d * (1.0 / l)
        } else {
            V2::new(if i < j { -1.0 } else { 1.0 }, 0.0)
        };
        n * ((min - l) * share)
    }

    /// True if a friend stands in the way of a blow from `me` along `u`, `d` long.
    fn blocked(&self, i: usize, me: V2, u: V2, d: f32) -> bool {
        let friends = &self.grids[self.side[i] as usize];
        friends.near(me + u * (d * 0.5), d * 0.5 + 0.5).any(|j| {
            if j == i || self.state[j] == SoldierState::Routing {
                return false;
            }
            let v = self.pos[j] - me;
            let along = v.dot(u);
            along > 0.3 && along < d - 0.3 && u.cross(v).abs() < 0.5
        })
    }

    /// Nerve gained or lost from what is around a soldier.
    fn drift(&self, i: usize, c: &SquadCtl, fighting: bool) -> f32 {
        let p = &self.params;
        let sp = &p.soldiers;
        let dt = sp.substep;
        let f = self.fatigue[i];
        let mut dm = -sp.fatigue_morale * f * f * dt;
        if !fighting && self.quiet[i] >= sp.quiet_after && self.morale[i] < self.start_morale[i] {
            dm += p.morale_regen * dt;
            let near_leader = c.leader.is_some_and(|l| {
                self.state[l].present()
                    && (self.pos[l] - self.pos[i]).len2() <= sp.leader_radius * sp.leader_radius
            });
            if near_leader {
                dm += sp.leader_regen * dt;
            }
        }
        dm
    }

    /// Phases 3 and 4 for soldier `i`: their plan into `p`, and their blow
    /// or shot, if they make one that reaches anybody, onto `blows`. Most
    /// stand idle in any substep.
    #[inline]
    pub(super) fn plan_and_blow(&self, i: usize, p: &mut Plan, blows: &mut Vec<(u32, Blow)>) {
        *p = self.plan(i);
        if p.act != Act::None {
            let b = self.resolve(i, p);
            if b.target != NONE {
                blows.push((i as u32, b));
            }
        }
    }

    /// Phase 4: what soldier `i`'s blow or shot does.
    pub(super) fn resolve(&self, i: usize, plan: &Plan) -> Blow {
        let mut b = Blow::default();
        let p = &self.params;
        match plan.act {
            Act::None => {}
            Act::Strike => {
                let mut d = Draw::new(self.seed, self.substep, i, Purpose::Hit);
                self.strike(i, plan.target as usize, false, 0.0, &mut d, &mut b);
            }
            Act::Charge => {
                let t = plan.target as usize;
                let u = (self.pos[t] - self.pos[i]).norm_or(self.facing[i]);
                let closing = (self.vel[i] - self.vel[t]).dot(u).max(0.0);
                if self.braced(t, i) {
                    // They run onto the spear: its blow carries their speed.
                    let mut d = Draw::new(self.seed, self.substep, i, Purpose::Counter);
                    let mut counter = Blow::default();
                    self.strike(t, i, false, p.charge_damage * closing, &mut d, &mut counter);
                    b.counter = if counter.hit { counter.damage } else { 0.0 };
                    b.counter_by = t as u32;
                    b.target = plan.target;
                } else {
                    // Impact: a horse and rider hit with the full charge
                    // damage per metre per second, a man with his mass's share.
                    let k = &self.kits[self.kit[i] as usize];
                    let full = p.soldiers.man_kg + p.soldiers.horse_kg;
                    let extra = p.charge_damage * closing * k.mass / full;
                    let mut d = Draw::new(self.seed, self.substep, i, Purpose::Hit);
                    self.strike(i, t, true, extra, &mut d, &mut b);
                }
            }
            Act::Shoot => self.shoot(i, plan.aim as usize, &mut b),
            Act::Capture => {
                b.target = plan.target;
                b.capture = true;
            }
        }
        b
    }

    /// Whether soldier `t` stands braced against a charge from `i`: on foot,
    /// still, steady, with a weapon as long, facing them.
    fn braced(&self, t: usize, i: usize) -> bool {
        let (kt, ki) = (
            &self.kits[self.kit[t] as usize].kit,
            &self.kits[self.kit[i] as usize].kit,
        );
        let toward = (self.pos[i] - self.pos[t]).norm_or(self.facing[t]);
        matches!(self.state[t], SoldierState::Formed | SoldierState::Engaged)
            && !kt.mounted
            && kt.reach + 0.01 >= ki.reach
            && self.morale[t] >= self.params.waver_at
            && self.vel[t].len() < 0.5
            && self.facing[t].dot(toward) > 0.0
    }

    /// One blow from `i` on `t`, a `charge` (with its own dice and `extra`
    /// hit points of impact) or not, rolled on `d`: an attack roll against
    /// their armour class, then the dice.
    fn strike(&self, i: usize, t: usize, charge: bool, extra: f32, d: &mut Draw, b: &mut Blow) {
        let p = &self.params;
        let (ki, kt) = (
            &self.kits[self.kit[i] as usize].kit,
            &self.kits[self.kit[t] as usize].kit,
        );
        let to = self.pos[i] - self.pos[t];
        let dist = to.len();
        let from = to.norm_or(self.facing[t]);
        let face = self.facing[t].dot(from);
        // A man climbing or pushing through the breach can't parry.
        let on_wall = self.squads[self.squad[i] as usize].spec.behind_wall
            && self.field.wall_x.is_some_and(|w| self.pos[t].x < w);
        b.angle = if face < REAR {
            2
        } else if face < 0.0 {
            1
        } else {
            0
        };
        // A blow in the flank or the rear or on a runner meets no shield and
        // has advantage; so has one on a man in a crush. A man climbing the
        // wall can't hold up his shield, and the defender on it has the
        // higher ground (`wall_ground`). A frightened soldier, or one held in
        // a crush, strikes with disadvantage.
        let flank = b.angle > 0 || self.state[t] == SoldierState::Routing;
        let open = flank || on_wall;
        let roll = Roll::of(
            flank || self.restrained[t],
            self.frightened(i) || self.restrained[i],
        );
        let (u, v) = (d.f32(), d.f32());
        let ground = if on_wall { p.wall_ground } else { 0.0 };
        let bonus = self.attack_bonus(i) + ground;
        let (hit, crit) = rules::attack(bonus, kt.armour_class(!open), roll, u, v);
        // Once an enemy is well inside a long weapon's reach, it is clumsy;
        // on a wall nobody gets inside a defender's reach.
        let clumsy = dist < ki.reach - 1.0 && !on_wall && d.f32() < ki.close_penalty;
        b.target = t as u32;
        b.hit = hit && !clumsy;
        if b.hit {
            let rear = if b.angle == 2 {
                p.soldiers.rear_damage
            } else {
                1.0
            };
            let dice = if charge { ki.charge_dice } else { ki.dice };
            let hp = rules::roll_dice(dice, crit, || d.f32()) + extra;
            b.damage = hp * kt.taken[ki.damage_type.index()] * rear / self.max_hp[t];
        }
    }

    /// One arrow from `i` aimed at `a`. It lands with a scatter that puts
    /// it on a lone man `arrow_aim` of the time, on whoever stands there;
    /// then its attack roll, which loses `range_penalty` at full range,
    /// against their armour class decides whether it wounds.
    fn shoot(&self, i: usize, a: usize, b: &mut Blow) {
        let p = &self.params;
        let sp = &p.soldiers;
        let ki = &self.kits[self.kit[i] as usize].kit;
        let mut d = Draw::new(self.seed, self.substep, i, Purpose::Scatter);
        let me = self.pos[i];
        let dist = (self.pos[a] - me).len();
        let chance = p.arrow_aim.clamp(0.01, 0.95);
        // P(|offset| < r) = 1 - exp(-r^2 / 2s^2) for a normal scatter s.
        let s = sp.hit_radius / (-2.0 * (1.0 - chance).ln()).sqrt();
        let land = self.pos[a] + V2::new(d.normal(), d.normal()) * s;
        let reach = sp.hit_radius * p.mounted_size;
        let side = self.side[i] as usize;
        let mut victim = None;
        let mut best = f32::INFINITY;
        let grids: &[usize] = if sp.friendly_fire {
            &[0, 1]
        } else if side == 0 {
            &[1]
        } else {
            &[0]
        };
        for &g in grids {
            for j in self.grids[g].near(land, reach) {
                let r = if self.kits[self.kit[j] as usize].kit.mounted {
                    reach
                } else {
                    sp.hit_radius
                };
                let d2 = (self.pos[j] - land).len2();
                if d2 <= r * r && d2 < best && j != i {
                    best = d2;
                    victim = Some(j);
                }
            }
        }
        let Some(v) = victim else {
            return;
        };
        let kv = &self.kits[self.kit[v] as usize].kit;
        // A shield counts from the front; a man on the wall has cover from below.
        let from = (me - self.pos[v]).norm_or(self.facing[v]);
        let front = self.state[v] != SoldierState::Routing && self.facing[v].dot(from) > 0.0;
        let covered = self.field.wall_x.is_some_and(|w| {
            self.squads[self.squad[v] as usize].spec.behind_wall && self.pos[v].x > w && me.x < w
        });
        let ac = kv.armour_class(front) + if covered { p.wall_cover } else { 0.0 };
        let bonus = self.attack_bonus(i) - p.range_penalty * dist / ki.range.max(1.0);
        let roll = Roll::of(self.restrained[v], self.frightened(i));
        let (u, w) = (d.f32(), d.f32());
        let (hit, crit) = rules::attack(bonus, ac, roll, u, w);
        b.target = v as u32;
        b.hit = hit;
        if b.hit {
            let hp = rules::roll_dice(ki.shot_dice, crit, || d.f32());
            b.damage = hp * kv.taken[ki.shot_type.index()] / self.max_hp[v];
        }
    }
}
