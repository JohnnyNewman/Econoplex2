//! The town market (`market` in models.ron): prices, imports, buildings, caravans.

use serde::{Deserialize, Serialize};

fn default_starting_coins() -> f32 {
    1000.0
}

fn default_mint_below() -> f32 {
    3.0
}

/// `market.pricing` in models.ron: the stock of each good the market aims for. A
/// good's price rises while its stock is below that target and falls above it.
///
/// Goods that trades use as inputs: `cover_days` of what the town's trades use
/// a day, plus what profitable trades held up only by this input's shortage
/// would use (`blocked_workers` workers each). Final goods keep the per-resident
/// target; food gets both. So an input's price follows the demand of the
/// trades that need it, and caravans carry it to where it is wanted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pricing {
    pub cover_days: f32,
    pub blocked_workers: f32,
}

impl Default for Pricing {
    fn default() -> Self {
        Pricing {
            cover_days: 10.0,
            blocked_workers: 2.0,
        }
    }
}

/// `market.imports` in models.ron: emergency food imports.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportParams {
    /// Food units bought from one neighbour per good per trade round, at most.
    pub amount: f32,
    /// With imports on auto: buy while food per resident is below this share of
    /// `food_target_per_capita`.
    pub hungry_below: f32,
    /// A seller only sells stock above this share of its own food target.
    pub seller_keeps: f32,
    /// Yearly interest on import debts.
    pub debt_rate: f32,
    /// Share of the treasury above its reserve paid toward debts each day.
    pub repay_share: f32,
}

impl Default for ImportParams {
    fn default() -> Self {
        ImportParams {
            amount: 1000.0,
            hungry_below: 0.5,
            seller_keeps: 0.5,
            debt_rate: 0.05,
            repay_share: 0.05,
        }
    }
}

/// `market.scarcity_cap` in models.ron: how far a scarce good's price may rise.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum ScarcityCap {
    /// Prices stay within `max_price_factor` × base price.
    #[default]
    Fixed,
    /// While a good sits at its cap and is still short, each day its cap rises by
    /// `rate` (relative), up to `max_factor` × base price; once it is no longer at
    /// its cap and short, the cap falls back toward `max_price_factor` at the same
    /// rate. So a good that stays scarce can bid workers away from other trades.
    Loosen { rate: f32, max_factor: f32 },
}

/// What another building of a type a town already has is worth to it: what
/// its places would earn. While the town's buildings of the type are full
/// (`build_threshold`), its slots times how much more its best job pays than
/// the town's typical job, over `payback_days`, at most `max_factor` times
/// base price. Workers build what pays back.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BuildingValue {
    pub payback_days: f32,
    pub max_factor: f32,
}

