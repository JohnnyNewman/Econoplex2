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
/// 2: the ledger gained income tax (18 flows). 3: agents' pay expectations.
pub const SAVE_VERSION: u32 = 8;

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
    #[serde(default)]
    pub history: crate::history::History,
    /// Hours, season and weather (older saves start with clear ground).
    #[serde(default)]
    pub env: crate::env::Environment,
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
    history: &'a crate::history::History,
    env: &'a crate::env::Environment,
}

/// Fingerprint of the content a world was built from (FNV-1a, stable across builds).
pub fn content_hash(content: &sim_data::Content) -> u64 {
    let text = format!("{content:?}");
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Everything a save holds, borrowed from `world`.
fn save_ref(world: &World) -> SaveRef<'_> {
    // Archetype by archetype keeps entities in the order queries visit them.
    let entities: Vec<SavedEntity> = world
        .archetypes()
        .iter()
        .flat_map(|a| a.entities().iter().map(|e| e.id()))
        .filter_map(|id| SavedEntity::capture(world.entity(id)))
        .collect();
    SaveRef {
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
        history: world.resource::<crate::history::History>(),
        env: world.resource::<crate::env::Environment>(),
    }
}

/// Serialize the simulation state of `world` to RON.
pub fn to_string(world: &World) -> Result<String, String> {
    let save = save_ref(world);
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

/// First bytes of a compact binary save.
pub const BINARY_MAGIC: &[u8; 5] = b"EPXB1";

/// Serialize the simulation state of `world` compactly: MessagePack with field
/// names (so fields left out or added later still read), deflated,
/// after `BINARY_MAGIC`.
pub fn to_bytes(world: &World) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let raw =
        rmp_serde::to_vec_named(&save_ref(world)).map_err(|e| format!("cannot write save: {e}"))?;
    let mut out = BINARY_MAGIC.to_vec();
    let mut z = flate2::write::DeflateEncoder::new(&mut out, flate2::Compression::default());
    z.write_all(&raw)
        .map_err(|e| format!("cannot write save: {e}"))?;
    z.finish().map_err(|e| format!("cannot write save: {e}"))?;
    Ok(out)
}

/// Read a compact binary save (see `to_bytes`).
pub fn from_bytes(bytes: &[u8]) -> Result<SaveFile, String> {
    use std::io::Read;
    let body = bytes
        .strip_prefix(BINARY_MAGIC.as_slice())
        .ok_or("not a binary save")?;
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(body)
        .read_to_end(&mut raw)
        .map_err(|e| format!("cannot read save: {e}"))?;
    let save: SaveFile =
        rmp_serde::from_slice(&raw).map_err(|e| format!("cannot read save: {e}"))?;
    if save.version != SAVE_VERSION {
        return Err(format!(
            "save version {} is not supported (expected {SAVE_VERSION})",
            save.version
        ));
    }
    Ok(save)
}

/// Save to `path`: compact binary for a `.bin` file, readable RON otherwise.
pub fn save_to_file(world: &World, path: &Path) -> Result<(), String> {
    let data = if path.extension().is_some_and(|e| e == "bin") {
        to_bytes(world)?
    } else {
        to_string(world)?.into_bytes()
    };
    std::fs::write(path, data).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Read a save in either format (told apart by its first bytes).
pub fn read_file(path: &Path) -> Result<SaveFile, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if bytes.starts_with(BINARY_MAGIC) {
        from_bytes(&bytes)
    } else {
        let text =
            String::from_utf8(bytes).map_err(|_| format!("{} is not a save", path.display()))?;
        from_str(&text)
    }
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
        history,
        env,
        ..
    } = save;
    let mut db = Db::build(
        content,
        params.embedding.ancestor_decay,
        &params.embedding.method,
        params.embedding.dims,
    );
    db.group_firms(params.market.firms.grouping());

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
    // Saves from before larders existed.
    store.larder.resize(store.len(), Vec::new());
    store.lived_in.resize(store.len(), Vec::new());
    store.owned.resize(store.len(), Vec::new());
    store.comfort.resize(store.len(), 0.0);
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

    let time = params.time.clone();
    world.insert_resource(store);
    world.insert_resource(Towns(towns));
    world.insert_resource(Models::from_params(&params));
    world.insert_resource(crate::decision::DecisionModels::from_model_params(&params));
    world.insert_resource(crate::terrain::Terrain::build(&scenario));
    world.insert_resource(crate::world::player_town(&scenario));
    world.insert_resource(ScenarioSpec(scenario));
    world.insert_resource(Params(params));
    world.insert_resource(db);
    world.insert_resource(crate::profile::Profile::default());
    world.insert_resource(orders);
    // Saves from before sub-day ticks ran whole days: they end on a day's last tick.
    let mut clock = clock;
    let tpd = time.ticks_per_day.max(1);
    if clock.ticks_per_day != tpd {
        clock.ticks_per_day = tpd;
        clock.sub = tpd - 1;
    }
    clock.months_per_year = time.months_per_year.max(1);
    // Saves from before time compression: one game day per day of economy.
    if clock.game_day == 0 && clock.tick > 0 {
        clock.game_day = clock.tick / time.economy_days_per_day.max(1) as u64;
    }
    clock.economy_days_per_day = time.economy_days_per_day.max(1);
    world.insert_resource(clock);
    world.insert_resource(env);
    world.insert_resource(seed);
    world.insert_resource(decisions);
    world.insert_resource(work_log);
    world.insert_resource(metrics);
    world.insert_resource(history);
    world.insert_resource(match_state);
    world.insert_resource(governors);
    Ok(())
}
