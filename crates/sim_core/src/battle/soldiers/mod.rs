//! The soldier model: every soldier simulated (design note
//! `docs/design/battle-per-soldier.md`).
//!
//! Squads stay the unit the player and the AI command; only the battle inside
//! them changes. Every soldier has a place, a facing, a morale, a fatigue and
//! their own kit, so a flank attack, a broken line, a tired army and a wedge
//! come out of positions rather than out of flags.
//!
//! Soldiers are plain arrays, one entry per soldier and one array per field.
//! [`SoldierBattle::step`] advances one battle second (`params.dt`) in
//! substeps of `soldiers.substep` seconds. One substep runs in six phases:
//!
//! 1. **Grid.** Rebuild the spatial grid, one per side ([`grid`]).
//! 2. **Squad controllers.** Each squad sets its formation, anchor and facing
//!    from its order, and gives every soldier their slot ([`squad`]).
//! 3. **Soldier controllers.** Each soldier picks where to step and whom to
//!    strike or shoot ([`soldier`]).
//! 4. **Blows and shots.** Each strike rolls to hit on the striker's own
//!    random stream and is written into the striker's buffer.
//! 5. **Apply.** In soldier order: damage, movement, morale, fatigue and state.
//! 6. **Squads.** Recount each squad and tell the [`Report`] what happened.
//!
//! Phases 3 and 4 only read the last substep's state and write per-soldier
//! buffers, so one pass does both, soldier by soldier. It runs on rayon for
//! big battles and gives the same battle as one thread: every draw comes from
//! a counter-based stream keyed by seed, substep, soldier and purpose ([`rng`]).

mod apply;
mod grid;
mod rng;
mod soldier;
mod squad;
#[cfg(test)]
mod tests;
mod wall;

use super::*;
use grid::Grid;
use rayon::prelude::*;
use rng::{Draw, Purpose};
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};
use std::time::Instant;

/// Constants of the soldier model (`battle.soldiers` in `models.ron`). Every
/// rate is per battle second and is scaled by the substep.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SoldierParams {
    /// Battle seconds per substep.
    pub substep: f32,
    /// Side of a grid cell, metres.
    pub cell: f32,
    /// Radius of a man and of a horse, metres: nobody stands closer.
    pub body: f32,
    pub horse_body: f32,
    /// Kilograms: a man, and a horse under him. A charge hits with mass.
    pub man_kg: f32,
    pub horse_kg: f32,
    /// Fatigue gained per second running (times 1 + kit weight / `carry_kg`,
    /// and `horse_tire` of it on horseback), fighting and marching, and lost
    /// per second standing out of contact.
    pub fatigue_run: f32,
    pub fatigue_fight: f32,
    pub fatigue_walk: f32,
    pub fatigue_rest: f32,
    pub carry_kg: f32,
    pub horse_tire: f32,
    /// Top speed, strike rate and parry are multiplied by
    /// 1 - `fatigue_effect` * fatigue.
    pub fatigue_effect: f32,
    /// Morale lost per second at full fatigue (times fatigue squared).
    pub fatigue_morale: f32,
    /// Morale lost per wound, and per blow from the side or the rear.
    pub hit_morale: f32,
    pub flank_drain: f32,
    /// A blow from behind does this many times the damage.
    pub rear_damage: f32,
    /// Morale lost when a friend within `near_radius` metres falls, or breaks.
    pub near_fall: f32,
    pub near_rout: f32,
    pub near_radius: f32,
    /// Morale gained when a foe within `near_radius` metres breaks.
    pub foe_breaks: f32,
    /// Seconds without a blow, a wound or a fallen friend before nerve comes
    /// back (`battle.morale_regen` per second), and the extra per second within
    /// `leader_radius` metres of the squad's most experienced member.
    pub quiet_after: f32,
    pub leader_regen: f32,
    pub leader_radius: f32,
    /// Radians per second a squad's facing turns.
    pub turn_rate: f32,
    /// Radians per second a soldier turns: a man struck from a side he
    /// has not turned to yet takes the blow as a flank blow.
    pub soldier_turn: f32,
    /// Metres a squad's anchor may run ahead of its front rank.
    pub lead: f32,
    /// A soldier fights an enemy within this many metres of their slot, and
    /// strays no further from it while fighting.
    pub engage_slack: f32,
    /// Files of a marching column, and how far away a goal must be, metres,
    /// for a squad to march to it in column.
    pub column_files: u16,
    pub column_beyond: f32,
    /// Metres between archers in loose order.
    pub loose_spacing: f32,
    /// Spearmen brace when riders come within this many metres.
    pub brace_riders: f32,
    /// Riders who ran onto a spear wall break off after this many seconds in
    /// contact, ride back `rally_distance` metres and charge again.
    pub rally_after: f32,
    pub rally_distance: f32,
    /// An arrow hits a man whose centre is within this many metres of where
    /// it lands (a rider: times `battle.mounted_size`).
    pub hit_radius: f32,
    /// A soldier pressed by this many bodies at once is in a crush:
    /// restrained (blows on them have advantage, theirs disadvantage).
    pub crush_bodies: u8,
    /// Whether arrows can hit friends where they land.
    pub friendly_fire: bool,
    /// Phases 3 and 4 run on all threads from this many soldiers on.
    pub parallel_from: usize,
}

