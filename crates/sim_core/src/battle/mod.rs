//! Squad battles on a small field.
//!
//! The war redesign (design note "Econoplex war redesign") makes the squad the
//! unit of war: 10 to 40 people of one type, raised by one town, with one morale
//! and one place on the field. This module fights one battle between squads,
//! second by second, from physical properties only:
//!
//! - **Kit** gives reach, strike damage and rate, bow range and arrows, armour,
//!   a shield and whether the squad is mounted. Spears outreach swords, a lance
//!   hits hard at a gallop, mail stops arrows.
//! - **People** give skill and experience. Experience (battles survived) counts
//!   heavily, both in how well a fighter strikes and in the squad's discipline.
//! - **Morale** starts from the squad's needs (fed, warm, paid; computed by the
//!   caller) and falls with losses and with being hit in the flank. A wavering
//!   squad fights worse; a routing one runs for the edge of the field and goes home.
//!
//! The well-known matchups come out of those numbers rather than out of a table:
//! a braced spear wall turns a charge onto its own points, cavalry rides archers
//! down, arrows thin a slow levy, swords get inside a spear's reach, and walls
//! narrow the attacker's front to the breach.
//!
//! The battle is deterministic for a seed. It can run to the end on its own
//! (auto-resolve), or a player can pause between steps and change orders: the
//! same rules run either way. The caller (a raid, `systems::military`, or the
//! `--battle` command) builds [`SquadSpec`]s and reads the [`Report`] (and
//! updated members) back.

mod combat;
mod kit;
mod lineup;
mod report;
mod step;
#[cfg(test)]
mod tests;

pub use kit::*;
pub use lineup::*;
pub use report::*;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Physical constants of a battle (`battle:` in `models.ron`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BattleParams {
    /// Seconds per step, and the longest a battle may last.
    pub dt: f32,
    pub max_seconds: f32,
    /// Speeds in metres per second: marching and running on foot, walking and
    /// galloping on horseback.
    pub walk_speed: f32,
    pub run_speed: f32,
    pub horse_walk_speed: f32,
    pub horse_charge_speed: f32,
    /// Squads break into a run (or a gallop) this close to their target, in metres.
    pub charge_distance: f32,
    /// Metres per fighter along the front, on foot and mounted, and the widest
    /// front a squad forms, in metres.
    pub foot_spacing: f32,
    pub horse_spacing: f32,
    pub frontage: f32,
    /// Chance a strike lands between equally skilled fighters, and how steeply
    /// the chance follows the difference in effective skill.
    pub base_hit: f32,
    pub skill_slope: f32,
    /// Effective skill gained per log of battles survived: skill + weight * ln(1 + battles).
    pub experience_weight: f32,
    /// Battles after which experience gives full discipline.
    pub experience_full: f32,
    /// Share of a shield's cover that also turns blows from the front.
    pub shield_parry: f32,
    /// Metres per second at which a shorter weapon closes inside a longer one's reach.
    pub close_speed: f32,
    /// Extra damage per metre per second of closing speed in a charge, for a horse
    /// (and a spear braced against one). A running man hits with `foot_charge` of it.
    pub charge_damage: f32,
    pub foot_charge: f32,
    /// Chance an arrow hits a man at point-blank range, the share of that lost
    /// at full range, and how much bigger a mounted target is.
    pub shot_accuracy: f32,
    pub range_falloff: f32,
    pub mounted_size: f32,
    /// A fighter whose health falls below this is dead; above it, wounded.
    pub death_below: f32,
    /// Morale lost per share of the squad lost, per second of being hit in the
    /// flank, and regained per second out of the fight.
    pub morale_per_loss: f32,
    pub flank_drain: f32,
    pub morale_regen: f32,
    /// Below `waver_at` a squad fights worse; below `rout_at` it runs.
    pub waver_at: f32,
    pub rout_at: f32,
    /// Walls: share of a defender on the wall hidden from arrows, and the width
    /// of the gate or breach the attackers must fight through, in metres.
    pub wall_cover: f32,
    pub breach_width: f32,
}

impl Default for BattleParams {
    fn default() -> Self {
        Self {
            dt: 1.0,
            max_seconds: 3.0 * 3600.0,
            walk_speed: 1.4,
            run_speed: 3.0,
            horse_walk_speed: 2.5,
            horse_charge_speed: 8.0,
            charge_distance: 60.0,
            foot_spacing: 1.0,
            horse_spacing: 2.0,
            frontage: 40.0,
            base_hit: 0.35,
            skill_slope: 3.0,
            experience_weight: 0.15,
            experience_full: 10.0,
            shield_parry: 0.5,
            close_speed: 0.5,
            charge_damage: 0.08,
            foot_charge: 0.3,
            shot_accuracy: 0.5,
            range_falloff: 0.6,
            mounted_size: 1.5,
            death_below: -0.5,
            morale_per_loss: 1.5,
            flank_drain: 0.01,
            morale_regen: 0.002,
            waver_at: 0.4,
            rout_at: 0.15,
            wall_cover: 0.6,
            breach_width: 6.0,
        }
    }
}

/// The field: its size, and a wall at `wall_x` if the defenders have one
/// (defenders stand at larger x than the wall, attackers at smaller).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Field {
    pub width: f32,
    pub height: f32,
    pub wall_x: Option<f32>,
}

