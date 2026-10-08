//! Tunable parameters (`assets/config/models.ron`) and scenarios (`assets/config/scenario.ron`).
//!
//! Every formula in the simulation reads its constants from here. Formulas with
//! alternative shapes are enums: pick a variant in the RON file to swap the model.

use serde::Deserialize;
use sim_data::dims::{MIND_DIM, STATE_DIM};
use std::path::Path;

/// Number of decision features: value, skill, effort, novelty, social, need.
pub const FEATURES: usize = 6;
pub const FEATURE_NAMES: [&str; FEATURES] =
    ["value", "skill", "effort", "novelty", "social", "need"];

#[derive(Debug, Clone, Deserialize)]
pub struct ModelParams {
    pub embedding: EmbeddingParams,
    pub team_pooling: TeamPoolingParams,
    pub success: SuccessParams,
    pub quality: QualityParams,
    pub learning: LearningParams,
    pub diffusion: DiffusionParams,
    pub capacity: CapacityParams,
    pub decision: DecisionParams,
    pub social: SocialParams,
    pub needs: NeedsParams,
    pub physique: PhysiqueParams,
    pub genetics: GeneticsParams,
    pub life: LifeParams,
    pub market: MarketParams,
    pub organize: OrganizeParams,
    pub work: WorkParams,
}

