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
/// 10: towns' price-index tallies (`index_year`) and seasonal flow profile
/// (`flow_profile`) became sub-structs, and the model switches with one
/// choice left were folded into plain parameters. 13: production as a flow
/// with runs under way kept per town (`wip`), the day's job choice
/// (`job_today`), and `food_reserve_days` back in days. 14: tasks count
/// the work no practice has counted yet (`Task::worked_runs`). 15: each
/// town's own coin (`Town::coin_value`, `Town::foreign`), careers
/// (`off_trade`, `vacancies`, `intake`, signed `GuildLink::since`). 16: the
/// ledger's coin-exchanged flow (30 flows). 18: town walls (`Town::walls`,
/// `Policy::walls`, the war tallies' wall pieces and cost).
pub const SAVE_VERSION: u32 = 18;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<crate::goods::Owner>,
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
            owner: e.get::<crate::goods::Owner>().copied(),
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

    /// Insert the components on the entity `self.id` (already mapped to this world).
    fn spawn(self, world: &mut World) {
        let mut e = world.entity_mut(self.id);
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
        // Items from saves made before owners existed: owned by the armory
        // they sit in, or by the person who carries them.
        let owner = self.owner.or_else(|| {
            self.item?;
            if let Some(b) = self.equipped_by {
                Some(crate::goods::Owner(crate::goods::Holder::Person(b.0)))
            } else {
                self.town
                    .filter(|_| self.stored)
                    .map(|t| crate::goods::Owner(crate::goods::Holder::Armory(t.0)))
            }
        });
        if let Some(c) = owner {
            e.insert(c);
        }
        if let Some(c) = self.nature {
            e.insert(c);
        }
        if let Some(c) = self.organization {
            e.insert(c);
        }
        if let Some(c) = self.membership {
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
#[derive(Serialize, Deserialize)]
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
    #[serde(default)]
    pub squads: crate::goods::Squads,
}

impl SaveFile {
    /// Rewrite every entity reference in the save through `map`.
    fn remap(&mut self, map: &EntityMap) {
        for s in &mut self.entities {
            s.id = map.get(s.id);
            if let Some(c) = &mut s.membership {
                c.org = map.get(c.org);
            }
        }
        let store = &mut self.store;
        map.all(&mut store.entity);
        let opt = |v: &mut Vec<Option<Entity>>| v.iter_mut().for_each(|e| *e = map.opt(*e));
        opt(&mut store.equipped);
        opt(&mut store.weapon);
        opt(&mut store.armour);
        opt(&mut store.shield);
        opt(&mut store.mount);
        for a in &mut store.activity {
            if let Activity::Working(e) = a {
                *e = map.get(*e);
            }
        }
        for g in store.guild.iter_mut().flatten() {
            g.org = map.get(g.org);
            g.membership = map.get(g.membership);
        }
        for t in &mut self.towns {
            t.org = map.get(t.org);
            t.squad_org = map.get(t.squad_org);
            t.buildings.iter_mut().for_each(|b| b.1 = map.get(b.1));
            map.all(&mut t.nature_sites);
            map.all(&mut t.armory);
            map.all(&mut t.guilds);
        }
        for s in &mut self.squads.list {
            map.all(&mut s.items);
        }
    }

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
    squads: &'a crate::goods::Squads,
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
        squads: world.resource::<crate::goods::Squads>(),
    }
}

/// FNV-1a over the bytes written to it (stable across builds and platforms).
struct Fnv(u64);

