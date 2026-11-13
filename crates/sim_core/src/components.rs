//! ECS components for every entity kind: agents, items, buildings, nature,
//! organizations, memberships and tasks.

use crate::store::Role;
use bevy_ecs::prelude::*;

/// Every unit. Its vectors live in [`crate::store::AgentStore`] at `idx`.
#[derive(Component, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Agent {
    pub idx: u32,
}

/// World position in map units (static entities; agents keep theirs in the store).
#[derive(Component, Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

/// Which town an entity belongs to.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TownId(pub u16);

/// A made thing with a definition and a quality. Shared by items and buildings,
/// so quality, recipes and markets treat both uniformly.
#[derive(Component, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Product {
    pub def: u32,
    pub quality: f32,
}

/// A building: a product with a footprint that serves as a workplace.
#[derive(Component, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Structure {
    pub footprint: (f32, f32),
}

/// Individually tracked good (tools, weapons) with wear: `durability` uses
/// left, each `goods::HOURS_PER_USE` hours of use; `worn_hours` is the use
/// since the last whole one (see `goods::wear`).
#[derive(Component, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub durability: u32,
    #[serde(default)]
    pub worn_hours: f32,
}

/// Item sits in its town's armory.
#[derive(Component, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Stored;

/// Item is equipped by the agent with this store index.
#[derive(Component, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct EquippedBy(pub u32);

/// Trees, ore veins, fields.
#[derive(Component, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct NaturalResource {
    pub kind: u32,
    pub amount: f32,
    pub capacity: f32,
    pub regrowth: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OrgKind {
    Town,
    Guild,
    Squad,
}

#[derive(Component, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Organization {
    pub kind: OrgKind,
    pub name: String,
    /// Skill domain for guilds.
    pub domain: Option<u32>,
}

/// Edge entity linking an agent to an organization with a role.
#[derive(Component, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Membership {
    pub agent: u32,
    pub org: Entity,
    pub role: Role,
    pub since_day: u64,
}

/// A unit of work in progress: one run of a recipe by a team.
#[derive(Component, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Task {
    /// Creation order, used to resolve tasks deterministically.
    pub serial: u64,
    pub recipe: u32,
    pub town: u16,
    pub progress: f32,
    pub required: f32,
    pub input_quality: f32,
    /// How many runs of the recipe this team does in one go (larger teams batch work).
    pub batches: u32,
}

#[derive(Component, Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Participants {
    pub workers: Vec<u32>,
    /// Children learning by watching; they do not add work.
    pub apprentices: Vec<u32>,
    /// Workers who took this job as apprentices: they work and learn, at
    /// `apprentice_wage` of a regular worker's pay.
    #[serde(default)]
    pub trainees: Vec<u32>,
}
