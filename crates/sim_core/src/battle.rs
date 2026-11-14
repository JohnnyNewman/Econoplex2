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
//! same rules run either way. It is not wired into the economy yet: the caller
//! builds [`SquadSpec`]s and reads the [`Report`] (and updated members) back.

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

/// What a squad's members carry. Later this comes from the items in their
/// inventories; for now the presets stand in for them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Kit {
    pub name: String,
    /// Melee: reach in metres, health taken per hit, strikes per second per
    /// fighter in the front rank, and the share of strikes lost once an enemy
    /// with a shorter weapon is inside the reach.
    pub reach: f32,
    pub damage: f32,
    pub strike_rate: f32,
    pub close_penalty: f32,
    /// Bows: range in metres, shots per second, health per hit, arrows per fighter.
    pub range: f32,
    pub shot_rate: f32,
    pub shot_damage: f32,
    pub arrows: u32,
    /// Share of a hit's damage armour stops, and share of arrows a shield stops.
    pub armour: f32,
    pub shield: f32,
    pub mounted: bool,
}

impl Kit {
    /// Kits for the unit types in the design note. Numbers are physical guesses:
    /// a 3 m spear, a 1.2 m sword, a 150 m bow with 24 arrows, mail stopping half.
    pub fn preset(name: &str) -> Option<Kit> {
        let k = |reach, damage, strike_rate, close_penalty, armour, shield, mounted| Kit {
            name: name.to_string(),
            reach,
            damage,
            strike_rate,
            close_penalty,
            range: 0.0,
            shot_rate: 0.0,
            shot_damage: 0.0,
            arrows: 0,
            armour,
            shield,
            mounted,
        };
        Some(match name {
            "spearmen" => k(3.0, 0.45, 0.30, 0.4, 0.2, 0.4, false),
            "swordsmen" => k(1.2, 0.6, 0.35, 0.0, 0.5, 0.3, false),
            "cavalry" => k(3.0, 0.5, 0.30, 0.3, 0.5, 0.3, true),
            "levy" => k(1.5, 0.3, 0.25, 0.0, 0.0, 0.0, false),
            "heavy" => k(3.0, 0.55, 0.32, 0.1, 0.5, 0.4, false),
            "archers" => Kit {
                range: 150.0,
                shot_rate: 0.1,
                shot_damage: 0.4,
                arrows: 24,
                ..k(0.5, 0.25, 0.3, 0.0, 0.15, 0.0, false)
            },
            "horse_archers" => Kit {
                range: 120.0,
                shot_rate: 0.08,
                shot_damage: 0.35,
                arrows: 20,
                ..k(0.5, 0.25, 0.3, 0.0, 0.2, 0.0, true)
            },
            _ => return None,
        })
    }

    /// The kit a squad fights with, from what its members carry: for each
    /// member, the `fight` values of their items (weapon, armour, shield, mount).
    /// Melee values are averaged over the squad; a member without a weapon
    /// fights with fists. The squad shoots if at least half its members carry a
    /// bow, and fights mounted if at least half ride.
    pub fn from_gear(name: &str, members: &[Vec<&sim_data::FightDef>]) -> Kit {
        let fists = sim_data::FightDef {
            reach: 0.6,
            damage: 0.15,
            strike_rate: 0.25,
            ..Default::default()
        };
        let mut kit = Kit {
            name: name.to_string(),
            reach: 0.0,
            damage: 0.0,
            strike_rate: 0.0,
            close_penalty: 0.0,
            range: 0.0,
            shot_rate: 0.0,
            shot_damage: 0.0,
            arrows: 0,
            armour: 0.0,
            shield: 0.0,
            mounted: false,
        };
        let n = members.len().max(1) as f32;
        let (mut shooters, mut riders) = (0.0f32, 0.0f32);
        let (mut range, mut shot_rate, mut shot_damage, mut arrows) = (0.0, 0.0, 0.0, 0.0);
        for gear in members {
            let weapon = gear
                .iter()
                .copied()
                .find(|f| f.damage > 0.0 || f.range > 0.0)
                .unwrap_or(&fists);
            kit.reach += weapon.reach / n;
            kit.damage += weapon.damage / n;
            kit.strike_rate += weapon.strike_rate / n;
            kit.close_penalty += weapon.close_penalty / n;
            if weapon.range > 0.0 {
                shooters += 1.0;
                range += weapon.range;
                shot_rate += weapon.shot_rate;
                shot_damage += weapon.shot_damage;
                arrows += weapon.arrows as f32;
            }
            // Layers of armour each stop a share of what gets through.
            let through: f32 = gear.iter().map(|f| 1.0 - f.armour).product();
            kit.armour += (1.0 - through) / n;
            kit.shield += gear.iter().map(|f| f.shield).fold(0.0, f32::max) / n;
            if gear.iter().any(|f| f.mounted) {
                riders += 1.0;
            }
        }
        if shooters > 0.0 && shooters >= n / 2.0 {
            kit.range = range / shooters;
            kit.shot_rate = shot_rate / shooters;
            kit.shot_damage = shot_damage / shooters;
            kit.arrows = (arrows / shooters).round() as u32;
        }
        kit.mounted = riders >= n / 2.0 && riders > 0.0;
        kit
    }