impl ModelParams {
    /// Check matrix shapes so a typo in the RON file fails loudly at load time.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let d = &self.decision;
        let shape = |name: &str, rows: &[Vec<f32>], cols: usize| -> Result<(), ConfigError> {
            if rows.len() != FEATURES || rows.iter().any(|r| r.len() != cols) {
                return Err(ConfigError(format!(
                    "decision.{name} must be {FEATURES} rows of {cols} values"
                )));
            }
            Ok(())
        };
        if d.bias.len() != FEATURES {
            return Err(ConfigError(format!(
                "decision.bias must have {FEATURES} values"
            )));
        }
        shape("mind_weights", &d.mind_weights, MIND_DIM)?;
        shape("state_weights", &d.state_weights, STATE_DIM)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingParams {
    /// Weight of an ancestor skill per level of depth in the closure vector.
    pub ancestor_decay: f32,
    /// `Auto`, `Closure` or `Nmf`.
    pub method: String,
}

#[derive(Debug, Clone, Deserialize)]
pub enum TeamPoolingParams {
    /// Best member per capability dimension, minus a cost per extra member.
    ElementwiseMax { coordination_cost: f32 },
    /// Best member plus `assist` times the sum of the others.
    LeaderPlusAssistants { assist: f32, coordination_cost: f32 },
}

#[derive(Debug, Clone, Deserialize)]
pub enum SuccessParams {
    /// p = sigmoid(steepness * (effective - difficulty))
    Sigmoid { steepness: f32 },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub enum QualityCombine {
    /// Weakest link: final quality is the worst of step and inputs.
    Min,
    GeometricMean,
}

#[derive(Debug, Clone, Deserialize)]
pub enum QualityParams {
    /// Step quality = sigmoid(steepness * (effective - difficulty) + offset), combined with input quality.
    Margin {
        steepness: f32,
        offset: f32,
        combine: QualityCombine,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub enum LearningParams {
    /// a += rate * aptitude * max(0, 1 - a·s / mastery) * s; failure teaches `failure_factor` as much.
    Diminishing {
        rate: f32,
        mastery: f32,
        failure_factor: f32,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub enum DiffusionParams {
    /// Apprentice moves toward the best team member along the skill direction only.
    AlongSkill { rate: f32, teacher_gain: f32 },
}

#[derive(Debug, Clone, Deserialize)]
pub enum CapacityParams {
    /// Daily multiplicative forgetting, then renormalize to at most `budget`.
    NormBudget { budget: f32, forgetting: f32 },
}

#[derive(Debug, Clone, Deserialize)]
pub struct DecisionParams {
    /// Softmax temperature: higher = less rational choices.
    pub temperature: f32,
    /// Preference weights per feature: w = bias + mind_weights · mind + state_weights · state.
    /// One entry per feature.
    pub bias: Vec<f32>,
    /// One row per feature, one column per mind axis.
    pub mind_weights: Vec<Vec<f32>>,
    /// One row per feature, one column per state axis.
    pub state_weights: Vec<Vec<f32>>,
    /// Scale in `value = tanh(profit_per_day / value_scale)`.
    pub value_scale: f32,
    /// How many candidate jobs each agent considers per decision (sampled), plus resting.
    pub candidates: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub enum SocialParams {
    /// Deffuant-style bounded confidence on the ideological subspace of the mind vector.
    BoundedConfidence {
        rate: f32,
        confidence: f32,
        repulsion_distance: f32,
        repulsion: f32,
        noise: f32,
        /// Chance per day that an agent talks to a random townsperson (coworkers always interact).
        daily_contacts: f32,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct NeedsParams {
    pub hunger_per_day: f32,
    pub eat_threshold: f32,
    pub starvation_damage: f32,
    pub health_regen: f32,
    pub fatigue_per_work_day: f32,
    pub fatigue_rest_recovery: f32,
    pub fatigue_idle_recovery: f32,
    pub happiness_rate: f32,
    /// Effective proficiency multiplier: 1 - fatigue_penalty * fatigue.
    pub fatigue_penalty: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhysiqueParams {
    /// How strongly physique counts in effective proficiency, relative to the recipe's demand.
    pub weight: f32,
    pub train_rate: f32,
    pub disuse_rate: f32,
    /// Fraction of potential kept without any training.
    pub untrained_level: f32,
    pub peak_age: f32,
    pub decline_age: f32,
    pub decline_per_year: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneticsParams {
    /// 1 = child takes each gene from one parent; 0 = child gets the parents' mean.
    pub heritability: f32,
    pub mutation: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LifeParams {
    /// Simulation ticks per in-game year (1 tick = 1 day).
    pub days_per_year: u32,
    pub adult_age: f32,
    pub apprentice_age: f32,
    pub min_birth_age: f32,
    pub max_birth_age: f32,
    pub births_per_year: f32,
    /// Gompertz mortality: yearly hazard = a * exp(b * age).
    pub mortality_a: f32,
    pub mortality_b: f32,
    pub partner_min_score: f32,
    pub partner_interval: u32,
    /// Food stock per capita at which births run at full rate.
    pub food_for_births: f32,
    /// How much more likely founders start in a trade per unit of its food output.
    pub starting_food_bias: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketParams {
    pub price_adjust: f32,
    pub target_per_capita: f32,
    pub food_target_per_capita: f32,
    pub min_price_factor: f32,
    pub max_price_factor: f32,
    pub trade_interval: u32,
    pub trade_amount: f32,
    /// Fraction of price lost per unit traded (transport cost).
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
    /// Guild members may buy a tool on credit until their wealth reaches minus this.
    pub tool_credit: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrganizeParams {
    pub interval: u32,
    pub guild_threshold: f32,
    pub master_threshold: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkParams {
    /// Team work rate per day = members ^ exponent (below 1 = diminishing returns).
    pub work_exponent: f32,
    /// Share of the output's market value paid to the workers.
    pub wage_share: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Scenario {
    pub seed: u64,
    pub towns: Vec<TownSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TownSpec {
    pub name: String,
    pub position: (f32, f32),
    pub population: u32,
    /// (nature kind id, number of sites)
    pub nature: Vec<(String, u32)>,
    pub buildings: Vec<String>,
    /// Starting stock: (product id, quantity)
    #[serde(default)]
    pub stock: Vec<(String, f32)>,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ConfigError {}

pub fn load_ron<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("cannot read {}: {e}", path.display())))?;
    ron::from_str(&text).map_err(|e| ConfigError(format!("cannot parse {}: {e}", path.display())))
}
