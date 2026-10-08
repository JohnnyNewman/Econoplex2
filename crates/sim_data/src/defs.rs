//! Designer-facing definitions, deserialized from RON.

use serde::Deserialize;

/// One content file. Every section is optional so a domain file only lists what it adds.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContentFile {
    #[serde(default)]
    pub skills: Vec<SkillDef>,
    #[serde(default)]
    pub products: Vec<ProductDef>,
    #[serde(default)]
    pub recipes: Vec<RecipeDef>,
    #[serde(default)]
    pub nature: Vec<NatureDef>,
}

/// A granular capability. Skills form a DAG through `prereqs` (several parents allowed).
#[derive(Debug, Clone, Deserialize)]
pub struct SkillDef {
    pub id: String,
    pub name: String,
    pub domain: String,
    #[serde(default)]
    pub prereqs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
pub enum Category {
    Raw,
    Material,
    Food,
    Tool,
    Weapon,
    Building,
}

impl Category {
    /// Goods tracked as individual entities (with their own quality) rather than as stacks.
    pub fn is_item(self) -> bool {
        matches!(self, Category::Tool | Category::Weapon)
    }
}

/// Bonus an equipped item gives along a skill domain.
#[derive(Debug, Clone, Deserialize)]
pub struct EquipDef {
    pub domain: String,
    pub bonus: f32,
    #[serde(default = "default_durability")]
    pub durability: u32,
}

fn default_durability() -> u32 {
    60
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProductDef {
    pub id: String,
    pub name: String,
    pub category: Category,
    #[serde(default)]
    pub tier: u8,
    pub base_price: f32,
    /// Hunger removed when eaten (0 = not food).
    #[serde(default)]
    pub food: f32,
    #[serde(default)]
    pub equip: Option<EquipDef>,
    /// Buildings: how many people can work in one at a time (0 = unlimited).
    #[serde(default)]
    pub slots: u32,
}

/// One production step. A product such as a level-3 longsword is the end of a chain of steps.
#[derive(Debug, Clone, Deserialize)]
pub struct RecipeDef {
    pub id: String,
    pub name: String,
    /// The skill exercised by this step.
    pub skill: String,
    /// Required effective proficiency for a 50% success chance.
    pub difficulty: f32,
    #[serde(default)]
    pub inputs: Vec<(String, u32)>,
    pub outputs: Vec<(String, u32)>,
    /// Natural resource kind that must be harvested (and is depleted by the outputs).
    #[serde(default)]
    pub nature: Option<String>,
    /// Building product id that must exist in the town.
    #[serde(default)]
    pub building: Option<String>,
    /// Work required, in person-days.
    pub duration: f32,
    #[serde(default = "default_team")]
    pub max_team: u32,
    /// Physical demands by physique dimension name (weights; sum should stay below 1).
    #[serde(default)]
    pub physique: Vec<(String, f32)>,
}

fn default_team() -> u32 {
    3
}

#[derive(Debug, Clone, Deserialize)]
pub struct NatureDef {
    pub id: String,
    pub name: String,
    /// Units regrown per day.
    pub regrowth: f32,
    /// Maximum units per resource site.
    pub capacity: f32,
}