impl Default for SoldierParams {
    fn default() -> Self {
        Self {
            substep: 0.25,
            cell: 4.0,
            body: 0.4,
            horse_body: 0.9,
            man_kg: 80.0,
            horse_kg: 520.0,
            fatigue_run: 0.004,
            fatigue_fight: 0.003,
            fatigue_walk: 0.0005,
            fatigue_rest: 0.002,
            carry_kg: 40.0,
            horse_tire: 0.5,
            fatigue_effect: 0.5,
            fatigue_morale: 0.002,
            hit_morale: 0.02,
            flank_drain: 0.06,
            rear_damage: 1.5,
            near_fall: 0.01,
            near_rout: 0.02,
            foe_breaks: 0.02,
            near_radius: 5.0,
            quiet_after: 10.0,
            leader_regen: 0.002,
            leader_radius: 15.0,
            turn_rate: 0.5,
            soldier_turn: 3.0,
            lead: 1.5,
            engage_slack: 2.0,
            column_files: 4,
            column_beyond: 150.0,
            loose_spacing: 3.0,
            brace_riders: 200.0,
            rally_after: 4.0,
            rally_distance: 30.0,
            hit_radius: 0.5,
            crush_bodies: 4,
            friendly_fire: true,
            parallel_from: 2048,
        }
    }
}

/// A point or a direction on the field, metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V2 {
    pub x: f32,
    pub y: f32,
}

impl V2 {
    pub const ZERO: V2 = V2 { x: 0.0, y: 0.0 };
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    #[inline]
    pub fn dot(self, o: V2) -> f32 {
        self.x * o.x + self.y * o.y
    }
    /// The z of the cross product: positive when `o` is to the left.
    #[inline]
    pub fn cross(self, o: V2) -> f32 {
        self.x * o.y - self.y * o.x
    }
    #[inline]
    pub fn len2(self) -> f32 {
        self.dot(self)
    }
    #[inline]
    pub fn len(self) -> f32 {
        self.len2().sqrt()
    }
    /// Unit length, or `or` when too short to tell a direction.
    #[inline]
    pub fn norm_or(self, or: V2) -> V2 {
        let l = self.len();
        if l > 1e-6 {
            self * (1.0 / l)
        } else {
            or
        }
    }
    /// At most `max` long.
    #[inline]
    pub fn clamp_len(self, max: f32) -> V2 {
        let l2 = self.len2();
        if l2 > max * max {
            self * (max / l2.sqrt())
        } else {
            self
        }
    }
    /// Turned a quarter to the right of a facing.
    #[inline]
    pub fn right(self) -> V2 {
        V2::new(self.y, -self.x)
    }
    /// This unit vector turned toward unit vector `to` by at most the angle
    /// whose cosine and sine are `c` and `s`.
    #[inline]
    pub fn turned(self, to: V2, c: f32, s: f32) -> V2 {
        if self.dot(to) >= c {
            return to;
        }
        let s = if self.cross(to) >= 0.0 { s } else { -s };
        V2::new(self.x * c - self.y * s, self.x * s + self.y * c).norm_or(to)
    }
}

