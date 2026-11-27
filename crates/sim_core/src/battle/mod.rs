//! Battles between squads on a small field.
//!
//! The war redesign (design note "Econoplex war redesign") makes the squad the
//! unit of war: 10 to 40 people of one type, raised by one town, with one morale
//! and one place on the field. This module fights one battle between squads
//! from physical properties only:
//!
//! - **Kit** gives reach, damage dice and attacks per second, bow range and
//!   arrows, armour class and resistances, a shield and whether the squad is
//!   mounted. Spears outreach swords, a lance hits hard at a gallop, mail
//!   turns blades.
//! - **People** give skill and experience. Experience (battles survived) counts
//!   heavily: in the attack bonus, in hit points and in the squad's discipline.
//! - **The rules** ([`rules`]) settle every blow and arrow: an attack roll of
//!   d20 + bonus against armour class, damage dice, resistance, advantage and
//!   conditions, after the ideas of the open fifth-edition SRD.
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

mod control;
mod kit;
mod lineup;
mod report;
pub mod rts;
pub mod rules;
pub mod soldiers;
pub mod squads;
#[cfg(test)]
mod tests;

pub use control::*;
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
    /// Which model fights a battle that is resolved at once (raids, the
    /// planner's forecasts, `--battle`).
    pub model: BattleModel,
    /// Which model fights a battle the player watches and commands.
    pub live_model: BattleModel,
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
    /// The combat rules (`rules`): proficiency for a fighter of skill 0.5
    /// and no battles, more per unit of skill above that and per ln(1 +
    /// battles survived); hit points unhurt, and more per ln(1 + battles).
    pub proficiency: f32,
    pub skill_bonus: f32,
    pub experience_bonus: f32,
    pub hit_points: f32,
    pub hp_per_experience: f32,
    /// Attack bonus lost per level of exhaustion.
    pub exhaustion_penalty: f32,
    /// Initiative when blows meet (`rules::initiative`): agility per ln(1 +
    /// battles survived), and the bonus of a longer weapon against a man
    /// coming into its reach.
    pub initiative_experience: f32,
    pub reach_initiative: f32,
    /// Battles after which experience gives full discipline.
    pub experience_full: f32,
    /// Metres per second at which a shorter weapon closes inside a longer one's reach.
    pub close_speed: f32,
    /// Extra hit points of damage per metre per second of closing speed in a
    /// charge, for a horse (and a spear braced against one). A running man
    /// hits with `foot_charge` of it.
    pub charge_damage: f32,
    pub foot_charge: f32,
    /// Arrows: the share that come down on the man aimed at (the rest fall
    /// among his neighbours on the soldier model, and are lost on the squad
    /// model), the attack bonus lost at full range, the bonus against a
    /// rider on the squad model (a bigger target), and how much bigger a
    /// rider is where an arrow lands on the soldier model.
    pub arrow_aim: f32,
    pub range_penalty: f32,
    pub rider_target: f32,
    pub mounted_size: f32,
    /// Death (`rules::fate`): a fighter whose damage past 0 hit points
    /// reaches their hit points dies outright; one who is down otherwise
    /// dies after the battle with chance `wound_death`, times `tended` when
    /// their own side holds the field or their captors tend them, and times
    /// `no_quarter` when the winners kill those they catch.
    pub wound_death: f32,
    pub tended: f32,
    pub no_quarter: f32,
    /// Morale lost per share of the squad lost, per second of being hit in the
    /// flank, and regained per second out of the fight.
    pub morale_per_loss: f32,
    pub flank_drain: f32,
    pub morale_regen: f32,
    /// Below `waver_at` a squad fights worse; below `rout_at` it runs.
    pub waver_at: f32,
    pub rout_at: f32,
    /// Walls: armour class a defender on the wall gains against arrows from
    /// below it (cover), the attack bonus their blows have on a man climbing
    /// it or pushing through the breach (the higher ground), and the width
    /// of the gate or breach the attackers must fight through, in metres.
    pub wall_cover: f32,
    pub wall_ground: f32,
    pub breach_width: f32,
    /// Stances and doctrine: archers who skirmish back away from enemies
    /// this close (metres between the squads' edges), and squads of a
    /// cautious side hold their ground until an enemy is this close.
    pub skirmish_distance: f32,
    pub cautious_wait: f32,
    /// Constants only the soldier model uses.
    pub soldiers: soldiers::SoldierParams,
}

