//! Saving and loading the full simulation state.
//!
//! A save holds everything that changes while the simulation runs: the clock,
//! the agent store, towns, orders, the work log, metrics and every simulation
//! entity with its components. It also keeps the model parameters and the
//! scenario the world was started from, so command-line overrides survive a
//! reload. Content (skills, products, recipes, nature) is not stored; it is
//! loaded from the assets again and checked against a fingerprint in the save.
//! Everything derived (content database, models, terrain) is rebuilt on load.
//!
//! Entities get new ids on load. Every entity reference in the state is mapped
//! to the new id; references to entities that no longer existed when the game
//! was saved become [`Entity::PLACEHOLDER`], which no query finds either.
//!
//! The format is RON. Floats are written in their shortest exact form, so a
//! loaded world has the same [`crate::state_hash`] and continues exactly as
//! the saved one would have.

use crate::components::*;
use crate::config::{ModelParams, Scenario};
use crate::db::Db;
use crate::metrics::Metrics;
use crate::models::Models;
use crate::policy::OrderQueue;
use crate::store::{Activity, AgentStore};
use crate::world::{Decisions, Params, ScenarioSpec, SimClock, SimSeed, Town, Towns, WorkLog};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Bumped whenever the save layout changes incompatibly.
/// 2: the ledger gained income tax (18 flows).
pub const SAVE_VERSION: u32 = 2;

/// One simulation entity and its components.
#[derive(Serialize, Deserialize)]
pub struct SavedEntity {
    pub id: Entity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Agent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<Pos>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub town: Option<TownId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<Product>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<Structure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Item>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipped_by: Option<EquippedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nature: Option<NaturalResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<Organization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership: Option<Membership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Task>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participants: Option<Participants>,
}

impl SavedEntity {
    fn capture(e: EntityRef) -> Option<Self> {
        let s = SavedEntity {
            id: e.id(),
            agent: e.get::<Agent>().copied(),
            pos: e.get::<Pos>().copied(),
            town: e.get::<TownId>().copied(),
            product: e.get::<Product>().copied(),
            structure: e.get::<Structure>().copied(),
            item: e.get::<Item>().copied(),
            stored: e.contains::<Stored>(),
            equipped_by: e.get::<EquippedBy>().copied(),
            nature: e.get::<NaturalResource>().copied(),
            organization: e.get::<Organization>().cloned(),
            membership: e.get::<Membership>().copied(),
            task: e.get::<Task>().cloned(),
            participants: e.get::<Participants>().cloned(),
        };
        let any = s.agent.is_some()
            || s.pos.is_some()
            || s.town.is_some()
            || s.product.is_some()
            || s.structure.is_some()
            || s.item.is_some()
            || s.stored
            || s.equipped_by.is_some()
            || s.nature.is_some()
            || s.organization.is_some()
            || s.membership.is_some()
            || s.task.is_some()
            || s.participants.is_some();
        any.then_some(s)
    }

    fn spawn(self, world: &mut World, new: Entity, map: &EntityMap) {
        let mut e = world.entity_mut(new);
        if let Some(c) = self.agent {
            e.insert(c);
        }
        if let Some(c) = self.pos {
            e.insert(c);
        }
        if let Some(c) = self.town {
            e.insert(c);
        }
        if let Some(c) = self.product {
            e.insert(c);
        }
        if let Some(c) = self.structure {
            e.insert(c);
        }
        if let Some(c) = self.item {
            e.insert(c);
        }
        if self.stored {
            e.insert(Stored);
        }
        if let Some(c) = self.equipped_by {
            e.insert(c);
        }
        if let Some(c) = self.nature {
            e.insert(c);
        }
        if let Some(c) = self.organization {
            e.insert(c);
        }
        if let Some(mut c) = self.membership {
            c.org = map.get(c.org);
            e.insert(c);
        }
        if let Some(c) = self.task {
            e.insert(c);
        }
        if let Some(c) = self.participants {
            e.insert(c);
        }
    }
}

