//! Battles between squads on a small field.
//!
//! The war redesign (design note "Econoplex war redesign") makes the squad the
//! unit of war: 10 to 40 people of one type, raised by one town, with one morale
//! and one place on the field. This module fights one battle between squads
//! from physical properties only:
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
//! Two models fight the same battle (`battle.model` in `models.ron`):
//!
//! - [`squads`]: each squad is a disc with one morale, settled with dice per
//!   squad each second. Cheap; the war planner's forecasts always use it.
//! - [`soldiers`]: every soldier has a place, a facing, a morale, a fatigue and
//!   their own kit, so flanks, broken lines and tired armies come out of the
//!   positions (design note `docs/design/battle-per-soldier.md`).
//!
//! [`Battle`] is either one, with the same methods and outputs. The battle is
//! deterministic for a seed. It can run to the end on its own (auto-resolve),
//! or a player can pause between steps and change orders: the same rules run
//! either way. The caller (a raid, `systems::military`, or the `--battle`
//! command) builds [`SquadSpec`]s and reads the [`Report`] (and updated
//! members) back.

mod kit;
mod lineup;
mod report;
pub mod soldiers;
pub mod squads;
#[cfg(test)]
mod tests;

pub use kit::*;
pub use lineup::*;
pub use report::*;

use serde::{Deserialize, Serialize};
use std::fmt;

/// Which model fights a battle (`battle.model` in `models.ron`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleModel {
    /// Squads as discs, dice per squad.
    #[default]
    Squads,
    /// Every soldier simulated.
    Soldiers,
}

impl std::str::FromStr for BattleModel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "squads" => Ok(Self::Squads),
            "soldiers" => Ok(Self::Soldiers),
            _ => Err(format!("unknown battle model `{s}` (squads or soldiers)")),
        }
    }
}

/// Physical constants of a battle (`battle:` in `models.ron`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BattleParams {
    /// Which model fights the battle.
    pub model: BattleModel,
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
    /// Constants only the soldier model uses.
    pub soldiers: soldiers::SoldierParams,
}