impl Add for V2 {
    type Output = V2;
    #[inline]
    fn add(self, o: V2) -> V2 {
        V2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for V2 {
    type Output = V2;
    #[inline]
    fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for V2 {
    type Output = V2;
    #[inline]
    fn mul(self, k: f32) -> V2 {
        V2::new(self.x * k, self.y * k)
    }
}
impl Neg for V2 {
    type Output = V2;
    #[inline]
    fn neg(self) -> V2 {
        V2::new(-self.x, -self.y)
    }
}
impl AddAssign for V2 {
    #[inline]
    fn add_assign(&mut self, o: V2) {
        *self = *self + o;
    }
}
impl SubAssign for V2 {
    #[inline]
    fn sub_assign(&mut self, o: V2) {
        *self = *self - o;
    }
}

/// What a soldier is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SoldierState {
    /// In their place in the squad.
    Formed,
    /// An enemy is within reach or near their slot.
    Engaged,
    /// Formed or fighting, at the morale factor.
    Wavering,
    /// Running for their own edge of the field.
    Routing,
    /// Lying where they fell.
    Down,
    /// Left the field.
    Fled,
}

impl SoldierState {
    /// Formed, engaged or wavering: in their place in the squad.
    #[inline]
    fn in_line(self) -> bool {
        matches!(self, Self::Formed | Self::Engaged | Self::Wavering)
    }
    /// Still on the field and on their feet.
    #[inline]
    fn present(self) -> bool {
        !matches!(self, Self::Down | Self::Fled)
    }
}

/// How a squad stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Formation {
    /// `frontage` wide (or as wide as the player set), as many ranks as it takes.
    Line,
    /// `column_files` wide, for marching to a goal far away.
    Column,
    /// A triangle, point at the front: riders charging.
    Wedge,
    /// A line whose front faces the threat and stands to take a charge.
    Braced,
    /// A line at `loose_spacing`: archers, or a squad told to stand loose.
    Loose,
}

/// No soldier, no target.
const NONE: u32 = u32::MAX;

/// Half the thickness of a town wall, metres.
const WALL_HALF: f32 = 0.5;

/// One kit in the battle's table: kits that are the same are stored once.
struct KitEntry {
    kit: Kit,
    /// Kilograms that hit in a charge: man, gear and horse.
    mass: f32,
    /// Radius, metres.
    body: f32,
    /// Top speed and marching pace, metres per second.
    top: f32,
    walk: f32,
}

/// A squad's controller and its tally.
struct SquadCtl {
    spec: SquadSpec,
    state: SquadState,
    /// Orders left, the first in force.
    orders: Vec<Order>,
    stance: Stance,
    morale: f32,
    /// Its soldiers are `first..first + spec.members.len()`.
    first: usize,
    formation: Formation,
    /// The front the player set (`MoveTo` width), metres; `None`: its own.
    width: Option<f32>,
    /// The width changed: re-form, even in the same formation.
    reform_due: bool,
    /// Files it narrowed to, to pass between obstacles (`None`: as wide as
    /// it would be).
    squeeze: Option<u16>,
    /// Its way round the obstacles to where it goes.
    route: Route,
    /// The piece of the town's wall it batters (an index in the ring's
    /// pieces), while the wall is closed.
    assault: Option<usize>,
    /// It stands at the wall (battering, manning or plugging it): it keeps
    /// its width there, however little room lies ahead.
    at_wall: bool,
    /// The centre of the front rank, and where the front faces (unit).
    anchor: V2,
    facing: V2,
    /// Files of a line-like formation, and metres between soldiers.
    files: u16,
    spacing: f32,
    /// The enemy squad it closes with, and the one it shoots.
    target: Option<usize>,
    shoot_at: Option<usize>,
    /// Riders breaking off to charge again: seconds in contact, and the
    /// point they ride back to.
    contact_time: f32,
    rally: Option<V2>,
    /// 1 - half the discipline: how much each loss shakes it.
    steady: f32,
    /// Its most experienced member, while they stand.
    leader: Option<usize>,
    /// Arrows left, and whether running out was told.
    arrows: u32,
    told_arrows: bool,
    /// Tally of the last substep.
    centre: V2,
    front: V2,
    in_line: usize,
    routing: usize,
    in_contact: usize,
    fighting_with: Option<usize>,
    /// Mean fatigue factor of those in line.
    vigour: f32,
    /// Soldiers who fell this substep, and who left the line (fell, broke
    /// or fled).
    losses: u32,
    left: u32,
    /// Struck, shot or hit at all (for experience).
    engaged: bool,
    wavering: bool,
}

impl SquadCtl {
    /// The order in force.
    fn order(&self) -> Order {
        order_now(&self.orders)
    }
    /// An order comes into force: a `MoveTo` with a width sets the front.
    fn start_order(&mut self) {
        if let Order::MoveTo { width: Some(w), .. } = self.order() {
            if self.width != Some(w) {
                self.width = Some(w);
                self.reform_due = true;
            }
        }
    }
    fn fighting(&self) -> bool {
        self.state == SquadState::Fighting
    }
    fn on_field(&self) -> bool {
        matches!(self.state, SquadState::Fighting | SquadState::Routing)
    }
    fn has_bow(&self) -> bool {
        self.spec.kit.range > 0.0 && self.arrows > 0
    }
    fn mounted(&self) -> bool {
        self.spec.kit.mounted
    }
    /// Whether it strikes enemies who run.
    fn pursues(&self) -> bool {
        self.stance.pursuit == Pursuit::Kill
    }
    /// Whether it goes after enemies who run (to cut them down or take them).
    fn chases(&self) -> bool {
        self.stance.pursuit != Pursuit::LetGo
    }
    /// A disc for distances between squads, as the squad model has it.
    fn radius(&self) -> f32 {
        ((self.in_line + self.routing) as f32).sqrt() * self.spacing * 0.5 + 0.5
    }
    /// Where slot (file, rank) stands now.
    #[inline]
    fn slot(&self, file: i16, rank: u16) -> V2 {
        let lat = match self.formation {
            Formation::Wedge => file as f32,
            _ => file as f32 - (self.files as f32 - 1.0) * 0.5,
        } * self.spacing;
        let back = rank as f32 * self.spacing;
        self.anchor + self.facing.right() * lat - self.facing * back
    }
}

/// What a soldier means to do this substep (phase 3).
#[derive(Clone, Copy, Debug)]
struct Plan {
    /// Where they step, metres per second, and how far bodies around push
    /// them, metres.
    vel: V2,
    push: V2,
    facing: V2,
    target: u32,
    act: Act,
    /// Whom an archer aims at.
    aim: u32,
    /// Morale gained or lost from what is around them.
    morale: f32,
    /// Bodies pressing on them.
    crush: u8,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            vel: V2::ZERO,
            push: V2::ZERO,
            facing: V2::new(1.0, 0.0),
            target: NONE,
            act: Act::None,
            aim: NONE,
            morale: 0.0,
            crush: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    None,
    Strike,
    /// The first blow of a run into the enemy.
    Charge,
    Shoot,
    /// Laying hands on a runner, to take them prisoner.
    Capture,
}

/// What a soldier's blow or shot did (phase 4).
#[derive(Clone, Copy, Debug)]
struct Blow {
    /// Who is hit (or missed), or `NONE`.
    target: u32,
    damage: f32,
    hit: bool,
    /// The blow came from the target's side (1) or rear (2).
    angle: u8,
    /// A braced spear's blow on this soldier as they charged, and whose.
    counter: f32,
    counter_by: u32,
    /// Not a blow: the target, running, is taken prisoner.
    capture: bool,
    /// Its initiative: blows of the same substep land highest first
    /// (`rules::initiative`).
    init: f32,
}

impl Default for Blow {
    fn default() -> Self {
        Self {
            target: NONE,
            damage: 0.0,
            hit: false,
            angle: 0,
            counter: 0.0,
            counter_by: NONE,
            capture: false,
            init: rules::FIRST,
        }
    }
}

/// The blows that landed on one soldier in a substep.
#[derive(Clone, Copy, Debug, Default)]
struct Hurt {
    damage: f32,
    wounds: u16,
    /// Blows, hit or not, from their side or rear.
    side_blows: u16,
}

/// Seconds spent per phase, for `--profile`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Profile {
    pub substeps: u64,
    pub phases: [f64; 5],
}

