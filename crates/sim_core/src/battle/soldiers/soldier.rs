//! Phases 3 and 4: each soldier picks where to step and whom to strike or
//! shoot, then each strike and shot is rolled. Both only read the last
//! substep's state, so they can run in any order and on any thread.

use super::*;
use crate::battle::rules::{self, Roll};

use crate::battle::walls::OVER_WALL;

/// cos 135°: a blow from further round than this comes from behind.
const REAR: f32 = -std::f32::consts::FRAC_1_SQRT_2;

impl SoldierBattle {
    /// Phase 3: what soldier `i` means to do.
    #[inline(always)]
    pub(super) fn plan(&self, i: usize) -> Plan {
        let man = &self.soldier[i];
        let state = man.state;
        let mut plan = Plan {
            facing: man.facing,
            ..Plan::default()
        };
        if !state.present() {
            return plan;
        }
        let p = &self.params;
        let sp = &p.soldiers;
        let dt = sp.substep;
        let k = &self.kits[man.kit as usize];
        let c = &self.squads[man.squad as usize];
        let top = k.top * (1.0 - sp.fatigue_effect * man.fatigue);
        let me = man.pos;
        let side = (man.side as usize).min(1);
        let (friends, foes) = (&self.grids[side], &self.grids[side ^ 1]);
        let foes_near = c.foes_around && foes.anyone_around(foes.cell(me));

        if state == SoldierState::Routing {
            // Away from the nearest enemies, toward their own edge of the
            // field; round the enemy when they stand in the way.
            let edge = V2::new(if side == 0 { -1.0 } else { 1.0 }, 0.0);
            // Among obstacles, down the side's flow field to its edge.
            // One pressed into an obstacle steps out of it first.
            let home = self
                .nav
                .as_deref()
                .and_then(|n| {
                    n.home_dir(me, side as u8).or_else(|| {
                        let out = n.free_spot(me, side as u8) - me;
                        (out.len2() > 1e-4).then(|| out.norm_or(edge))
                    })
                })
                .unwrap_or(edge);
            let mut away = V2::ZERO;
            if foes_near {
                let cell2 = sp.cell * sp.cell;
                foes.each_within(me, sp.cell, cell2, |j, pj| {
                    let d = me - pj;
                    let d2 = d.len2();
                    if d2 >= cell2 {
                        return cell2;
                    }
                    if d2 > 1e-6 {
                        away += d * (2.0 / d2);
                    }
                    plan.push += self.shove(i, j, d, d2, 0.5);
                    cell2
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
                dir = (dir + man.vel.norm_or(V2::ZERO) * 1.5).norm_or(home);
            }
            plan.vel = dir * top;
            plan.facing = dir;
            return plan;
        }

        // The nearest enemy within reach, or near the slot; and bodies to push off.
        let mut slot = c.slot(man.file, man.rank);
        // In contact, the ranks behind keep a pace behind the man in front:
        // the front presses, the rear does not crowd it.
        let ahead = man.ahead as usize;
        if c.in_contact > 0 && ahead != NONE as usize && self.soldier[ahead].state.in_line() {
            let lim = self.soldier[ahead].pos - c.facing * c.spacing;
            let over = (slot - lim).dot(c.facing);
            if over > 0.0 {
                slot -= c.facing * over;
            }
        }
        // Where a slot falls on an obstacle, the nearest place beside it.
        if self.nav.as_deref().is_some_and(|n| n.any_near(slot)) {
            slot = self.clear_slot(slot, i);
        }
        let front = ahead == NONE as usize || !self.soldier[ahead].state.in_line();
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
            let over = if self.ring.is_some() && side == 1 {
                OVER_WALL
            } else {
                0.0
            };
            let r = look.max(k.body) + self.widest[1 - side] + over;
            // Nobody further than this touches them: their body and the
            // widest of the enemy's.
            let touch = man.body + self.widest[1 - side];
            let touch2 = touch * touch;
            let chases = c.chases();
            // Nobody further than `r` is pushed against or picked, and once
            // somebody is picked, nobody further than them.
            let r2 = r * r;
            let bound = |best_d2: f32| r2.min(touch2.max(best_d2));
            foes.each_within(me, r.min(sp.cell), r2, |j, pj| {
                let d = me - pj;
                let d2 = d.len2();
                // Most of them stand too far to touch and no nearer than
                // the nearest so far: nothing to read of them.
                if d2 >= touch2 && d2 >= best_d2 {
                    return bound(best_d2);
                }
                let them = &self.soldier[j];
                if d2 < touch2 {
                    let shove = self.shove(i, j, d, d2, 0.5);
                    plan.crush += u8::from(shove != V2::ZERO);
                    plan.push += shove;
                }
                // A squad that lets enemies go strikes nobody who runs (one
                // that takes prisoners goes for them, to take them).
                if !chases && them.state == SoldierState::Routing {
                    return bound(best_d2);
                }
                if d2 < best_d2 {
                    let rj = reach + them.body;
                    // In reach, or (for the front rank) near the slot.
                    // A defender on a town's wall strikes down over it a
                    // little further.
                    let close = d2 <= rj * rj
                        || (front && (pj - slot).len2() <= slack2)
                        || (over > 0.0
                            && d2 <= (rj + over) * (rj + over)
                            && self.nav.as_deref().is_some_and(|n| n.wall_across(me, pj)));
                    if close && !self.wall_between(me, pj, breach) && self.over_wall(i, me, pj) {
                        best = j as u32;
                        best_d2 = d2;
                    }
                }
                bound(best_d2)
            });
        }

        if best != NONE {
            let t = best as usize;
            let kt = &self.kits[self.soldier[t].kit as usize];
            let d = best_d2.sqrt();
            let u = (self.soldier[t].pos - me).norm_or(c.facing);
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
                let running = man.vel.len() >= p.run_speed - 0.1;
                let steady = matches!(state, SoldierState::Formed | SoldierState::Engaged);
                plan.act = if steady && running {
                    Act::Charge
                } else if man.cooldown <= 0.0 {
                    Act::Strike
                } else {
                    Act::None
                };
                if plan.act != Act::None && self.blocked(i, me, u, d) {
                    plan.act = Act::None;
                }
                // A runner within reach is taken, not struck.
                if plan.act != Act::None
                    && !c.pursues()
                    && self.soldier[t].state == SoldierState::Routing
                {
                    plan.act = Act::Capture;
                }
            }
        } else {
            plan.facing = c.facing;
            // Shoot at a soldier of the squad's mark, if they are in range
            // and no enemy is close enough to make them draw the knife.
            if let Some(q) = c.shoot_at {
                let free = !foes_near && man.arrows > 0 && man.cooldown <= 0.0;
                if k.kit.range > 0.0 && free {
                    let q = &self.squads[q];
                    let mut d = Draw::new(self.seed, self.substep, i, Purpose::Aim);
                    let n = q.spec.members.len().max(1);
                    let aim = (0..3)
                        .map(|_| q.first + d.below(n))
                        .find(|&j| self.soldier[j].state.present());
                    if let Some(a) = aim {
                        let to = self.soldier[a].pos - me;
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
            let my_squad = man.squad;
            friends.each_within(me, r, r * r, |j, pj| {
                let d = me - pj;
                let d2 = d.len2();
                // Only bodies that overlap push.
                if d2 >= r * r || j == i || self.soldier[j].state == SoldierState::Routing {
                    return r * r;
                }
                let share = if self.soldier[j].squad != my_squad || self.soldier[j].rank == man.rank
                {
                    0.5
                } else if self.soldier[j].rank < man.rank {
                    1.0
                } else {
                    0.0
                };
                let min = man.body + self.soldier[j].body;
                plan.crush += u8::from(d2 < min * min);
                if share > 0.0 {
                    plan.push += self.shove(i, j, d, d2, share);
                }
                r * r
            });
        }

        plan.morale = self.drift(man, c, best != NONE);
        plan
    }

    /// How far soldier `i` is pushed off soldier `j`, `d` away (squared
    /// `d2`), taking `share` of the overlap.
    #[inline]
    fn shove(&self, i: usize, j: usize, d: V2, d2: f32, share: f32) -> V2 {
        let min = self.soldier[i].body + self.soldier[j].body;
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
        let friends = &self.grids[self.soldier[i].side as usize];
        friends.near(me + u * (d * 0.5), d * 0.5 + 0.5).any(|j| {
            if j == i || self.soldier[j].state == SoldierState::Routing {
                return false;
            }
            let v = self.soldier[j].pos - me;
            let along = v.dot(u);
            along > 0.3 && along < d - 0.3 && u.cross(v).abs() < 0.5
        })
    }

    /// Nerve gained or lost from what is around a soldier.
    fn drift(&self, man: &Soldier, c: &SquadCtl, fighting: bool) -> f32 {
        let p = &self.params;
        let sp = &p.soldiers;
        let dt = sp.substep;
        let f = man.fatigue;
        let mut dm = -sp.fatigue_morale * f * f * dt;
        if !fighting && man.quiet >= sp.quiet_after && man.morale < man.start_morale {
            dm += p.morale_regen * dt;
            let near_leader = c.leader.is_some_and(|l| {
                let leader = &self.soldier[l];
                leader.state.present()
                    && (leader.pos - man.pos).len2() <= sp.leader_radius * sp.leader_radius
            });
            if near_leader {
                dm += sp.leader_regen * dt;
            }
        }
        dm
    }

    /// Phases 3 and 4 for soldier `i`, who is on the field: their plan into
    /// `p`, and their blow or shot, if they make one that reaches anybody,
    /// onto `blows`. Most stand idle in any substep.
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
                let u = (self.soldier[t].pos - self.soldier[i].pos).norm_or(self.soldier[i].facing);
                let closing = (self.soldier[i].vel - self.soldier[t].vel).dot(u).max(0.0);
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
                    let k = &self.kits[self.soldier[i].kit as usize];
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
            &self.kits[self.soldier[t].kit as usize].kit,
            &self.kits[self.soldier[i].kit as usize].kit,
        );
        let toward = (self.soldier[i].pos - self.soldier[t].pos).norm_or(self.soldier[t].facing);
        matches!(
            self.soldier[t].state,
            SoldierState::Formed | SoldierState::Engaged
        ) && !kt.mounted
            && kt.reach + 0.01 >= ki.reach
            && self.soldier[t].morale >= self.params.waver_at
            && self.soldier[t].vel.len() < 0.5
            && self.soldier[t].facing.dot(toward) > 0.0
    }

    /// Whether `i` at `me` may strike a man at `pj` for a town's wall:
    /// nobody strikes through a stretch of it from outside; a defender
    /// strikes down over it.
    #[inline]
    fn over_wall(&self, i: usize, me: V2, pj: V2) -> bool {
        self.soldier[i].side == 1
            || self.ring.is_none()
            || !self.nav.as_deref().is_some_and(|n| n.wall_across(me, pj))
    }

    /// One blow from `i` on `t`, a `charge` (with its own dice and `extra`
    /// hit points of impact) or not, rolled on `d`: an attack roll against
    /// their armour class, then the dice.
    fn strike(&self, i: usize, t: usize, charge: bool, extra: f32, d: &mut Draw, b: &mut Blow) {
        let p = &self.params;
        let (ki, kt) = (
            &self.kits[self.soldier[i].kit as usize].kit,
            &self.kits[self.soldier[t].kit as usize].kit,
        );
        let to = self.soldier[i].pos - self.soldier[t].pos;
        let dist = to.len();
        let from = to.norm_or(self.soldier[t].facing);
        let face = self.soldier[t].facing.dot(from);
        // A man climbing or pushing through the breach can't parry, nor
        // one below a town's wall.
        let on_wall = (self.squads[self.soldier[i].squad as usize].spec.behind_wall
            && self.field.wall_x.is_some_and(|w| self.soldier[t].pos.x < w))
            || (self.ring.is_some()
                && self.soldier[i].side == 1
                && self
                    .nav
                    .as_deref()
                    .is_some_and(|n| n.wall_across(self.soldier[i].pos, self.soldier[t].pos)));
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
        let flank = b.angle > 0 || self.soldier[t].state == SoldierState::Routing;
        let open = flank || on_wall;
        let roll = Roll::of(
            flank || self.restrained[t],
            self.frightened(i) || self.restrained[i],
        );
        let (u, v) = (d.f32(), d.f32());
        let ground = if on_wall { p.wall_ground } else { 0.0 };
        let bonus = self.attack_bonus(i) + ground;
        let (hit, crit) = rules::attack(bonus, kt.armour_class(!open), roll, u, v);
        // Initiative, if blows meet: his own d20, agility and the weapon's
        // speed, and the longer weapon's edge on a man coming into its reach.
        // A charge comes first.
        b.init = if charge {
            rules::FIRST
        } else {
            let face = d.below(20) as i32 + 1;
            let agility = self.seasoned[i] - rules::exhaustion(self.soldier[i].fatigue) as f32;
            let outreach = if ki.reach > kt.reach + 0.5 {
                p.reach_initiative
            } else {
                0.0
            };
            rules::initiative(face, agility, ki.speed, outreach)
        };
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
        let ki = &self.kits[self.soldier[i].kit as usize].kit;
        let mut d = Draw::new(self.seed, self.substep, i, Purpose::Scatter);
        let me = self.soldier[i].pos;
        let dist = (self.soldier[a].pos - me).len();
        let chance = p.arrow_aim.clamp(0.01, 0.95);
        // P(|offset| < r) = 1 - exp(-r^2 / 2s^2) for a normal scatter s.
        let s = sp.hit_radius / (-2.0 * (1.0 - chance).ln()).sqrt();
        let land = self.soldier[a].pos + V2::new(d.normal(), d.normal()) * s;
        let reach = sp.hit_radius * p.mounted_size;
        let side = self.soldier[i].side as usize;
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
                let r = if self.kits[self.soldier[j].kit as usize].kit.mounted {
                    reach
                } else {
                    sp.hit_radius
                };
                let d2 = (self.soldier[j].pos - land).len2();
                if d2 <= r * r && d2 < best && j != i {
                    best = d2;
                    victim = Some(j);
                }
            }
        }
        let Some(v) = victim else {
            return;
        };
        let kv = &self.kits[self.soldier[v].kit as usize].kit;
        // A shield counts from the front; a man on the wall has cover from below.
        let from = (me - self.soldier[v].pos).norm_or(self.soldier[v].facing);
        let front = self.soldier[v].state != SoldierState::Routing
            && self.soldier[v].facing.dot(from) > 0.0;
        let covered = self.field.wall_x.is_some_and(|w| {
            self.squads[self.soldier[v].squad as usize].spec.behind_wall
                && self.soldier[v].pos.x > w
                && me.x < w
        }) || (self.ring.is_some()
            && self.soldier[v].side == 1
            && self.soldier[i].side == 0
            && self.nav.as_deref().is_some_and(|n| {
                // The wall's cover is near him: his last metres toward me.
                let d = (me - self.soldier[v].pos).norm_or(V2::ZERO);
                n.wall_across(self.soldier[v].pos, self.soldier[v].pos + d * 4.0)
            }));
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
