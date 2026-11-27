//! The squad model: each squad is a disc on the field with one morale, and
//! each step settles its blows and arrows with dice per squad. A squad is
//! flanked when an enemy it does not face is in contact, and its front is
//! `min(alive, frontage / spacing)`.
//!
//! Among obstacles a disc does the nearest thing to a line of soldiers: it
//! goes round them by the same ways the soldier model's squads take
//! (`nav::Route`), and its front is no wider than the clear ground across
//! its way (`Nav::room`), so in a built-up town fewer fight at once.
//!
//! It takes the soldier model's orders and stances as nearly as a disc can:
//! a move's facing and width and the spacing stance change nothing, a move
//! at a run runs, archers who skirmish back away, and a squad that lets
//! enemies go does not chase those who run.

mod combat;
mod step;

use super::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Clone, Debug)]
struct Squad {
    spec: SquadSpec,
    state: SquadState,
    morale: f32,
    /// Orders left, the first in force.
    orders: Vec<Order>,
    stance: Stance,
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
    /// Its way round the obstacles, and the width of clear ground where it
    /// stands, metres (infinite on an open field).
    route: Route,
    room: f32,
    /// Effective skill and discipline, refreshed every step.
    eff: f32,
    discipline: f32,
}

impl Squad {
    fn alive(&self) -> usize {
        self.spec.members.iter().filter(|m| m.fights()).count()
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
        let front = (self.alive() as f32).min(p.frontage / self.spacing(p));
        if self.room.is_finite() {
            front.min((self.room / self.spacing(p)).floor().max(1.0))
        } else {
            front
        }
    }
    fn has_bow(&self) -> bool {
        self.spec.kit.range > 0.0 && self.arrows > 0
    }
    /// Marching and running pace, metres per second.
    fn paces(&self, p: &BattleParams) -> (f32, f32) {
        if self.spec.kit.mounted {
            (p.horse_walk_speed, p.horse_charge_speed)
        } else {
            (p.walk_speed, p.run_speed)
        }
    }
    /// Whether it strikes enemies who run.
    fn pursues(&self) -> bool {
        self.stance.pursuit == Pursuit::Kill
    }
    /// Whether it goes after enemies who run (to cut them down or take them).
    fn chases(&self) -> bool {
        self.stance.pursuit != Pursuit::LetGo
    }
}

/// A battle between squads, as discs.
pub struct SquadBattle {
    params: BattleParams,
    field: Field,
    /// The field's obstacles (`None`: an open field).
    nav: Option<Arc<Nav>>,
    squads: Vec<Squad>,
    t: f32,
    rng: ChaCha8Rng,
    events: Vec<Event>,
    /// Each side's doctrine.
    doctrine: [Doctrine; 2],
}

impl SquadBattle {
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
                orders: Vec::new(),
                stance: Stance::default(),
                primary: None,
                contacts: Vec::new(),
                closing: Vec::new(),
                speed: 0.0,
                quiet: 0.0,
                engaged: false,
                wavering: false,
                route: Route::default(),
                room: f32::INFINITY,
                eff: 0.0,
                discipline: 0.0,
            })
            .collect();
        let nav = Nav::new(&field, 0).map(Arc::new);
        let mut squads: Vec<Squad> = squads;
        if let Some(nav) = &nav {
            for s in &mut squads {
                let p = nav.free_spot(soldiers::V2::new(s.spec.pos.0, s.spec.pos.1), s.spec.side);
                s.spec.pos = (p.x, p.y);
            }
        }
        Self {
            params,
            nav,
            field,
            squads,
            t: 0.0,
            rng: ChaCha8Rng::seed_from_u64(seed),
            events: Vec::new(),
            doctrine: [Doctrine::default(); 2],
        }
    }

    fn finished(&self) -> bool {
        finished(self)
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

impl Model for SquadBattle {
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
        SquadBattle::step(self);
    }
    fn set_orders(&mut self, squad: usize, orders: &[Order]) {
        if let Some(s) = self.squads.get_mut(squad) {
            s.orders = orders.to_vec();
        }
    }
    fn orders(&self, squad: usize) -> &[Order] {
        &self.squads[squad].orders
    }
    fn line(&self, squad: usize) -> SquadLine {
        let s = &self.squads[squad];
        let spacing = if s.spec.kit.mounted {
            self.params.horse_spacing
        } else {
            self.params.foot_spacing
        };
        SquadLine {
            anchor: s.spec.pos,
            facing: (if s.spec.side == 0 { 1.0 } else { -1.0 }, 0.0),
            width: (standing(&s.spec) as f32).sqrt().ceil() * spacing,
            spacing,
        }
    }
    fn set_stance(&mut self, squad: usize, stance: Stance) {
        self.squads[squad].stance = stance;
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
    fn pace(&self, squad: usize) -> (f32, f32) {
        self.squads[squad].paces(&self.params)
    }
    fn deploy(&mut self, squad: usize, anchor: (f32, f32), _: (f32, f32), _: Option<f32>) {
        self.squads[squad].spec.pos = anchor;
    }
    fn count(&self) -> usize {
        self.squads.len()
    }
    fn spec(&self, squad: usize) -> &SquadSpec {
        &self.squads[squad].spec
    }
    fn position(&self, squad: usize) -> (f32, f32) {
        self.squads[squad].spec.pos
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