impl Profile {
    pub const PHASES: [&'static str; 5] = [
        "grid",
        "squad controllers",
        "soldiers and blows",
        "apply",
        "squads",
    ];
}

/// A battle in which every soldier is simulated.
pub struct SoldierBattle {
    params: BattleParams,
    field: Field,
    /// The field's obstacles (`None`: an open field).
    nav: Option<Arc<Nav>>,
    /// The town's wall, if the field has one.
    ring: Option<crate::battle::walls::Ring>,
    seed: u64,
    t: f32,
    substep: u64,
    events: Vec<Event>,
    squads: Vec<SquadCtl>,
    kits: Vec<KitEntry>,
    // One entry per soldier.
    pos: Vec<V2>,
    facing: Vec<V2>,
    vel: Vec<V2>,
    health: Vec<f32>,
    morale: Vec<f32>,
    fatigue: Vec<f32>,
    kit: Vec<u16>,
    /// Their kit's body radius, kept beside the positions for the
    /// neighbour loops.
    body: Vec<f32>,
    squad: Vec<u16>,
    /// Their slot in the squad's formation: file and rank.
    file: Vec<i16>,
    rank: Vec<u16>,
    /// The soldier one rank ahead in their file, whom they keep behind
    /// while the squad is in contact (`NONE` in the front rank).
    ahead: Vec<u32>,
    /// Attack bonus before exhaustion: proficiency and their weapon's
    /// quality; hit points unhurt; and whether they were in a crush last
    /// substep (restrained).
    bonus: Vec<f32>,
    max_hp: Vec<f32>,
    /// Initiative from experience (`rules::agility` before exhaustion).
    seasoned: Vec<f32>,
    restrained: Vec<bool>,
    target: Vec<u32>,
    cooldown: Vec<f32>,
    arrows: Vec<u16>,
    state: Vec<SoldierState>,
    side: Vec<u8>,
    /// Morale they started with (nerve comes back up to it), and seconds since
    /// they last fought, were hit or saw a friend fall.
    start_morale: Vec<f32>,
    quiet: Vec<f32>,
    grids: [Grid; 2],
    /// The widest body on each side: how far to look for bodies to push off.
    widest: [f32; 2],
    // Buffers, reused every substep.
    plans: Vec<Plan>,
    /// The substep's blows and shots, by striker, in soldier order.
    blows: Vec<(u32, Blow)>,
    /// Which of them land, and the order they land in (scratch).
    lands: Vec<bool>,
    order: Vec<u32>,
    /// What the substep's blows did to each soldier, emptied as it is read.
    hurt: Vec<Hurt>,
    fallen: Vec<usize>,
    broke: Vec<usize>,
    /// Runners taken prisoner this substep.
    captured: Vec<usize>,
    tally: Vec<u32>,
    /// Each squad's nearest enemy squad still fighting, as phase 2 began:
    /// no squad's centre or state changes during it.
    near_foe: Vec<Option<usize>>,
    /// Squad pairs that have met, and that have charged, so each is told once.
    met: Vec<bool>,
    charged: Vec<bool>,
    profile: Option<Profile>,
    /// Each side's doctrine.
    doctrine: [Doctrine; 2],
}