impl Default for Field {
    fn default() -> Self {
        Self {
            width: 600.0,
            height: 400.0,
            wall_x: None,
        }
    }
}

/// An order to one squad. `Auto` lets the squad's own judgment decide, which is
/// also what an auto-resolved battle uses for every squad.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Order {
    Auto,
    /// Stand where it is; spearmen brace.
    Hold,
    /// Close with this squad and fight it.
    Attack(usize),
    /// Shoot this squad while it is in range; close no further.
    Shoot(usize),
    MoveTo(f32, f32),
    /// Leave the field in good order (counts as fled, not routed).
    Withdraw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SquadState {
    Fighting,
    Routing,
    /// Left the field, by routing or by withdrawing; it marches home.
    Fled,
    /// Everyone is down.
    Destroyed,
}

#[derive(Clone, Debug)]
struct Squad {
    spec: SquadSpec,
    state: SquadState,
    morale: f32,
    order: Order,
    /// Arrows left for the whole squad.
    arrows: u32,
    /// The enemy this squad faces; blows from any other enemy in contact are flank blows.
    primary: Option<usize>,
    /// Enemies in melee contact last step.
    contacts: Vec<usize>,
    /// Seconds until a shorter weapon is inside this squad's reach (or this
    /// squad's inside the other's), per enemy in contact.
    closing: Vec<(usize, f32)>,
    /// Metres moved last step, per second.
    speed: f32,
    /// Seconds since the squad last lost anyone or was in contact.
    quiet: f32,
    engaged: bool,
    wavering: bool,
    /// Effective skill and discipline, refreshed every step.
    eff: f32,
    discipline: f32,
}

impl Squad {
    fn alive(&self) -> usize {
        self.spec.members.iter().filter(|m| m.health > 0.0).count()
    }
    fn fighting(&self) -> bool {
        self.state == SquadState::Fighting
    }
    fn on_field(&self) -> bool {
        matches!(self.state, SquadState::Fighting | SquadState::Routing)
    }
    fn spacing(&self, p: &BattleParams) -> f32 {
        if self.spec.kit.mounted {
            p.horse_spacing
        } else {
            p.foot_spacing
        }
    }
    /// Half the depth of the formation: a squad is a disc for distances.
    fn radius(&self, p: &BattleParams) -> f32 {
        (self.alive() as f32).sqrt() * self.spacing(p) * 0.5 + 0.5
    }
    fn front(&self, p: &BattleParams) -> f32 {
        (self.alive() as f32).min(p.frontage / self.spacing(p))
    }
    fn has_bow(&self) -> bool {
        self.spec.kit.range > 0.0 && self.arrows > 0
    }
}

/// A battle in progress.
pub struct Battle {
    pub params: BattleParams,
    pub field: Field,
    squads: Vec<Squad>,
    t: f32,
    rng: ChaCha8Rng,
    events: Vec<Event>,
}

impl Battle {
    pub fn new(params: BattleParams, field: Field, specs: Vec<SquadSpec>, seed: u64) -> Self {
        let squads = specs
            .into_iter()
            .map(|spec| Squad {
                arrows: spec.kit.arrows * spec.members.len() as u32,
                morale: spec.morale.clamp(0.0, 1.0),
                state: if spec.members.iter().any(|m| m.health > 0.0) {
                    SquadState::Fighting
                } else {
                    SquadState::Destroyed
                },
                spec,
                order: Order::Auto,
                primary: None,
                contacts: Vec::new(),
                closing: Vec::new(),
                speed: 0.0,
                quiet: 0.0,
                engaged: false,
                wavering: false,
                eff: 0.0,
                discipline: 0.0,
            })
            .collect();
        Self {
            params,
            field,
            squads,
            t: 0.0,
            rng: ChaCha8Rng::seed_from_u64(seed),
            events: Vec::new(),
        }
    }

    /// Give a squad an order (between steps: the player's pause).
    pub fn set_order(&mut self, squad: usize, order: Order) {
        if let Some(s) = self.squads.get_mut(squad) {
            s.order = order;
        }
    }

    pub fn seconds(&self) -> f32 {
        self.t
    }

    pub fn position(&self, squad: usize) -> (f32, f32) {
        self.squads[squad].spec.pos
    }

    pub fn state(&self, squad: usize) -> SquadState {
        self.squads[squad].state
    }

    pub fn morale(&self, squad: usize) -> f32 {
        self.squads[squad].morale
    }

    /// The battle is over when time is up, or when one side has no squad left
    /// fighting and every routing squad has left the field (pursuit goes on until then).
    pub fn finished(&self) -> bool {
        if self.t >= self.params.max_seconds {
            return true;
        }
        let decided = self.sides_fighting().iter().filter(|&&b| b).count() < 2;
        decided && !self.squads.iter().any(|s| s.state == SquadState::Routing)
    }