/// The save layout. [`SaveRef`] writes the same fields by reference.
#[derive(Deserialize)]
pub struct SaveFile {
    pub version: u32,
    pub content_hash: u64,
    /// Length of the capability vectors in the save.
    #[serde(default)]
    pub cap_dim: usize,
    pub params: ModelParams,
    pub scenario: Scenario,
    pub clock: SimClock,
    pub seed: SimSeed,
    pub entities: Vec<SavedEntity>,
    pub store: AgentStore,
    pub towns: Vec<Town>,
    pub orders: OrderQueue,
    pub work_log: WorkLog,
    pub decisions: Decisions,
    pub metrics: Metrics,
    #[serde(default)]
    pub match_state: crate::goal::MatchState,
    #[serde(default)]
    pub governors: crate::governor::Governors,
}

impl SaveFile {
    /// Whether this save can be restored with `content`: same content, valid parameters.
    pub fn check(&self, content: &sim_data::Content) -> Result<(), String> {
        if content_hash(content) != self.content_hash {
            return Err(
                "the game content (skills, products or recipes) changed since this save was made"
                    .into(),
            );
        }
        self.params.validate().map_err(|e| e.to_string())?;
        // Capability vectors must have the length this content and these parameters give.
        let dim = self
            .params
            .embedding
            .dims
            .unwrap_or_else(|| sim_data::dims::cap_dim_for(content.skills.len()))
            .max(1);
        let saved = if self.cap_dim > 0 {
            self.cap_dim
        } else {
            self.store.cap.first().map_or(dim, Vec::len)
        };
        let consistent = self
            .store
            .cap
            .iter()
            .chain(&self.store.equip_vec)
            .all(|v| v.len() == saved);
        if saved != dim || !consistent {
            return Err(format!(
                "this save has {saved}-dimensional skills, but the game now uses {dim}"
            ));
        }
        Ok(())
    }

    pub fn day(&self) -> u64 {
        self.clock.tick
    }
}

#[derive(Serialize)]
struct SaveRef<'a> {
    version: u32,
    content_hash: u64,
    cap_dim: usize,
    params: &'a ModelParams,
    scenario: &'a Scenario,
    clock: &'a SimClock,
    seed: &'a SimSeed,
    entities: Vec<SavedEntity>,
    store: &'a AgentStore,
    towns: &'a Vec<Town>,
    orders: &'a OrderQueue,
    work_log: &'a WorkLog,
    decisions: &'a Decisions,
    metrics: &'a Metrics,
    match_state: &'a crate::goal::MatchState,
    governors: &'a crate::governor::Governors,
}