    /// The preset names, for help texts.
    pub const PRESETS: [&'static str; 7] = [
        "spearmen",
        "swordsmen",
        "archers",
        "cavalry",
        "levy",
        "heavy",
        "horse_archers",
    ];
}

/// One fighter.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Member {
    /// 1 is unhurt; at or below 0 the fighter is out of the battle.
    pub health: f32,
    /// Skill with the squad's weapon (the economy's skill level, roughly 0..1).
    pub skill: f32,
    /// Battles survived.
    pub experience: f32,
}

/// A squad as it enters the battle.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SquadSpec {
    pub name: String,
    /// 0 attacks, 1 defends.
    pub side: u8,
    pub kit: Kit,
    pub members: Vec<Member>,
    /// How drilled the squad is, 0..1 (adds to discipline with experience).
    pub drill: f32,
    /// Starting morale 0..1, from the members' needs, pay owed and cohesion.
    pub morale: f32,
    /// Position on the field, metres.
    pub pos: (f32, f32),
    /// Stands behind the town's wall (defenders only).
    pub behind_wall: bool,
}

impl SquadSpec {
    /// A squad of `n` fighters of a preset kit with even skill and experience.
    #[allow(clippy::too_many_arguments)]
    pub fn uniform(
        name: &str,
        side: u8,
        kit: Kit,
        n: usize,
        skill: f32,
        experience: f32,
        morale: f32,
        pos: (f32, f32),
    ) -> Self {
        Self {
            name: name.to_string(),
            side,
            kit,
            members: vec![
                Member {
                    health: 1.0,
                    skill,
                    experience,
                };
                n
            ],
            drill: 0.5,
            morale,
            pos,
            behind_wall: false,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Contact {
        by: usize,
        with: usize,
    },
    Charge {
        by: usize,
        into: usize,
        braced: bool,
    },
    Wavering(usize),
    Routing(usize),
    Fled(usize),
    Destroyed(usize),
    OutOfArrows(usize),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub t: f32,
    pub kind: EventKind,
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

/// One squad's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SquadReport {
    pub name: String,
    pub side: u8,
    pub kit: String,
    pub start: usize,
    pub standing: usize,
    pub wounded: usize,
    pub dead: usize,
    pub state: SquadState,
    pub morale: f32,
}

/// What happened: who won, when, and each squad's losses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The side left holding the field, or `None` if time ran out with both standing.
    pub winner: Option<u8>,
    pub seconds: f32,
    pub squads: Vec<SquadReport>,
    pub events: Vec<Event>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let who = match self.winner {
            Some(0) => "attackers win",
            Some(_) => "defenders win",
            None => "no decision",
        };
        writeln!(f, "{who} after {:.0} min", self.seconds / 60.0)?;
        for s in &self.squads {
            writeln!(
                f,
                "  {:<5} {:<14} {:<13} {:>3} in, {:>3} standing, {:>3} wounded, {:>3} dead, {:?}, morale {:.2}",
                if s.side == 0 { "att" } else { "def" },
                s.name,
                s.kit,
                s.start,
                s.standing,
                s.wounded,
                s.dead,
                s.state,
                s.morale
            )?;
        }
        Ok(())
    }
}

/// One squad in a battle replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplaySquad {
    pub name: String,
    pub side: u8,
    pub mounted: bool,
    /// Fighters at the start.
    pub start: u16,
}

