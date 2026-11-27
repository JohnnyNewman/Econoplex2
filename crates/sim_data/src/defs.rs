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
    /// Where it is carried: "tool", "weapon", "armour", "shield" or "mount".
    /// Default: tools in the tool slot, weapons in the weapon slot.
    #[serde(default)]
    pub slot: Option<String>,
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
    /// Households use it: the needs it serves (see `HouseholdDef`).
    #[serde(default)]
    pub household: Option<HouseholdDef>,
    /// Weight of one unit, kg (for carrying).
    #[serde(default = "one_kg")]
    pub kg: f32,
    /// What the item does in a squad battle (weapons, armour, shields, mounts).
    #[serde(default)]
    pub fight: Option<FightDef>,
    /// A house (a building): how many people it holds and how well it shelters
    /// them (1 = fully). Houses are counted per town, not placed one by one.
    #[serde(default)]
    pub house: Option<HouseDef>,
    /// A workplace (a building that is not land or a house): the ground it
    /// stands on, map units, across its front and from front to back. Its
    /// front faces the town centre (`sim_core::layout`); soldiers and
    /// walkers keep out of it, and the views draw it this size.
    #[serde(default)]
    pub footprint: Option<(f32, f32)>,
}

impl ProductDef {
    /// A building that is a workplace: neither land nor a house.
    pub fn workplace(&self) -> bool {
        self.category == Category::Building && self.opens.is_none() && self.house.is_none()
    }
}

/// A house: people it holds, and how well it shelters them (0..1).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct HouseDef {
    pub residents: u32,
    pub quality: f32,
}

/// How a blow or a shot hurts: a point, an edge or a weight. Armour stops
/// each kind by its own share.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, serde::Serialize)]
pub enum DamageType {
    #[default]
    Pierce,
    Slash,
    Blunt,
}

impl DamageType {
    pub const ALL: [DamageType; 3] = [DamageType::Pierce, DamageType::Slash, DamageType::Blunt];

    /// Position in per-type arrays such as [`Resist::shares`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// The share of a hit's damage a piece of armour stops, per damage type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Resist {
    pub pierce: f32,
    pub slash: f32,
    pub blunt: f32,
}

impl Resist {
    /// The shares in [`DamageType`] order.
    pub fn shares(&self) -> [f32; 3] {
        [self.pierce, self.slash, self.blunt]
    }
}

/// What an item does in a squad battle (`sim_core::battle`). Every field is 0
/// unless given, so a shield lists only `shield` and a horse only `mounted`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct FightDef {
    /// Melee: reach in metres, health taken per hit (a person has 1), the kind
    /// of damage (required with damage), strikes per second, and the share of
    /// strikes lost once an enemy is inside the reach.
    pub reach: f32,
    pub damage: f32,
    pub damage_type: Option<DamageType>,
    pub strike_rate: f32,
    pub close_penalty: f32,
    /// Shooting: range in metres, shots per second, health per hit, the kind
    /// of damage (Pierce unless given), arrows carried.
    pub range: f32,
    pub shot_rate: f32,
    pub shot_damage: f32,
    pub shot_type: DamageType,
    pub arrows: u32,
    /// Share of a hit's damage stopped, per damage type, and share of arrows
    /// a shield stops.
    pub armour: Resist,
    pub shield: f32,
    /// A mount: its rider fights mounted.
    pub mounted: bool,
}

fn one_kg() -> f32 {
    1.0
}

/// How households use a good (see `sim_core::ladder`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HouseholdDef {
    /// Days a unit lasts.
    pub life_days: f32,
    /// The needs it serves (with `market.consumption: Needs`), and how much.
    #[serde(default)]
    pub serves: Vec<Serves>,
}

/// A need a good serves, how much, and how (see `sim_core::ladder`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Serves {
    pub need: Need,
    /// In the need's unit: degree-days of heat per unit burned, degrees °C
    /// of insulation per piece worn, or points of satisfaction per unit held.
    pub amount: f32,
    pub by: Use,
}

/// The needs ladder, from the bottom rung up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, serde::Serialize)]
pub enum Need {
    // Physiological
    Food,
    Warmth,
    Rest,
    // Safety
    Shelter,
    Health,
    Savings,
    // Belonging
    Belonging,
    // Esteem
    Esteem,
    // Self-actualisation
    Growth,
}

impl Need {
    pub const ALL: [Need; 9] = [
        Need::Food,
        Need::Warmth,
        Need::Rest,
        Need::Shelter,
        Need::Health,
        Need::Savings,
        Need::Belonging,
        Need::Esteem,
        Need::Growth,
    ];
    /// Rung of the ladder, 0 (physiological) .. 4 (self-actualisation).
    pub fn level(self) -> usize {
        match self {
            Need::Food | Need::Warmth | Need::Rest => 0,
            Need::Shelter | Need::Health | Need::Savings => 1,
            Need::Belonging => 2,
            Need::Esteem => 3,
            Need::Growth => 4,
        }
    }
}

/// How a household uses a good to serve a need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Use {
    /// Burned for heat (fuel): used up as the cold asks.
    Burn,
    /// Worn (clothes): insulates while held, wears out over `life_days`.
    Wear,
    /// Kept (pots, linen, silver): serves while held, wears out over `life_days`.
    Own,
    /// Used up over `life_days` (ale).
    Consume,
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
    /// Level of this way of making its outputs: 1 for the first, higher levels improve it
    /// (see docs/design/inventions.md). Each recipe is one recipe-skill, one node of the tech graph.
    #[serde(default = "default_level")]
    pub level: u32,
    /// Recipe-skills a person must know before they can learn or invent this one. Every
    /// entry must be met; an entry is one recipe id, or a list of ids of which any one is enough.
    /// Empty: known from the start of the tech graph.
    #[serde(default)]
    pub requires: Vec<Requirement>,
}

fn default_team() -> u32 {
    3
}

fn default_level() -> u32 {
    1
}

/// One entry of [`RecipeDef::requires`]: `"a"` needs recipe `a`; `["a", "b"]` needs `a` or `b`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Requirement {
    One(String),
    AnyOf(Vec<String>),
}

impl Requirement {
    /// The recipe ids of which any one meets this requirement.
    pub fn options(&self) -> &[String] {
        match self {
            Requirement::One(id) => std::slice::from_ref(id),
            Requirement::AnyOf(ids) => ids,
        }
    }
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