impl Default for BattleParams {
    fn default() -> Self {
        Self {
            model: BattleModel::Squads,
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
            soldiers: soldiers::SoldierParams::default(),
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

/// What both models keep, so [`Battle`] reports on either the same way.
trait Model {
    fn params(&self) -> &BattleParams;
    fn field(&self) -> &Field;
    fn seconds(&self) -> f32;
    /// Advance one step of `params.dt` battle seconds.
    fn step(&mut self);
    fn set_order(&mut self, squad: usize, order: Order);
    fn order(&self, squad: usize) -> Order;
    /// How many squads fight.
    fn count(&self) -> usize;
    /// A squad as it fights: its kit, side and members with their health now.
    fn spec(&self, squad: usize) -> &SquadSpec;
    fn position(&self, squad: usize) -> (f32, f32);
    fn state(&self, squad: usize) -> SquadState;
    fn morale(&self, squad: usize) -> f32;
    fn events(&self) -> &[Event];
    /// Whether the squad fought (struck, shot or was hit), and forget it.
    fn take_engaged(&mut self, squad: usize) -> bool;
    fn members_mut(&mut self, squad: usize) -> &mut [Member];
}

/// Which sides still have a squad fighting.
fn sides_fighting(m: &(impl Model + ?Sized)) -> [bool; 2] {
    let mut f = [false; 2];
    for s in 0..m.count() {
        if m.state(s) == SquadState::Fighting {
            f[(m.spec(s).side as usize).min(1)] = true;
        }
    }
    f
}

/// The battle is over when time is up, or when one side has no squad left
/// fighting and every routing squad has left the field (pursuit goes on until then).
fn finished(m: &(impl Model + ?Sized)) -> bool {
    if m.seconds() >= m.params().max_seconds {
        return true;
    }
    let decided = sides_fighting(m).iter().filter(|&&b| b).count() < 2;
    decided && !(0..m.count()).any(|s| m.state(s) == SquadState::Routing)
}

/// A battle in progress, on either model.
pub enum Battle {
    Squads(Box<squads::SquadBattle>),
    Soldiers(Box<soldiers::SoldierBattle>),
}

impl Battle {
    /// A battle on the model `params.model` chooses.
    pub fn new(params: BattleParams, field: Field, specs: Vec<SquadSpec>, seed: u64) -> Self {
        match params.model {
            BattleModel::Squads => Self::Squads(Box::new(squads::SquadBattle::new(
                params, field, specs, seed,
            ))),
            BattleModel::Soldiers => Self::Soldiers(Box::new(soldiers::SoldierBattle::new(
                params, field, specs, seed,
            ))),
        }
    }

    fn model(&self) -> &dyn Model {
        match self {
            Self::Squads(b) => b.as_ref(),
            Self::Soldiers(b) => b.as_ref(),
        }
    }

    fn model_mut(&mut self) -> &mut dyn Model {
        match self {
            Self::Squads(b) => b.as_mut(),
            Self::Soldiers(b) => b.as_mut(),
        }
    }

    pub fn params(&self) -> &BattleParams {
        self.model().params()
    }

    /// On the soldier model, time each phase from now on (`--profile`).
    pub fn enable_profile(&mut self) {
        if let Self::Soldiers(b) = self {
            b.enable_profile();
        }
    }

    /// Time spent per phase so far, on the soldier model with profiling on.
    pub fn profile(&self) -> Option<soldiers::Profile> {
        match self {
            Self::Squads(_) => None,
            Self::Soldiers(b) => b.profile(),
        }
    }

    /// How many fighters the battle started with.
    pub fn soldiers(&self) -> usize {
        let m = self.model();
        (0..m.count()).map(|s| m.spec(s).members.len()).sum()
    }

    /// Advance the battle by one step of `params.dt` seconds.
    pub fn step(&mut self) {
        if !self.finished() {
            self.model_mut().step();
        }
    }

    /// Give a squad an order (between steps: the player's pause).
    pub fn set_order(&mut self, squad: usize, order: Order) {
        if squad < self.model().count() {
            self.model_mut().set_order(squad, order);
        }
    }

    pub fn seconds(&self) -> f32 {
        self.model().seconds()
    }

    /// Where a squad stands: its centre, metres.
    pub fn position(&self, squad: usize) -> (f32, f32) {
        self.model().position(squad)
    }

    pub fn state(&self, squad: usize) -> SquadState {
        self.model().state(squad)
    }

    pub fn morale(&self, squad: usize) -> f32 {
        self.model().morale(squad)
    }

    /// The battle is over when time is up, or when one side has no squad left
    /// fighting and every routing squad has left the field (pursuit goes on until then).
    pub fn finished(&self) -> bool {
        finished(self.model())
    }

    /// Run to the end with the current orders (auto-resolve).
    pub fn run(&mut self) -> Report {
        while !self.finished() {
            self.step();
        }
        self.finish()
    }

    pub fn field_size(&self) -> (f32, f32) {
        let f = self.model().field();
        (f.width, f.height)
    }

    pub fn side(&self, squad: usize) -> Option<u8> {
        let m = self.model();
        (squad < m.count()).then(|| m.spec(squad).side)
    }

    fn replay_squads(&self) -> Vec<ReplaySquad> {
        let m = self.model();
        (0..m.count())
            .map(|s| {
                let spec = m.spec(s);
                ReplaySquad {
                    name: spec.name.clone(),
                    side: spec.side,
                    mounted: spec.kit.mounted,
                    start: spec.members.len() as u16,
                }
            })
            .collect()
    }

    /// The battle as the game shows it while the player fights it.
    pub fn live_view(
        &self,
        town: u16,
        at: (f32, f32),
        facing: (f32, f32),
        player_side: u8,
    ) -> LiveView {
        let m = self.model();
        let finished = self.finished();
        LiveView {
            town,
            at,
            facing,
            field: self.field_size(),
            squads: self.replay_squads(),
            orders: (0..m.count()).map(|s| m.order(s)).collect(),
            morale: (0..m.count()).map(|s| m.morale(s)).collect(),
            frame: self.frame(),
            player_side,
            finished,
            winner: if finished { self.report().winner } else { None },
        }
    }

    /// Where every squad stands now.
    pub fn frame(&self) -> ReplayFrame {
        let m = self.model();
        ReplayFrame {
            t: m.seconds(),
            squads: (0..m.count())
                .map(|s| {
                    let (x, y) = m.position(s);
                    (x, y, standing(m.spec(s)) as u16, m.state(s))
                })
                .collect(),
        }
    }

    /// Like [`Self::run`], recording a frame every `every` seconds (and the last).
    pub fn run_recorded(&mut self, every: f32) -> (Report, Vec<ReplaySquad>, Vec<ReplayFrame>) {
        let m = self.model();
        let squads = (0..m.count())
            .map(|s| {
                let spec = m.spec(s);
                ReplaySquad {
                    name: spec.name.clone(),
                    side: spec.side,
                    mounted: spec.kit.mounted,
                    start: standing(spec) as u16,
                }
            })
            .collect();
        let mut frames = vec![self.frame()];
        let mut next = every;
        while !self.finished() {
            self.step();
            if self.seconds() >= next {
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
        let m = self.model_mut();
        for s in 0..m.count() {
            if m.take_engaged(s) {
                for member in m.members_mut(s) {
                    if member.health > 0.0 {
                        member.experience += 1.0;
                    }
                }
            }
        }
        self.report()
    }

    pub fn members(&self, squad: usize) -> &[Member] {
        &self.model().spec(squad).members
    }

    pub fn report(&self) -> Report {
        let m = self.model();
        let winner = match sides_fighting(m) {
            [true, false] => Some(0),
            [false, true] => Some(1),
            [false, false] => Some(1), // nobody left: the defenders keep their town
            _ => None,
        };
        let death = m.params().death_below;
        Report {
            winner,
            seconds: m.seconds(),
            squads: (0..m.count())
                .map(|s| {
                    let spec = m.spec(s);
                    let members = &spec.members;
                    SquadReport {
                        name: spec.name.clone(),
                        side: spec.side,
                        kit: spec.kit.name.clone(),
                        start: members.len(),
                        standing: standing(spec),
                        wounded: members
                            .iter()
                            .filter(|m| m.health <= 0.0 && m.health > death)
                            .count(),
                        dead: members.iter().filter(|m| m.health <= death).count(),
                        state: m.state(s),
                        morale: m.morale(s),
                    }
                })
                .collect(),
            events: m.events().to_vec(),
        }
    }
}

/// Members of a squad still standing.
fn standing(spec: &SquadSpec) -> usize {
    spec.members.iter().filter(|m| m.health > 0.0).count()
}