impl Default for BuildingValue {
    fn default() -> Self {
        BuildingValue {
            payback_days: 10.0,
            max_factor: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketParams {
    pub price_adjust: f32,
    pub target_per_capita: f32,
    pub food_target_per_capita: f32,
    /// Nobody makes a good once the town holds this many times its target stock.
    pub glut_factor: f32,
    /// Also stop offering work on a good as soon as the town holds more than
    /// its target stock and its price has fallen to the floor: the market
    /// signals it has enough. Off by default (only `glut_factor` applies).
    #[serde(default)]
    pub stop_at_floor: bool,
    /// Days of the town's food need whose inputs (grain, flour, wood) are
    /// kept for the mills and bakeries, out of reach of eating and of other
    /// trades.
    pub food_reserve_days: f32,
    pub min_price_factor: f32,
    pub max_price_factor: f32,
    /// Whether a good's price cap loosens while it stays scarce.
    #[serde(default)]
    pub scarcity_cap: ScarcityCap,
    /// What stock of each good the market aims for (and so where its price goes).
    #[serde(default)]
    pub pricing: Pricing,
    /// Who buys output and pays wages: the treasury, or workshops as firms (see
    /// `firms`).
    #[serde(default)]
    pub firms: crate::firms::FirmsParams,
    /// What households buy for their own use (see `consumption`).
    #[serde(default)]
    pub consumption: crate::consumption::ConsumptionParams,
    /// Price dynamics per price class (see `pricing`); none: the legacy rule.
    #[serde(default)]
    pub price_classes: Option<crate::pricing::PriceClasses>,
    /// Per-good dynamics that replace their class's: (product id, params).
    #[serde(default)]
    pub price_overrides: Vec<(String, crate::pricing::ClassParams)>,
    /// Months (simulated days) between caravan rounds (and import rounds).
    pub trade_interval: u32,
    pub trade_amount: f32,
    /// How much a caravan carries. `Fixed` (default): up to `trade_amount`
    /// units, at most half the seller's stock. `Balance { max_load }`: the
    /// seller's surplus over its own target, up to what the buyer lacks of its
    /// target (at most `max_load` units), so piles flow to shortages.
    #[serde(default)]
    pub caravans: Caravans,
    /// Fraction of price lost per unit traded per 1000 units of road (transport cost).
    pub transport_cost: f32,
    /// Yearly tax on each agent's wealth above `tax_free_wealth`, paid to the town
    /// treasury. It returns savings to the market so wages can keep flowing.
    pub wealth_tax: f32,
    pub tax_free_wealth: f32,
    /// Treasury a town keeps per resident; above it, `public_spending` of the excess
    /// per year is paid out to residents, so towns don't hoard money either.
    pub treasury_reserve_per_capita: f32,
    pub public_spending: f32,
    /// Fraction of food stock that spoils per year.
    pub food_spoilage: f32,
    /// Spoilage in a granary, as a share of the market's.
    #[serde(default = "granary_spoilage")]
    pub granary_spoilage: f32,
    /// Guild members may buy a tool on credit until their wealth reaches minus this.
    pub tool_credit: f32,
    /// Coins each town starts with in its treasury.
    #[serde(default = "default_starting_coins")]
    pub starting_coins: f32,
    /// Whether a treasury can pay out more of its own coin than it holds.
    #[serde(default)]
    pub treasury: Treasury,
    /// A town on automatic minting builds a mint and strikes coin while its treasury
    /// holds less than this per resident.
    #[serde(default = "default_mint_below")]
    pub mint_below_per_capita: f32,
    /// A money-supply rule for the mint (auto): it also strikes coin while the
    /// town's whole money stock (treasury, residents, firms) per resident is
    /// below this. 0: off.
    #[serde(default)]
    pub mint_money_per_capita: f32,
    /// Emergency food imports (see `policy::Imports`).
    #[serde(default)]
    pub imports: ImportParams,
    /// Share of a building type's slots in use above which the town wants another.
    pub build_threshold: f32,
    /// Price, as a share of base price, a town offers for a building type it lacks.
    pub new_building_appeal: f32,
    /// What a town pays for another building of a type it has.
    #[serde(default)]
    pub building_value: BuildingValue,
    /// Plots of new land (fields, woodlots) each town can still open up.
    pub free_land: u32,
    /// A town opens new land when its sites of a kind are this depleted on average.
    pub land_threshold: f32,
}

fn granary_spoilage() -> f32 {
    0.25
}

/// How caravans decide their load (`market.caravans`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Caravans {
    #[default]
    Fixed,
    Balance {
        max_load: f32,
    },
}

/// `market.treasury` in models.ron: what a treasury can pay in its own coin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Treasury {
    /// Only what it holds: an empty treasury pays no wages.
    Holds,
    /// The town issues its own coin, so its treasury pays its own people
    /// (wages, subsidies, soldiers, schooling, its own market's purchases)
    /// in full and may go below zero. What limits it is trust: more coin
    /// in people's hands raises prices, and the coin falls abroad. Paying
    /// other towns still takes coin it holds.
    #[default]
    Issues,
}
