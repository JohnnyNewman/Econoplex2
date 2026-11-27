//! The squad model: each squad is a disc on the field with one morale, and
//! each step settles its blows and arrows with dice per squad. A squad is
//! flanked when an enemy it does not face is in contact, and its front is
//! `min(alive, frontage / spacing)`.

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

/// A battle between squads, as discs.
pub struct SquadBattle {
    params: BattleParams,
    field: Field,
    squads: Vec<Squad>,
    t: f32,
    rng: ChaCha8Rng,
    events: Vec<Event>,
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
    fn set_order(&mut self, squad: usize, order: Order) {
        if let Some(s) = self.squads.get_mut(squad) {
            s.order = order;
        }
    }
    fn order(&self, squad: usize) -> Order {
        self.squads[squad].order
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
}
