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
    /// Names of the technology eras, oldest first (the last file that lists them wins).
    #[serde(default)]
    pub eras: Vec<String>,
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
    /// Land: finishing this "building" opens a new nature site of this kind (a
    /// cleared field, a planted woodlot) instead of putting up a workplace.
    #[serde(default)]
    pub opens: Option<String>,
    /// Coin: each unit made is this much new money in the town treasury instead of
    /// a good in stock (0 = not money). Only a mint makes coin.
    #[serde(default)]
    pub coin: f32,
    /// Technology era the product belongs to (index into the content's `eras`).
    /// A label for the player only: no mechanic reads it.
    #[serde(default)]
    pub era: u8,
    /// Buildings: towns may start with it (scenario.ron lists which do). Starter
    /// buildings break bootstrap loops such as planks needing a workshop.
    #[serde(default)]
    pub starter: bool,
    /// How the good's price moves (see `sim_core::pricing`); none: food is
    /// essential, everything else a commodity.
    #[serde(default)]
    pub price_class: Option<PriceClass>,
    /// Households use it: with `market.consumption: Basket` and no goods listed
    /// there, every good that says so is in the household basket.
    #[serde(default)]
    pub household: Option<HouseholdDef>,
}

/// How households use a good (see `sim_core::consumption`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HouseholdDef {
    /// Units a household of median income and middling taste wants to hold.
    pub want: f32,
    /// Days a unit lasts.
    pub life_days: f32,
    /// Mind dimension (by name) that makes an agent want more of it.
    #[serde(default)]
    pub taste: Option<String>,
    /// How much more richer households want (overrides the basket's
    /// `elasticity`): above 1 for luxuries.
    #[serde(default)]
    pub elasticity: Option<f32>,
    /// A luxury bought only from spare money: a household buys it only while
    /// its wealth is at least this many times its town's median.
    #[serde(default)]
    pub min_wealth: Option<f32>,
}

impl ProductDef {
    pub fn price_class(&self) -> PriceClass {
        self.price_class
            .unwrap_or(if self.category == Category::Food {
                PriceClass::Essential
            } else {
                PriceClass::Commodity
            })
    }
}

/// How a good's price moves: essentials (food, fuel, clothing) smoothly and
/// within a tight daily cap, commodities faster. Labour is the class of wages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
pub enum PriceClass {
    Essential,
    Commodity,
    Labour,
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