/// Fingerprint of the content a world was built from (FNV-1a, stable across builds).
pub fn content_hash(content: &sim_data::Content) -> u64 {
    let text = format!("{content:?}");
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Serialize the simulation state of `world` to RON.
pub fn to_string(world: &World) -> Result<String, String> {
    // Archetype by archetype keeps entities in the order queries visit them.
    let entities: Vec<SavedEntity> = world
        .archetypes()
        .iter()
        .flat_map(|a| a.entities().iter().map(|e| e.id()))
        .filter_map(|id| SavedEntity::capture(world.entity(id)))
        .collect();
    let save = SaveRef {
        version: SAVE_VERSION,
        content_hash: content_hash(&world.resource::<Db>().content),
        cap_dim: world.resource::<Db>().cap_dim,
        params: &world.resource::<Params>().0,
        scenario: &world.resource::<ScenarioSpec>().0,
        clock: world.resource::<SimClock>(),
        seed: world.resource::<SimSeed>(),
        entities,
        store: world.resource::<AgentStore>(),
        towns: &world.resource::<Towns>().0,
        orders: world.resource::<OrderQueue>(),
        work_log: world.resource::<WorkLog>(),
        decisions: world.resource::<Decisions>(),
        metrics: world.resource::<Metrics>(),
        match_state: world.resource::<crate::goal::MatchState>(),
        governors: world.resource::<crate::governor::Governors>(),
    };
    let config = ron::ser::PrettyConfig::new().depth_limit(2);
    ron::ser::to_string_pretty(&save, config).map_err(|e| format!("cannot write save: {e}"))
}

pub fn from_str(text: &str) -> Result<SaveFile, String> {
    let save: SaveFile = ron::from_str(text).map_err(|e| format!("cannot read save: {e}"))?;
    if save.version != SAVE_VERSION {
        return Err(format!(
            "save version {} is not supported (expected {SAVE_VERSION})",
            save.version
        ));
    }
    Ok(save)
}

pub fn save_to_file(world: &World, path: &Path) -> Result<(), String> {
    let text = to_string(world)?;
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn read_file(path: &Path) -> Result<SaveFile, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    from_str(&text)
}

/// Old entity id to new entity id.
struct EntityMap(HashMap<Entity, Entity>);

impl EntityMap {
    fn get(&self, old: Entity) -> Entity {
        self.0.get(&old).copied().unwrap_or(Entity::PLACEHOLDER)
    }
    fn opt(&self, old: Option<Entity>) -> Option<Entity> {
        old.map(|e| self.get(e))
    }
    fn all(&self, v: &mut [Entity]) {
        v.iter_mut().for_each(|e| *e = self.get(*e));
    }
}

/// Put a saved simulation into `world`: its resources and entities. `content`
/// must be the content the game was saved with. The world should hold no
/// simulation entities yet (a fresh world, or a Bevy app before setup).
pub fn restore(
    world: &mut World,
    save: SaveFile,
    content: sim_data::Content,
) -> Result<(), String> {
    save.check(&content)?;
    let SaveFile {
        params,
        scenario,
        clock,
        seed,
        entities,
        mut store,
        mut towns,
        orders,
        work_log,
        decisions,
        metrics,
        match_state,
        governors,
        ..
    } = save;
    let db = Db::build(
        content,
        params.embedding.ancestor_decay,
        &params.embedding.method,
        params.embedding.dims,
    );

    let map = EntityMap(
        entities
            .iter()
            .map(|s| (s.id, world.spawn_empty().id()))
            .collect(),
    );
    for s in entities {
        let new = map.get(s.id);
        s.spawn(world, new, &map);
    }

    map.all(&mut store.entity);
    for i in 0..store.len() {
        store.equipped[i] = map.opt(store.equipped[i]);
        store.weapon[i] = map.opt(store.weapon[i]);
        if let Activity::Working(e) = store.activity[i] {
            store.activity[i] = Activity::Working(map.get(e));
        }
        if let Some(g) = &mut store.guild[i] {
            g.org = map.get(g.org);
            g.membership = map.get(g.membership);
        }
    }
    for t in &mut towns {
        t.org = map.get(t.org);
        t.squad_org = map.get(t.squad_org);
        t.buildings.iter_mut().for_each(|b| b.1 = map.get(b.1));
        map.all(&mut t.nature_sites);
        map.all(&mut t.armory);
        map.all(&mut t.guilds);
    }

    world.insert_resource(store);
    world.insert_resource(Towns(towns));
    world.insert_resource(Models::from_params(&params));
    world.insert_resource(crate::terrain::Terrain::build(&scenario));
    world.insert_resource(crate::world::player_town(&scenario));
    world.insert_resource(ScenarioSpec(scenario));
    world.insert_resource(Params(params));
    world.insert_resource(db);
    world.insert_resource(crate::profile::Profile::default());
    world.insert_resource(orders);
    world.insert_resource(clock);
    world.insert_resource(seed);
    world.insert_resource(decisions);
    world.insert_resource(work_log);
    world.insert_resource(metrics);
    world.insert_resource(match_state);
    world.insert_resource(governors);
    Ok(())
}