impl SoldierBattle {
    pub fn new(params: BattleParams, field: Field, specs: Vec<SquadSpec>, seed: u64) -> Self {
        let sp = params.soldiers.clone();
        let n: usize = specs.iter().map(|s| s.members.len()).sum();
        let mut b = Self {
            grids: [
                Grid::new(field.width, field.height, sp.cell),
                Grid::new(field.width, field.height, sp.cell),
            ],
            params,
            nav: Nav::new(&field, 0).map(Arc::new),
            ring: crate::battle::walls::Ring::of(&field),
            field,
            seed,
            t: 0.0,
            substep: 0,
            events: Vec::new(),
            squads: Vec::with_capacity(specs.len()),
            kits: Vec::new(),
            pos: Vec::with_capacity(n),
            facing: Vec::with_capacity(n),
            vel: vec![V2::ZERO; n],
            health: Vec::with_capacity(n),
            morale: Vec::with_capacity(n),
            fatigue: Vec::with_capacity(n),
            kit: Vec::with_capacity(n),
            body: Vec::with_capacity(n),
            squad: Vec::with_capacity(n),
            file: vec![0; n],
            rank: vec![0; n],
            ahead: vec![NONE; n],
            bonus: Vec::with_capacity(n),
            max_hp: Vec::with_capacity(n),
            seasoned: Vec::with_capacity(n),
            restrained: vec![false; n],
            target: vec![NONE; n],
            cooldown: Vec::with_capacity(n),
            arrows: Vec::with_capacity(n),
            state: Vec::with_capacity(n),
            side: Vec::with_capacity(n),
            start_morale: Vec::with_capacity(n),
            quiet: vec![0.0; n],
            plans: vec![Plan::default(); n],
            blows: Vec::new(),
            lands: Vec::new(),
            order: Vec::new(),
            hurt: vec![Hurt::default(); n],
            fallen: Vec::new(),
            broke: Vec::new(),
            captured: Vec::new(),
            tally: vec![0; specs.len()],
            near_foe: Vec::new(),
            met: vec![false; specs.len() * specs.len()],
            charged: vec![false; specs.len() * specs.len()],
            profile: None,
            widest: [0.0; 2],
            doctrine: [Doctrine::default(); 2],
        };
        let centres: Vec<(u8, V2)> = specs
            .iter()
            .map(|s| (s.side, V2::new(s.pos.0, s.pos.1)))
            .collect();
        for (c, spec) in specs.into_iter().enumerate() {
            b.add_squad(c, spec, &centres);
        }
        for c in 0..b.squads.len() {
            b.deploy(c);
        }
        for i in 0..b.pos.len() {
            let s = (b.side[i] as usize).min(1);
            b.widest[s] = b.widest[s].max(b.body[i]);
        }
        b
    }

    /// The kit table's entry for `kit`, added if it is new.
    fn intern(&mut self, kit: &Kit) -> u16 {
        if let Some(k) = self.kits.iter().position(|e| &e.kit == kit) {
            return k as u16;
        }
        let (p, sp) = (&self.params, &self.params.soldiers);
        let mass = sp.man_kg + kit.weight + if kit.mounted { sp.horse_kg } else { 0.0 };
        self.kits.push(KitEntry {
            mass,
            body: if kit.mounted { sp.horse_body } else { sp.body },
            top: if kit.mounted {
                p.horse_charge_speed
            } else {
                p.run_speed
            },
            walk: if kit.mounted {
                p.horse_walk_speed
            } else {
                p.walk_speed
            },
            kit: kit.clone(),
        });
        (self.kits.len() - 1) as u16
    }