    fn sides_fighting(&self) -> [bool; 2] {
        let mut f = [false; 2];
        for s in &self.squads {
            if s.fighting() {
                f[(s.spec.side as usize).min(1)] = true;
            }
        }
        f
    }

    /// Run to the end with the current orders (auto-resolve).
    pub fn run(&mut self) -> Report {
        while !self.finished() {
            self.step();
        }
        self.finish()
    }

    pub fn field_size(&self) -> (f32, f32) {
        (self.field.width, self.field.height)
    }

    pub fn side(&self, squad: usize) -> Option<u8> {
        self.squads.get(squad).map(|s| s.spec.side)
    }

    /// The battle as the game shows it while the player fights it.
    pub fn live_view(
        &self,
        town: u16,
        at: (f32, f32),
        facing: (f32, f32),
        player_side: u8,
    ) -> LiveView {
        let finished = self.finished();
        LiveView {
            town,
            at,
            facing,
            field: self.field_size(),
            squads: self
                .squads
                .iter()
                .map(|s| ReplaySquad {
                    name: s.spec.name.clone(),
                    side: s.spec.side,
                    mounted: s.spec.kit.mounted,
                    start: s.spec.members.len() as u16,
                })
                .collect(),
            orders: self.squads.iter().map(|s| s.order).collect(),
            morale: self.squads.iter().map(|s| s.morale).collect(),
            frame: self.frame(),
            player_side,
            finished,
            winner: if finished { self.report().winner } else { None },
        }
    }

    /// Where every squad stands now.
    pub fn frame(&self) -> ReplayFrame {
        ReplayFrame {
            t: self.t,
            squads: self
                .squads
                .iter()
                .map(|s| (s.spec.pos.0, s.spec.pos.1, s.alive() as u16, s.state))
                .collect(),
        }
    }

    /// Like [`Self::run`], recording a frame every `every` seconds (and the last).
    pub fn run_recorded(&mut self, every: f32) -> (Report, Vec<ReplaySquad>, Vec<ReplayFrame>) {
        let squads = self
            .squads
            .iter()
            .map(|s| ReplaySquad {
                name: s.spec.name.clone(),
                side: s.spec.side,
                mounted: s.spec.kit.mounted,
                start: s.alive() as u16,
            })
            .collect();
        let mut frames = vec![self.frame()];
        let mut next = every;
        while !self.finished() {
            self.step();
            if self.t >= next {
                frames.push(self.frame());
                next += every;
            }
        }
        frames.push(self.frame());
        (self.finish(), squads, frames)
    }

    /// Survivors gain a battle's experience; the report is returned and the
    /// members (with their wounds and experience) can be read back with [`Self::members`].
    pub fn finish(&mut self) -> Report {
        for s in &mut self.squads {
            if s.engaged {
                for m in &mut s.spec.members {
                    if m.health > 0.0 {
                        m.experience += 1.0;
                    }
                }
                s.engaged = false;
            }
        }
        self.report()
    }

    pub fn members(&self, squad: usize) -> &[Member] {
        &self.squads[squad].spec.members
    }

    pub fn report(&self) -> Report {
        let f = self.sides_fighting();
        let winner = match f {
            [true, false] => Some(0),
            [false, true] => Some(1),
            [false, false] => Some(1), // nobody left: the defenders keep their town
            _ => None,
        };
        let death = self.params.death_below;
        Report {
            winner,
            seconds: self.t,
            squads: self
                .squads
                .iter()
                .map(|s| {
                    let m = &s.spec.members;
                    SquadReport {
                        name: s.spec.name.clone(),
                        side: s.spec.side,
                        kit: s.spec.kit.name.clone(),
                        start: m.len(),
                        standing: m.iter().filter(|m| m.health > 0.0).count(),
                        wounded: m
                            .iter()
                            .filter(|m| m.health <= 0.0 && m.health > death)
                            .count(),
                        dead: m.iter().filter(|m| m.health <= death).count(),
                        state: s.state,
                        morale: s.morale,
                    }
                })
                .collect(),
            events: self.events.clone(),
        }
    }

    fn event(&mut self, kind: EventKind) {
        self.events.push(Event { t: self.t, kind });
    }

    fn dist(&self, a: usize, b: usize) -> f32 {
        let (p, q) = (self.squads[a].spec.pos, self.squads[b].spec.pos);
        ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
    }

    /// Distance between the squads' edges.
    fn gap(&self, a: usize, b: usize) -> f32 {
        let p = &self.params;
        self.dist(a, b) - self.squads[a].radius(p) - self.squads[b].radius(p)
    }

    fn enemies(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let side = self.squads[i].spec.side;
        (0..self.squads.len()).filter(move |&j| self.squads[j].spec.side != side)
    }

    fn nearest(&self, i: usize, keep: impl Fn(&Squad) -> bool) -> Option<usize> {
        self.enemies(i)
            .filter(|&j| keep(&self.squads[j]))
            .min_by(|&a, &b| self.dist(i, a).total_cmp(&self.dist(i, b)).then(a.cmp(&b)))
    }
}