/// Where every squad stood at one moment of a battle: (x, y) in metres on the
/// field, fighters still standing, and its state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayFrame {
    pub t: f32,
    pub squads: Vec<(f32, f32, u16, SquadState)>,
}

/// A fought battle, for the 3D view to play back where it happened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Replay {
    /// The town fought over, and the map position of the field's centre.
    pub town: u16,
    pub at: (f32, f32),
    /// Map direction from the attackers' side of the field to the defenders'
    /// (the field's +x), unit length.
    pub facing: (f32, f32),
    /// Field size, metres.
    pub field: (f32, f32),
    pub squads: Vec<ReplaySquad>,
    pub frames: Vec<ReplayFrame>,
    pub winner: Option<u8>,
}

/// A battle being fought live, as the game shows it and the player commands it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveView {
    pub town: u16,
    pub at: (f32, f32),
    pub facing: (f32, f32),
    pub field: (f32, f32),
    pub squads: Vec<ReplaySquad>,
    pub orders: Vec<Order>,
    pub morale: Vec<f32>,
    pub frame: ReplayFrame,
    /// The side the player fights on (0 attacks, 1 defends).
    pub player_side: u8,
    pub finished: bool,
    pub winner: Option<u8>,
}

/// Battles fought, for the view (the newest few; `total` counts all of them so
/// a reader can tell which it has seen).
#[derive(bevy_ecs::prelude::Resource, Clone, Debug, Default)]
pub struct BattleLog {
    pub replays: std::collections::VecDeque<Replay>,
    pub total: usize,
}

impl BattleLog {
    /// Battles kept for a view that falls behind.
    pub const KEEP: usize = 16;

    pub fn push(&mut self, replay: Replay) {
        self.replays.push_back(replay);
        self.total += 1;
        while self.replays.len() > Self::KEEP {
            self.replays.pop_front();
        }
    }

    /// Replays after the first `seen` battles ever fought.
    pub fn since(&self, seen: usize) -> Vec<Replay> {
        let first = self.total - self.replays.len();
        self.replays
            .iter()
            .skip(seen.saturating_sub(first))
            .cloned()
            .collect()
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

        // 1. Move, by order or by the squad's own judgment.
        for i in 0..n {
            self.move_squad(i);
        }

        // 2. Contacts, and charges where a contact is new.
        let contacts: Vec<Vec<usize>> = (0..n)
            .map(|i| {
                if !self.squads[i].on_field() {
                    return Vec::new();
                }
                self.enemies(i)
                    .filter(|&j| self.squads[j].on_field() && self.gap(i, j) <= 1.0)
                    .collect()
            })
            .collect();
        for i in 0..n {
            for &j in &contacts[i] {
                if self.squads[i].contacts.contains(&j) {
                    continue;
                }
                // New contact: the faster one charges, the longer weapon gets free blows.
                if i < j {
                    self.event(EventKind::Contact { by: i, with: j });
                    let (a, b) = if self.squads[i].speed >= self.squads[j].speed {
                        (i, j)
                    } else {
                        (j, i)
                    };
                    self.charge(a, b);
                }
                let (ri, rj) = (self.squads[i].spec.kit.reach, self.squads[j].spec.kit.reach);
                let t = (ri - rj).abs() / self.params.close_speed.max(1e-3);
                self.squads[i].closing.push((j, t));
            }
        }
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

        // 3. Arrows from squads not locked in melee.
        for i in 0..n {
            self.shoot(i);
        }

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
            if self.squads[p].primary != Some(i) && self.squads[p].fighting() {
                flanked[p] = true;
            }
            self.melee(i, p);
        }

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