    fn add_squad(&mut self, c: usize, spec: SquadSpec, centres: &[(u8, V2)]) {
        let p = self.params.clone();
        let first = self.pos.len();
        let kits: Vec<u16> = spec.kits.iter().map(|k| self.intern(k)).collect();
        let squad_kit = self.intern(&spec.kit);
        let morale = spec.morale.clamp(0.0, 1.0);
        let (mut exp, mut standing) = (0.0, 0.0);
        let mut leader: Option<(usize, f32)> = None;
        for (m, member) in spec.members.iter().enumerate() {
            let i = first + m;
            let k = kits.get(member.kit as usize).copied().unwrap_or(squad_kit);
            let e = member.experience.max(0.0);
            let up = member.health > 0.0;
            if up {
                exp += e;
                standing += 1.0;
                if leader.is_none_or(|l| e > l.1) {
                    leader = Some((i, e));
                }
            }
            self.health.push(member.health);
            self.morale.push(morale);
            self.start_morale.push(morale);
            self.fatigue.push(member.fatigue.clamp(0.0, 1.0));
            self.kit.push(k);
            self.body.push(self.kits[k as usize].body);
            self.squad.push(c as u16);
            let kit = &self.kits[k as usize].kit;
            self.bonus
                .push(rules::proficiency(&p, member.skill, e) + kit.quality);
            self.max_hp.push(rules::max_hp(&p, e));
            self.seasoned.push(rules::agility(&p, e, 0.0));
            let mut d = Draw::new(self.seed, 0, i, Purpose::Start);
            self.cooldown
                .push(d.f32() / kit.strike_rate.max(kit.shot_rate).max(1e-3));
            self.arrows.push(kit.arrows.min(u16::MAX as u32) as u16);
            self.state.push(if up {
                SoldierState::Formed
            } else {
                SoldierState::Down
            });
            self.side.push(spec.side);
            self.pos.push(V2::new(spec.pos.0, spec.pos.1));
            self.facing.push(V2::new(1.0, 0.0));
        }
        let seasoned = if standing > 0.0 {
            ((1.0 + exp / standing).ln() / (1.0 + p.experience_full).ln()).min(1.0)
        } else {
            0.0
        };
        let discipline = (0.5 * spec.drill.clamp(0.0, 1.0) + 0.5 * seasoned).clamp(0.0, 1.0);
        let centre = V2::new(spec.pos.0, spec.pos.1);
        // Facing the enemy army as a whole: a squad sent round its side
        // finds it facing the other way.
        let ahead = V2::new(if spec.side == 0 { 1.0 } else { -1.0 }, 0.0);
        let (sum, k) = centres
            .iter()
            .filter(|e| e.0 != spec.side)
            .fold((V2::new(0.0, 0.0), 0.0), |(s, k), e| (s + e.1, k + 1.0));
        let facing = if k > 0.0 {
            (sum * (1.0 / k) - centre).norm_or(ahead)
        } else {
            ahead
        };
        let arrows = self.arrows[first..].iter().map(|&a| a as u32).sum();
        let state = if standing > 0.0 {
            SquadState::Fighting
        } else {
            SquadState::Destroyed
        };
        let mounted = spec.kit.mounted;
        let spacing = if mounted {
            p.horse_spacing
        } else {
            p.foot_spacing
        };
        self.squads.push(SquadCtl {
            spec,
            state,
            orders: Vec::new(),
            stance: Stance::default(),
            morale,
            first,
            formation: Formation::Line,
            width: None,
            reform_due: false,
            squeeze: None,
            assault: None,
            at_wall: false,
            route: Route::default(),
            anchor: centre,
            facing,
            files: 1,
            spacing,
            target: None,
            shoot_at: None,
            contact_time: 0.0,
            rally: None,
            steady: 1.0 - 0.5 * discipline,
            leader: leader.map(|l| l.0),
            arrows,
            told_arrows: false,
            centre,
            front: centre,
            in_line: standing as usize,
            routing: 0,
            in_contact: 0,
            fighting_with: None,
            vigour: 1.0,
            losses: 0,
            left: 0,
            engaged: false,
            wavering: false,
        });
    }

    /// Deploy a squad in the formation it would take, centred on its position.
    fn deploy(&mut self, c: usize) {
        let formation = self.formation_for(c, None, false);
        self.reform(c, formation);
        let s = &mut self.squads[c];
        let (first, n) = (s.first, s.spec.members.len());
        let centre = V2::new(s.spec.pos.0, s.spec.pos.1);
        let ranks = (first..first + n)
            .filter(|&i| self.state[i].present())
            .map(|i| self.rank[i])
            .max()
            .unwrap_or(0);
        s.anchor = centre + s.facing * (ranks as f32 * s.spacing * 0.5);
        if let Some(w) = self.field.wall_x {
            // Defenders line the wall from inside.
            s.anchor.x = if s.spec.behind_wall {
                w + WALL_HALF + 0.5
            } else {
                s.anchor.x.min(w - WALL_HALF - 0.5)
            };
        }
        if let Some(nav) = &self.nav {
            s.anchor = nav.free_spot(s.anchor, s.spec.side);
        }
        s.front = s.anchor;
        let s = &self.squads[c];
        for i in first..first + n {
            self.pos[i] = self.clear_slot(s.slot(self.file[i], self.rank[i]), i);
            self.facing[i] = s.facing;
        }
    }

    /// Where soldier `i` can stand nearest to `p`: out of every obstacle.
    #[inline]
    fn clear_slot(&self, p: V2, i: usize) -> V2 {
        match &self.nav {
            Some(nav) => nav.push_out(p, self.body[i], self.side[i]),
            None => p,
        }
    }

    /// Stand a squad anew before the battle: its front's centre at
    /// `anchor`, facing `facing` (unit), `width` wide if given, everyone in
    /// their slot.
    fn redeploy(&mut self, c: usize, anchor: V2, facing: V2, width: Option<f32>) {
        let s = &mut self.squads[c];
        s.anchor = anchor;
        s.facing = facing;
        if width.is_some() {
            s.width = width;
        }
        let formation = self.formation_for(c, None, false);
        self.reform(c, formation);
        let s = &mut self.squads[c];
        s.front = anchor;
        let (first, n) = (s.first, s.spec.members.len());
        let (mut sum, mut k) = (V2::ZERO, 0.0);
        for i in first..first + n {
            self.pos[i] = self.clear_slot(self.squads[c].slot(self.file[i], self.rank[i]), i);
            self.facing[i] = facing;
            if self.state[i].present() {
                sum += self.pos[i];
                k += 1.0;
            }
        }
        let s = &mut self.squads[c];
        if k > 0.0 {
            s.centre = sum * (1.0 / k);
        }
        s.spec.pos = (s.centre.x, s.centre.y);
    }