impl std::io::Write for Fnv {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &b in buf {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The saved state of `world` with entities in an order that depends on the
/// state alone and their ids replaced by their position in that order, so that
/// a loaded game gives the same as the game it was saved from.
fn canonical(world: &World) -> SaveFile {
    let raw = rmp_serde::to_vec_named(&save_ref(world)).expect("the saved state serializes");
    let mut save: SaveFile = rmp_serde::from_slice(&raw).expect("the saved state reads back");
    // The order entities are saved in follows the world's internal tables, which
    // a loaded world builds in another order. Sort by what each entity holds
    // (leaving out ids), so the order depends on the state alone. Entities
    // that hold exactly the same keep their saved order.
    let blank = EntityMap(HashMap::new());
    let mut keyed: Vec<(Vec<u8>, SavedEntity)> = std::mem::take(&mut save.entities)
        .into_iter()
        .map(|mut s| {
            let (id, org) = (s.id, s.membership.as_ref().map(|m| m.org));
            s.id = Entity::PLACEHOLDER;
            if let Some(m) = &mut s.membership {
                m.org = blank.get(m.org);
            }
            let key = rmp_serde::to_vec_named(&s).expect("the saved state serializes");
            s.id = id;
            if let (Some(m), Some(org)) = (&mut s.membership, org) {
                m.org = org;
            }
            (key, s)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    save.entities = keyed.into_iter().map(|(_, s)| s).collect();
    let map = EntityMap(
        save.entities
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id, Entity::from_bits(i as u64 + 1)))
            .collect(),
    );
    save.remap(&map);
    save
}

/// The state that [`state_hash`] hashes, as text. Comparing the text of two
/// worlds line by line shows where they differ when their hashes do.
pub fn state_text(world: &World) -> String {
    let config = ron::ser::PrettyConfig::new().depth_limit(3);
    ron::ser::to_string_pretty(&canonical(world), config).expect("the saved state serializes")
}

/// Hash of the simulation state: everything a save holds except what the
/// game was set up with (parameters, scenario, content), in the form
/// [`canonical`] gives it, so a loaded game hashes like the game it was saved
/// from. A field added to the save is hashed without further work.
pub fn state_hash(world: &World) -> u64 {
    let save = canonical(world);
    let SaveFile {
        clock,
        seed,
        entities,
        store,
        towns,
        orders,
        work_log,
        decisions,
        metrics,
        match_state,
        governors,
        history,
        env,
        squads,
        version: _,
        content_hash: _,
        cap_dim: _,
        params: _,
        scenario: _,
    } = &save;
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    rmp_serde::encode::write(
        &mut h,
        &(
            (clock, seed, entities, store, towns, orders, work_log),
            (
                decisions,
                metrics,
                match_state,
                governors,
                history,
                env,
                squads,
            ),
        ),
    )
    .expect("the saved state serializes");
    h.0
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
    mut save: SaveFile,
    content: sim_data::Content,
) -> Result<(), String> {
    save.check(&content)?;
    let map = EntityMap(
        save.entities
            .iter()
            .map(|s| (s.id, world.spawn_empty().id()))
            .collect(),
    );
    save.remap(&map);
    let SaveFile {
        params,
        scenario,
        clock,
        seed,
        entities,
        mut store,
        towns,
        orders,
        work_log,
        decisions,
        metrics,
        match_state,
        governors,
        history,
        env,
        squads,
        ..
    } = save;
    let mut db = Db::build(
        content,
        params.embedding.ancestor_decay,
        &params.embedding.method,
        params.embedding.dims,
    );
    db.group_firms(params.market.firms.grouping());

    for s in entities {
        s.spawn(world);
    }

    // Saves from before larders existed.
    store.larder.resize(store.len(), Vec::new());
    store.nutrition.resize(store.len(), [1.0; 3]);
    store.lived_in.resize(store.len(), Vec::new());
    store.owned.resize(store.len(), Vec::new());
    store.off_trade.resize(store.len(), 0);
    store.armour.resize(store.len(), None);
    store.shield.resize(store.len(), None);
    store.mount.resize(store.len(), None);
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
    world.insert_resource(clock);
    world.insert_resource(env);
    world.insert_resource(squads);
    world.insert_resource(seed);
    world.insert_resource(decisions);
    world.insert_resource(work_log);
    world.insert_resource(metrics);
    world.insert_resource(history);
    world.insert_resource(match_state);
    world.insert_resource(governors);
    Ok(())
}