impl Default for BattleParams {
    fn default() -> Self {
        Self {
            model: BattleModel::Squads,
            live_model: BattleModel::Soldiers,
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
            proficiency: 2.0,
            skill_bonus: 4.0,
            experience_bonus: 1.0,
            hit_points: 8.0,
            hp_per_experience: 2.0,
            exhaustion_penalty: 1.0,
            initiative_experience: 1.0,
            reach_initiative: 2.0,
            experience_full: 10.0,
            close_speed: 0.5,
            charge_damage: 0.6,
            foot_charge: 0.3,
            arrow_aim: 0.6,
            range_penalty: 8.0,
            rider_target: 2.0,
            mounted_size: 1.5,
            wound_death: 0.25,
            tended: 0.5,
            no_quarter: 2.0,
            morale_per_loss: 1.5,
            flank_drain: 0.01,
            morale_regen: 0.002,
            waver_at: 0.4,
            rout_at: 0.15,
            wall_cover: 2.0,
            wall_ground: 2.0,
            breach_width: 6.0,
            skirmish_distance: 40.0,
            cautious_wait: 120.0,
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
///
/// A squad takes a list of orders and carries them out in turn: the first is
/// in force, and a `MoveTo` gives way to the next once the squad stands
/// there (`Battle::set_orders`). The orders are what an RTS control gives:
/// right-click ground is a `MoveTo` (right-drag sets its facing and width),
/// right-click an enemy is `Attack` (or `Shoot` for archers), shift queues,
/// double-click runs (module `control`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Order {
    Auto,
    /// Stand where it is; spearmen brace.
    Hold,
    /// Close with this squad and fight it.
    Attack(usize),
    /// Shoot this squad while it is in range; close no further.
    Shoot(usize),
    /// Go to `anchor` (metres on the field: the centre of the front rank on
    /// the soldier model, the squad's centre on the squad model), and stand
    /// there facing `facing` (unit; `None`: the way it came). On the soldier
    /// model, `width` sets the front, metres, from now on (`None`: as it
    /// was); the squad model has no front and ignores both. `run`: at a
    /// run (a gallop), not a march, which tires.
    MoveTo {
        anchor: (f32, f32),
        facing: Option<(f32, f32)>,
        width: Option<f32>,
        run: bool,
    },
    /// Leave the field in good order (counts as fled, not routed).
    Withdraw,
}

impl Order {
    /// To `anchor`, the way it comes, its front as it is.
    pub fn move_to(anchor: (f32, f32)) -> Self {
        Self::MoveTo {
            anchor,
            facing: None,
            width: None,
            run: false,
        }
    }
}

/// The order in force in a list of orders: the first, or `Auto`.
fn order_now(orders: &[Order]) -> Order {
    orders.first().copied().unwrap_or(Order::Auto)
}

/// Metres from a `MoveTo`'s anchor at which a squad counts as there.
const ARRIVED: f32 = 1.0;

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
    /// Give a squad its orders, carried out in turn.
    fn set_orders(&mut self, squad: usize, orders: &[Order]);
    /// A squad's orders left, the first in force.
    fn orders(&self, squad: usize) -> &[Order];
    /// Where its front stands.
    fn line(&self, squad: usize) -> SquadLine;
    fn set_stance(&mut self, squad: usize, stance: Stance);
    fn stance(&self, squad: usize) -> Stance;
    /// A side's doctrine; its squads take its `prisoners` as their pursuit.
    fn set_doctrine(&mut self, side: u8, doctrine: Doctrine);
    fn doctrine(&self, side: u8) -> Doctrine;
    /// The seed the battle was set up with.
    fn seed(&self) -> u64;
    /// A squad's marching and running pace now, metres per second.
    fn pace(&self, squad: usize) -> (f32, f32);
    /// Stand a squad with its front at `anchor` before the battle starts.
    fn deploy(&mut self, squad: usize, anchor: (f32, f32), facing: (f32, f32), width: Option<f32>);
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

    /// Give a squad its orders, carried out in turn (between steps: the
    /// player's pause). They replace what it was doing; an empty list gives
    /// it back its own judgment.
    pub fn set_orders(&mut self, squad: usize, orders: &[Order]) {
        if squad < self.model().count() {
            self.model_mut().set_orders(squad, orders);
        }
    }

    /// A squad's stance, from now on.
    pub fn set_stance(&mut self, squad: usize, stance: Stance) {
        if squad < self.model().count() {
            self.model_mut().set_stance(squad, stance);
        }
    }

    pub fn stance(&self, squad: usize) -> Stance {
        self.model().stance(squad)
    }

    /// Side `side`'s doctrine (0 attacks, 1 defends): how its squads go
    /// about it on their own judgment. Every squad of the side takes its
    /// `prisoners` as its pursuit, which a stance may change after.
    pub fn set_doctrine(&mut self, side: u8, doctrine: Doctrine) {
        self.model_mut().set_doctrine(side, doctrine);
    }

    /// Before the first step only: stand squad `squad` with its front's
    /// centre at `anchor`, facing `facing`, `width` wide (`None`: its own),
    /// which must be in its side's zone (`deploy_zone`).
    pub fn deploy(
        &mut self,
        squad: usize,
        anchor: (f32, f32),
        facing: (f32, f32),
        width: Option<f32>,
    ) -> Result<(), String> {
        let side = self.side(squad).ok_or("no such squad")?;
        if self.seconds() > 0.0 {
            return Err("the battle has started".into());
        }
        let (x0, x1, y0, y1) = deploy_zone(self.model().field(), side);
        if !(x0..=x1).contains(&anchor.0) || !(y0..=y1).contains(&anchor.1) {
            return Err(format!(
                "({:.0}, {:.0}) is outside the side's zone",
                anchor.0, anchor.1
            ));
        }
        let l = facing.0.hypot(facing.1);
        if l < 1e-6 {
            return Err("no facing".into());
        }
        let facing = (facing.0 / l, facing.1 / l);
        self.model_mut().deploy(squad, anchor, facing, width);
        Ok(())
    }

    /// What the player tells a squad: orders, a stance or where to deploy.
    pub fn command(&mut self, squad: usize, command: Command) -> Result<(), String> {
        match command {
            Command::Orders(o) => self.set_orders(squad, &o),
            Command::Stance(s) => self.set_stance(squad, s),
            Command::Deploy {
                anchor,
                facing,
                width,
            } => return self.deploy(squad, anchor, facing, width),
        }
        Ok(())
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
        let condition = match self {
            Self::Squads(_) => Vec::new(),
            Self::Soldiers(b) => b.condition(),
        };
        let (sheets, bonus): (Vec<_>, Vec<_>) = (0..m.count())
            .map(|s| rules::sheets(m.params(), m.spec(s), condition.get(s)))
            .unzip();
        LiveView {
            town,
            at,
            facing,
            field: self.field_size(),
            squads: self.replay_squads(),
            orders: (0..m.count()).map(|s| m.orders(s).to_vec()).collect(),
            lines: (0..m.count()).map(|s| m.line(s)).collect(),
            stances: (0..m.count()).map(|s| m.stance(s)).collect(),
            paces: (0..m.count()).map(|s| m.pace(s)).collect(),
            kits: (0..m.count()).map(|s| m.spec(s).kit.clone()).collect(),
            bonus: bonus
                .iter()
                .map(|&b| (b, b - 0.5 * m.params().range_penalty))
                .collect(),
            sheets,
            wall_x: m.field().wall_x,
            morale: (0..m.count()).map(|s| m.morale(s)).collect(),
            condition,
            frame: self.frame(),
            player_side,
            deploying: false,
            finished,
            winner: if finished { self.report().winner } else { None },
        }
    }

    /// Where every squad stands now, and on the soldier model every soldier.
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
            soldiers: match self {
                Self::Squads(_) => Vec::new(),
                Self::Soldiers(b) => b.frame_soldiers(),
            },
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
                    if member.fights() {
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

    /// The side left holding the field, if one is.
    fn winner(&self) -> Option<u8> {
        match sides_fighting(self.model()) {
            [true, false] => Some(0),
            [false, true] => Some(1),
            [false, false] => Some(1), // nobody left: the defenders keep their town
            _ => None,
        }
    }

    /// Who has squad `squad`'s fallen once the battle is over: their own
    /// side, or the winners, as their doctrine takes those they catch.
    pub fn care(&self, squad: usize) -> rules::Care {
        let m = self.model();
        match self.winner() {
            Some(w) if self.finished() && m.spec(squad).side != w => {
                match m.doctrine(w).prisoners {
                    Pursuit::Kill => rules::Care::NoQuarter,
                    Pursuit::Capture => rules::Care::Captors,
                    Pursuit::LetGo => rules::Care::LeftLying,
                }
            }
            _ => rules::Care::Friends,
        }
    }

    /// Whether member `k` of squad `squad` dies of the battle
    /// (`rules::dies`). Before the end, only the killed outright.
    pub fn dies(&self, squad: usize, k: usize) -> bool {
        let m = self.model();
        let Some(member) = m.spec(squad).members.get(k) else {
            return false;
        };
        if !self.finished() {
            return rules::killed_outright(member.health);
        }
        rules::dies(
            m.params(),
            m.seed(),
            squad,
            k,
            member.health,
            self.care(squad),
        )
    }

    pub fn report(&self) -> Report {
        let m = self.model();
        let winner = self.winner();
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
                            .enumerate()
                            .filter(|(k, m)| m.health <= 0.0 && !self.dies(s, *k))
                            .count(),
                        dead: (0..members.len()).filter(|&k| self.dies(s, k)).count(),
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
    spec.members.iter().filter(|m| m.fights()).count()
}