    /// Each soldier's morale and fatigue, by squad in member order.
    pub fn condition(&self) -> Vec<Vec<Condition>> {
        self.squads
            .iter()
            .map(|s| {
                (s.first..s.first + s.spec.members.len())
                    .map(|i| Condition::new(self.morale[i], self.fatigue[i]))
                    .collect()
            })
            .collect()
    }

    /// Time per phase from now on (`--profile`).
    pub fn enable_profile(&mut self) {
        self.profile.get_or_insert_with(Profile::default);
    }

    pub fn profile(&self) -> Option<Profile> {
        self.profile
    }

    /// Every soldier of each squad, in member order, as a live battle shows them.
    pub fn frame_soldiers(&self) -> Vec<Vec<FrameSoldier>> {
        self.squads
            .iter()
            .map(|s| {
                (s.first..s.first + s.spec.members.len())
                    .map(|i| {
                        FrameSoldier::new(
                            self.pos[i].x,
                            self.pos[i].y,
                            (self.facing[i].x, self.facing[i].y),
                            match self.state[i] {
                                SoldierState::Routing => FighterState::Routing,
                                SoldierState::Down => FighterState::Down,
                                SoldierState::Fled => FighterState::Fled,
                                _ => FighterState::Standing,
                            },
                        )
                    })
                    .collect()
            })
            .collect()
    }

    /// How many soldiers fight.
    pub fn soldiers(&self) -> usize {
        self.pos.len()
    }

    /// Advance one battle second (`params.dt`), in substeps.
    pub fn step(&mut self) {
        let sp = &self.params.soldiers;
        let k = (self.params.dt / sp.substep.max(1e-3)).round().max(1.0) as usize;
        for _ in 0..k {
            self.substep();
        }
    }

    fn substep(&mut self) {
        let dt = self.params.soldiers.substep;
        let par = self.pos.len() >= self.params.soldiers.parallel_from
            && rayon::current_num_threads() > 1;
        let mut clock = self.profile.map(|_| Instant::now());
        let mut lap = |b: &mut Self, phase: usize| {
            if let (Some(c), Some(p)) = (clock.as_mut(), b.profile.as_mut()) {
                let now = Instant::now();
                p.phases[phase] += (now - *c).as_secs_f64();
                *c = now;
            }
        };
        // 1. Grid.
        for g in &mut self.grids {
            g.clear();
        }
        let soldiers = self.state.iter().zip(&self.side).zip(&self.pos);
        for (i, ((st, &side), &at)) in soldiers.enumerate() {
            if st.present() {
                self.grids[(side as usize).min(1)].push(i, at);
            }
        }
        for g in &mut self.grids {
            g.finish();
        }
        lap(self, 0);
        // 2. Squad controllers, and the wall battered.
        self.command();
        self.batter_walls();
        lap(self, 1);
        // 3 and 4. Soldier controllers, and each soldier's blow or shot.
        let mut plans = std::mem::take(&mut self.plans);
        let mut blows = std::mem::take(&mut self.blows);
        blows.clear();
        {
            let this = &*self;
            if par {
                // In chunks, each listing its blows; joined in soldier order.
                const CHUNK: usize = 256;
                let lists: Vec<Vec<(u32, Blow)>> = plans
                    .par_chunks_mut(CHUNK)
                    .enumerate()
                    .map(|(c, chunk)| {
                        let mut list = Vec::new();
                        for (k, p) in chunk.iter_mut().enumerate() {
                            let i = c * CHUNK + k;
                            // Nobody reads the plan of a soldier who is down or gone.
                            if this.state[i].present() {
                                this.plan_and_blow(i, p, &mut list);
                            }
                        }
                        list
                    })
                    .collect();
                blows.extend(lists.into_iter().flatten());
            } else {
                for (i, p) in plans.iter_mut().enumerate() {
                    if this.state[i].present() {
                        this.plan_and_blow(i, p, &mut blows);
                    }
                }
            }
        }
        lap(self, 2);
        // 5. Apply.
        self.apply(&plans, &blows, dt);
        self.plans = plans;
        self.blows = blows;
        lap(self, 3);
        // 6. Squads.
        self.t += dt;
        self.substep += 1;
        self.recount();
        lap(self, 4);
        if let Some(p) = self.profile.as_mut() {
            p.substeps += 1;
        }
    }

    fn event(&mut self, kind: EventKind) {
        self.events.push(Event { t: self.t, kind });
    }

    /// 1 - `fatigue_effect` * fatigue: the share of their speed and attack
    /// rate a tired soldier keeps.
    #[inline]
    fn vigour(&self, i: usize) -> f32 {
        1.0 - self.params.soldiers.fatigue_effect * self.fatigue[i]
    }

    /// Frightened: a wavering soldier attacks with disadvantage.
    #[inline]
    fn frightened(&self, i: usize) -> bool {
        self.morale[i] < self.params.waver_at
    }

    /// Soldier `i`'s attack bonus now: less a point per level of exhaustion.
    #[inline]
    fn attack_bonus(&self, i: usize) -> f32 {
        self.bonus[i] - self.params.exhaustion_penalty * rules::exhaustion(self.fatigue[i]) as f32
    }