    /// Effective skill and discipline from the members still standing.
    fn refresh(&mut self) {
        let p = &self.params;
        for s in &mut self.squads {
            let (mut sum, mut exp, mut n) = (0.0, 0.0, 0.0);
            for m in s.spec.members.iter().filter(|m| m.health > 0.0) {
                let e = m.experience.max(0.0);
                sum += m.skill + p.experience_weight * (1.0 + e).ln();
                exp += e;
                n += 1.0;
            }
            if n > 0.0 {
                s.eff = sum / n;
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
        let (walk, run) = if mounted {
            (p.horse_walk_speed, p.horse_charge_speed)
        } else {
            (p.walk_speed, p.run_speed)
        };

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
        // Locked in melee: no moving.
        if !s.contacts.is_empty() {
            return;
        }
        let goal: Option<(f32, f32, Option<usize>)> = match s.order {
            Order::Hold => None,
            Order::MoveTo(x, y) => Some((x, y, None)),
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
        let near = target.is_some_and(|j| self.gap(i, j) < p.charge_distance);
        self.step_toward(i, (x, y), if near { run } else { walk }, target);
    }

    /// A squad's own judgment: where to go, and whom to close with.
    fn auto_goal(&self, i: usize) -> Option<(f32, f32, Option<usize>)> {
        let s = &self.squads[i];
        if s.spec.behind_wall {
            return None;
        }
        let kit = &s.spec.kit;
        let fighting = |q: &Squad| q.fighting();
        let target = if kit.mounted {
            // Riders look for archers and fleeing squads, and steer clear of a
            // spear wall while softer targets are about.
            self.enemies(i)
                .filter(|&j| self.squads[j].on_field())
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
        let dt = self.params.dt;
        let from = self.squads[i].spec.pos;
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let d = (dx * dx + dy * dy).sqrt();
        let mut step = speed * dt;
        if let Some(j) = target {
            // Stop at the enemy's edge.
            step = step.min((self.gap(i, j) - 0.5).max(0.0));
        } else {
            step = step.min(d);
        }
        if d < 1e-3 || step <= 0.0 {
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
        // A squad that moved at all hits at the pace it was moving.
        self.squads[i].speed = if moved > 1e-3 { speed } else { 0.0 };
    }

    /// True if the wall stands between two squads.
    fn across_wall(&self, a: usize, b: usize) -> bool {
        self.field.wall_x.is_some()
            && self.squads[a].spec.behind_wall != self.squads[b].spec.behind_wall
    }

    /// A charge on first contact. A braced line with the longer reach turns it
    /// onto its own points; otherwise the charging front strikes with its speed.
    fn charge(&mut self, a: usize, b: usize) {
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
    fn melee(&mut self, i: usize, t: usize) {
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
    fn shoot(&mut self, i: usize) {
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

/// Parse a battle line-up like `cavalry*20 vs spearmen*30` or
/// `swordsmen*50,archers*20 vs walls spearmen*30+5!0.6`: each squad is
/// `kit*count`, optionally `+battles` of experience, `@skill` and `!morale`.
/// `walls` before the defenders puts them behind a wall.
pub fn parse_lineup(text: &str) -> Result<(Field, Vec<SquadSpec>), String> {
    let (att, def) = text
        .split_once(" vs ")
        .ok_or("a line-up needs `ATTACKERS vs DEFENDERS`")?;
    let mut def = def.trim();
    let mut field = Field::default();
    let walls = def.starts_with("walls ");
    if walls {
        def = def["walls ".len()..].trim();
        field.wall_x = Some(field.width * 0.6);
    }
    let mut specs = Vec::new();
    for (side, list) in [(0u8, att), (1u8, def)] {
        let parts: Vec<&str> = list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let k = parts.len().max(1) as f32;
        for (n, part) in parts.iter().enumerate() {
            let mut rest = part.to_string();
            let num = |v: Option<String>, d: f32, what: &str| -> Result<f32, String> {
                v.map_or(Ok(d), |v| {
                    v.parse().map_err(|_| format!("bad {what} in `{part}`"))
                })
            };
            let morale = num(take(&mut rest, '!'), 0.8, "morale")?;
            let skill = num(take(&mut rest, '@'), 0.5, "skill")?;
            let exp = num(take(&mut rest, '+'), 0.0, "experience")?;
            let count = num(take(&mut rest, '*'), 20.0, "count")? as usize;
            let kit_name = rest.trim();
            let kit = Kit::preset(kit_name).ok_or_else(|| {
                format!(
                    "unknown kit `{kit_name}` (one of {})",
                    Kit::PRESETS.join(", ")
                )
            })?;
            let y = field.height * (n as f32 + 0.5) / k;
            let spacing = if kit.mounted { 2.0 } else { 1.0 };
            let x = match (side, walls) {
                (0, _) => field.width * 0.15,
                // Right behind the wall.
                (_, true) => {
                    field.wall_x.unwrap_or(0.0) + (count as f32).sqrt() * spacing * 0.5 + 0.6
                }
                _ => field.width * 0.75,
            };
            let level = if kit_name == "levy" {
                skill.min(0.2)
            } else {
                skill
            };
            let mut spec = SquadSpec::uniform(
                &format!("{kit_name}{}", n + 1),
                side,
                kit,
                count,
                level,
                exp,
                morale,
                (x, y),
            );
            spec.behind_wall = side == 1 && walls;
            specs.push(spec);
        }
    }
    Ok((field, specs))
}

/// Cut `<c>value` out of a squad description and return the value.
fn take(rest: &mut String, c: char) -> Option<String> {
    let at = rest.find(c)?;
    let end = rest[at + 1..]
        .find(['*', '+', '@', '!'])
        .map_or(rest.len(), |e| at + 1 + e);
    let v = rest[at + 1..end].to_string();
    rest.replace_range(at..end, "");
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wins(lineup: &str, seeds: u64) -> (u64, u64) {
        let mut att = 0;
        let mut def = 0;
        for seed in 0..seeds {
            let (field, specs) = parse_lineup(lineup).unwrap();
            let r = Battle::new(BattleParams::default(), field, specs, seed).run();
            match r.winner {
                Some(0) => att += 1,
                Some(_) => def += 1,
                None => {}
            }
        }
        (att, def)
    }

    fn losses(lineup: &str, seeds: u64) -> (usize, usize) {
        let (mut a, mut d) = (0, 0);
        for seed in 0..seeds {
            let (field, specs) = parse_lineup(lineup).unwrap();
            let r = Battle::new(BattleParams::default(), field, specs, seed).run();
            for s in &r.squads {
                let lost = s.wounded + s.dead;
                if s.side == 0 {
                    a += lost
                } else {
                    d += lost
                }
            }
        }
        (a, d)
    }

    #[test]
    fn same_seed_same_battle() {
        let run = |seed| {
            let (field, specs) = parse_lineup("swordsmen*30,archers*20 vs spearmen*30").unwrap();
            Battle::new(BattleParams::default(), field, specs, seed).run()
        };
        assert_eq!(run(7), run(7));
    }

    #[test]
    fn spear_wall_stops_a_charge() {
        let (att, def) = wins("cavalry*20 vs spearmen*30", 20);
        assert!(
            def >= 14,
            "spearmen won {def} of 20 against cavalry (cavalry {att})"
        );
    }

    #[test]
    fn cavalry_rides_down_archers() {
        let (att, _) = wins("cavalry*20 vs archers*30", 20);
        assert!(att >= 16, "cavalry won {att} of 20 against archers");
    }

    #[test]
    fn arrows_break_a_levy() {
        let (_, def) = wins("levy*40 vs archers*30", 20);
        assert!(def >= 14, "archers won {def} of 20 against a levy");
    }

    #[test]
    fn swords_get_inside_spears() {
        let (att, _) = wins("swordsmen*30 vs spearmen*30", 20);
        assert!(att >= 14, "swordsmen won {att} of 20 against spearmen");
    }

    #[test]
    fn veterans_beat_more_recruits() {
        let (att, _) = wins("swordsmen*25+20 vs swordsmen*30", 20);
        assert!(att >= 14, "25 veterans won {att} of 20 against 30 recruits");
    }

    #[test]
    fn walls_hold_the_breach() {
        let (open_a, _) = wins("swordsmen*45 vs spearmen*30", 20);
        let (wall_a, _) = wins("swordsmen*45 vs walls spearmen*30", 20);
        assert!(
            wall_a + 6 <= open_a,
            "attackers won {open_a} of 20 in the open but {wall_a} against walls"
        );
        let (a, d) = losses("swordsmen*45 vs walls spearmen*30", 10);
        assert!(a > d, "attackers lost {a}, defenders on the wall {d}");
    }

    #[test]
    fn hungry_squads_break_first() {
        let (fed, _) = wins("swordsmen*30!0.9 vs swordsmen*30!0.9", 20);
        let (hungry, _) = wins("swordsmen*30!0.3 vs swordsmen*30!0.9", 20);
        assert!(
            hungry + 5 <= fed,
            "fed won {fed}, hungry won {hungry} of 20"
        );
    }

    #[test]
    fn routed_squads_leave_the_field() {
        let (field, specs) = parse_lineup("levy*20!0.3 vs swordsmen*30+5").unwrap();
        let r = Battle::new(BattleParams::default(), field, specs, 1).run();
        assert_eq!(r.winner, Some(1));
        assert!(
            matches!(r.squads[0].state, SquadState::Fled | SquadState::Destroyed),
            "{r}"
        );
        assert!(r
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::Routing(0))));
    }

    #[test]
    fn orders_change_the_battle() {
        // Archers ordered to hold and shoot never close; ordered to attack, they do.
        let (field, specs) = parse_lineup("archers*20 vs spearmen*20").unwrap();
        let mut b = Battle::new(BattleParams::default(), field, specs, 3);
        b.set_order(1, Order::Hold);
        b.set_order(0, Order::Shoot(1));
        for _ in 0..120 {
            b.step();
        }
        assert!(b.gap(0, 1) > 50.0, "archers closed in despite their order");
    }

    #[test]
    fn content_kit_matches_the_presets() {
        let paths = crate::AssetPaths::discover();
        let (db, _, _) = crate::load(&paths).unwrap();
        let fight = |id: &str| {
            let p = db.content.product(id).expect(id);
            db.content.products[p as usize].fight.clone().expect(id)
        };
        let (spear, shield, coat) = (fight("iron_spear"), fight("shield"), fight("padded_coat"));
        let squad: Vec<Vec<&sim_data::FightDef>> =
            (0..20).map(|_| vec![&spear, &shield, &coat]).collect();
        let kit = Kit::from_gear("spearmen", &squad);
        let preset = Kit::preset("spearmen").unwrap();
        assert!((kit.reach - preset.reach).abs() < 1e-4);
        assert!((kit.armour - preset.armour).abs() < 1e-6);
        assert!((kit.shield - preset.shield).abs() < 1e-6);
        assert!(!kit.mounted && kit.range == 0.0);

        let (lance, mail, horse, bow) =
            (fight("lance"), fight("mail"), fight("horse"), fight("bow"));
        let riders: Vec<Vec<&sim_data::FightDef>> =
            (0..10).map(|_| vec![&lance, &mail, &horse]).collect();
        let kit = Kit::from_gear("cavalry", &riders);
        assert!(kit.mounted && (kit.armour - 0.5).abs() < 1e-6);

        // Half the squad unarmed: fists pull the average down; bows still count.
        let mut mixed: Vec<Vec<&sim_data::FightDef>> = (0..5).map(|_| vec![&bow]).collect();
        mixed.extend((0..5).map(|_| Vec::new()));
        let kit = Kit::from_gear("archers", &mixed);
        assert_eq!(kit.range, 150.0);
        assert!(kit.damage < bow.damage);

        // Every weapon in the content can fight.
        for p in &db.content.products {
            if p.category == sim_data::Category::Weapon {
                assert!(p.fight.is_some(), "{} has no fight values", p.id);
            }
        }
    }

    #[test]
    fn a_recorded_battle_ends_where_the_report_says() {
        let (field, specs) = parse_lineup("cavalry*20 vs archers*30").unwrap();
        let mut b = Battle::new(BattleParams::default(), field, specs, 5);
        let (report, squads, frames) = b.run_recorded(5.0);
        assert_eq!(squads.len(), 2);
        assert!(frames.len() > 2);
        let last = frames.last().unwrap();
        for (k, s) in report.squads.iter().enumerate() {
            assert_eq!(last.squads[k].2 as usize, s.standing);
            assert_eq!(last.squads[k].3, s.state);
        }
        let mut log = BattleLog::default();
        for _ in 0..BattleLog::KEEP + 3 {
            log.push(Replay {
                town: 0,
                at: (0.0, 0.0),
                facing: (1.0, 0.0),
                field: (600.0, 400.0),
                squads: squads.clone(),
                frames: Vec::new(),
                winner: report.winner,
            });
        }
        assert_eq!(log.replays.len(), BattleLog::KEEP);
        assert_eq!(log.since(log.total - 2).len(), 2);
        assert_eq!(log.since(0).len(), BattleLog::KEEP);
    }

    #[test]
    fn survivors_gain_experience() {
        let (field, specs) = parse_lineup("swordsmen*30 vs levy*20").unwrap();
        let mut b = Battle::new(BattleParams::default(), field, specs, 2);
        b.run();
        assert!(b
            .members(0)
            .iter()
            .any(|m| m.health > 0.0 && m.experience >= 1.0));
    }
}
