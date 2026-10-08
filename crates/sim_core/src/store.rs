//! The agent vector store: all per-agent vectors in contiguous arrays.
//!
//! Agents are ECS entities carrying an [`crate::components::Agent`] component with
//! an index into this store. Systems that do bulk math (learning, decisions,
//! social influence) iterate the arrays directly, which keeps the work cache-
//! and SIMD-friendly and makes a later GPU port a matter of uploading buffers.
//!
//! Indices are never reused in this prototype: dead agents keep their slot with
//! `alive = false`, so stale references (parents, partners) stay well-defined.

use bevy_ecs::prelude::*;
use sim_data::dims::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Child,
    Idle,
    Resting,
    Working(Entity),
    /// Serving in the town's squad (drilling at home, or marching on a raid).
    Soldier,
    /// Taken in a raid and marched to the captor's town.
    Captive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Apprentice,
    Member,
    Master,
}

#[derive(Debug, Clone, Copy)]
pub struct GuildLink {
    pub org: Entity,
    pub membership: Entity,
    pub domain: u32,
    pub role: Role,
}

#[derive(Resource, Default)]
pub struct AgentStore {
    // --- latent / named vector blocks
    pub cap: Vec<CapVec>,
    pub mind: Vec<MindVec>,
    pub phys: Vec<PhysVec>,
    pub state: Vec<StateVec>,
    // --- genome (fixed at birth)
    pub potential: Vec<PhysVec>,
    pub temperament: Vec<MindVec>,
    pub aptitude: Vec<f32>,
    // --- equipment: capability bonus from the equipped tool (cached)
    pub equip_vec: Vec<CapVec>,
    pub equipped: Vec<Option<Entity>>,
    /// Soldiers' weapon and its cached fighting power.
    pub weapon: Vec<Option<Entity>>,
    pub weapon_power: Vec<f32>,
    // --- bookkeeping
    pub alive: Vec<bool>,
    pub entity: Vec<Entity>,
    pub town: Vec<u16>,
    pub age_days: Vec<u32>,
    pub female: Vec<bool>,
    pub partner: Vec<Option<u32>>,
    pub parents: Vec<Option<(u32, u32)>>,
    pub wealth: Vec<f32>,
    pub activity: Vec<Activity>,
    pub guild: Vec<Option<GuildLink>>,
    /// Recipe worked today (for physique training), cleared every tick.
    pub worked_today: Vec<Option<u32>>,
    pub last_recipe: Vec<Option<u32>>,
    /// Where the agent is heading (world units). The view animates toward it.
    pub target: Vec<(f32, f32)>,
}

/// Everything needed to create an agent.
pub struct NewAgent {
    pub entity: Entity,
    pub town: u16,
    pub female: bool,
    pub age_days: u32,
    pub cap: CapVec,
    pub mind: MindVec,
    pub phys: PhysVec,
    pub potential: PhysVec,
    pub temperament: MindVec,
    pub aptitude: f32,
    pub parents: Option<(u32, u32)>,
    pub target: (f32, f32),
    pub wealth: f32,
}

impl AgentStore {
    pub fn len(&self) -> usize {
        self.alive.len()
    }

    pub fn is_empty(&self) -> bool {
        self.alive.is_empty()
    }

    pub fn push(&mut self, a: NewAgent, adult: bool) -> u32 {
        let idx = self.alive.len() as u32;
        self.cap.push(a.cap);
        self.mind.push(a.mind);
        self.phys.push(a.phys);
        let mut st = [0.0; STATE_DIM];
        st[state::HUNGER] = 0.2;
        st[state::HAPPINESS] = 0.6;
        st[state::HEALTH] = 1.0;
        self.state.push(st);
        self.potential.push(a.potential);
        self.temperament.push(a.temperament);
        self.aptitude.push(a.aptitude);
        self.equip_vec.push([0.0; CAP_DIM]);
        self.equipped.push(None);
        self.weapon.push(None);
        self.weapon_power.push(0.0);
        self.alive.push(true);
        self.entity.push(a.entity);
        self.town.push(a.town);
        self.age_days.push(a.age_days);
        self.female.push(a.female);
        self.partner.push(None);
        self.parents.push(a.parents);
        self.wealth.push(a.wealth);
        self.activity.push(if adult {
            Activity::Idle
        } else {
            Activity::Child
        });
        self.guild.push(None);
        self.worked_today.push(None);
        self.last_recipe.push(None);
        self.target.push(a.target);
        idx
    }

    /// Base capability plus equipment bonus.
    #[inline]
    pub fn effective_cap(&self, i: usize) -> CapVec {
        let mut v = self.cap[i];
        let e = &self.equip_vec[i];
        for k in 0..CAP_DIM {
            v[k] += e[k];
        }
        v
    }

    pub fn age_years(&self, i: usize, days_per_year: u32) -> f32 {
        self.age_days[i] as f32 / days_per_year as f32
    }

    pub fn alive_count(&self) -> usize {
        self.alive.iter().filter(|a| **a).count()
    }
}