    /// True if the wall stands between two points (outside the breach).
    #[inline]
    fn wall_between(&self, a: V2, b: V2, width: f32) -> bool {
        wall_between(&self.field, a, b, width)
    }
}

/// The share of runners a soldier of `morale` still lays hands on: less
/// when wavering.
#[inline]
fn nerve(p: &BattleParams, morale: f32) -> f32 {
    let w = p.waver_at.max(1e-3);
    if morale >= w {
        1.0
    } else {
        0.5 + 0.5 * morale / w
    }
}

/// True if the wall stands between two points (outside the breach).
/// `width` is the breach's: a defender on the wall strikes down over
/// twice its width.
#[inline]
fn wall_between(field: &Field, a: V2, b: V2, width: f32) -> bool {
    let Some(w) = field.wall_x else {
        return false;
    };
    // The wall is a metre thick: through the breach means through both faces.
    [w - WALL_HALF, w + WALL_HALF].into_iter().any(|x| {
        if (a.x - x) * (b.x - x) >= 0.0 {
            return false;
        }
        let y = a.y + (x - a.x) / (b.x - a.x) * (b.y - a.y);
        (y - field.height * 0.5).abs() > width * 0.5
    })
}

/// Whether `y` is within the breach in the wall.
#[inline]
fn in_breach(field: &Field, p: &BattleParams, y: f32) -> bool {
    (y - field.height * 0.5).abs() <= p.breach_width * 0.5
}

fn off_field(field: &Field, p: V2) -> bool {
    p.x < 0.0 || p.y < 0.0 || p.x > field.width || p.y > field.height
}

impl Model for SoldierBattle {
    fn params(&self) -> &BattleParams {
        &self.params
    }
    fn field(&self) -> &Field {
        &self.field
    }
    fn seconds(&self) -> f32 {
        self.t
    }
    fn step(&mut self) {
        SoldierBattle::step(self);
    }
    fn set_orders(&mut self, squad: usize, orders: &[Order]) {
        if let Some(s) = self.squads.get_mut(squad) {
            s.orders = orders.to_vec();
            s.start_order();
        }
    }
    fn orders(&self, squad: usize) -> &[Order] {
        &self.squads[squad].orders
    }
    fn line(&self, squad: usize) -> SquadLine {
        let s = &self.squads[squad];
        SquadLine {
            anchor: (s.anchor.x, s.anchor.y),
            facing: (s.facing.x, s.facing.y),
            width: s.files as f32 * s.spacing,
            spacing: s.spacing,
        }
    }
    fn set_stance(&mut self, squad: usize, stance: Stance) {
        let s = &mut self.squads[squad];
        s.reform_due |= s.stance.spacing != stance.spacing;
        s.stance = stance;
    }
    fn stance(&self, squad: usize) -> Stance {
        self.squads[squad].stance
    }
    fn set_doctrine(&mut self, side: u8, doctrine: Doctrine) {
        self.doctrine[(side as usize).min(1)] = doctrine;
        for s in self.squads.iter_mut().filter(|s| s.spec.side == side) {
            s.stance.pursuit = doctrine.prisoners;
        }
    }
    fn doctrine(&self, side: u8) -> Doctrine {
        self.doctrine[(side as usize).min(1)]
    }
    fn seed(&self) -> u64 {
        self.seed
    }
    fn pace(&self, squad: usize) -> (f32, f32) {
        let p = &self.params;
        let s = &self.squads[squad];
        let (walk, run) = if s.mounted() {
            (p.horse_walk_speed, p.horse_charge_speed)
        } else {
            (p.walk_speed, p.run_speed)
        };
        (walk * s.vigour, run * s.vigour)
    }
    fn deploy(&mut self, squad: usize, anchor: (f32, f32), facing: (f32, f32), width: Option<f32>) {
        self.redeploy(
            squad,
            V2::new(anchor.0, anchor.1),
            V2::new(facing.0, facing.1),
            width,
        );
    }
    fn count(&self) -> usize {
        self.squads.len()
    }
    fn spec(&self, squad: usize) -> &SquadSpec {
        &self.squads[squad].spec
    }
    fn position(&self, squad: usize) -> (f32, f32) {
        let c = self.squads[squad].centre;
        (c.x, c.y)
    }
    fn state(&self, squad: usize) -> SquadState {
        self.squads[squad].state
    }
    fn morale(&self, squad: usize) -> f32 {
        self.squads[squad].morale
    }
    fn events(&self) -> &[Event] {
        &self.events
    }
    fn take_engaged(&mut self, squad: usize) -> bool {
        std::mem::take(&mut self.squads[squad].engaged)
    }
    fn members_mut(&mut self, squad: usize) -> &mut [Member] {
        &mut self.squads[squad].spec.members
    }
    fn nav(&self) -> Option<&Arc<Nav>> {
        self.nav.as_ref()
    }
    fn set_obstacles(&mut self, obstacles: Vec<Obstacle>) {
        self.field.obstacles = obstacles;
        self.nav = renav(&self.field, self.nav.as_ref());
        for s in &mut self.squads {
            s.route.clear();
        }
    }
}
