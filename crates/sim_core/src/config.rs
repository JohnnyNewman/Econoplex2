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
    pub military: MilitaryParams,
    pub migration: MigrationParams,
    pub trust: TrustParams,
    pub policy: PolicyParams,
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
    /// Parents judge the food stock this many days ahead at its recent trend, so
    /// births slow while stores are still full but shrinking.
    pub food_foresight_days: f32,
    /// How much more likely founders start in a trade per unit of its food output.
    pub starting_food_bias: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketParams {
    pub price_adjust: f32,
    pub target_per_capita: f32,
    pub food_target_per_capita: f32,
    /// Nobody makes a good once the town holds this many times its target stock.
    pub glut_factor: f32,
    /// Days of the town's food need whose inputs (grain, flour, wood) are kept for
    /// the mills and bakeries, out of reach of eating and of other trades.
    pub food_reserve_days: f32,
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
    /// Share of a building type's slots in use above which the town wants another.
    pub build_threshold: f32,
    /// Price, as a share of base price, a town offers for a building type it lacks.
    pub new_building_appeal: f32,
    /// Plots of new land (fields, woodlots) each town can still open up.
    pub free_land: u32,
    /// A town opens new land when its sites of a kind are this depleted on average.
    pub land_threshold: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrganizeParams {
    pub interval: u32,
    pub guild_threshold: f32,
    pub master_threshold: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MilitaryParams {
    /// Days between musters (recruiting, releasing, arming soldiers).
    pub muster_interval: u32,
    /// Share of a town's adults it keeps under arms when it can pay them.
    pub soldier_share: f32,
    /// Daily pay per soldier, from the town treasury.
    pub soldier_pay: f32,
    /// Oldest age at which adults are recruited.
    pub max_recruit_age: f32,
    /// Learning multiplier for daily drill and for battle.
    pub drill_rate: f32,
    pub battle_learning: f32,
    /// Weapon power per tier at quality 0.5.
    pub weapon_power: f32,
    /// Fighting strength of each adult at home who is not a soldier.
    pub militia_power: f32,
    /// Map units a squad marches per day.
    pub march_speed: f32,
    /// Days a town waits after a raid before the next one.
    pub raid_cooldown: u32,
    /// A town raids only when its squad is this many times stronger than the defense.
    pub raid_margin: f32,
    /// Daily chance that a town able to raid decides to, scaled by its soldiers' ambition and risk.
    pub raid_appetite: f32,
    pub min_raid_squad: usize,
    /// Win chance = A^k / (A^k + D^k).
    pub battle_steepness: f32,
    /// Base chance a fighter falls in battle (scaled by the enemy's share of strength).
    pub casualty_rate: f32,
    /// Share of the defender's treasury and stores a winning raid carries off.
    pub loot_share: f32,
    /// Share of the defender's idle adults a winning raid takes captive,
    /// at most `captives_per_soldier` per surviving soldier.
    pub captive_share: f32,
    pub captives_per_soldier: f32,
    /// A winning raid conquers the defender when the attackers were this many
    /// times stronger than the defense.
    pub conquest_margin: f32,
    /// Days between tribute payments, and the share of its treasury a conquered
    /// town pays its ruler each time.
    pub tribute_interval: u32,
    pub tribute_share: f32,
    /// Share of its adults a conquered town may keep under arms.
    pub vassal_soldier_share: f32,
    /// Strength of the ruler's soldiers at home that helps defend a conquered town.
    pub ruler_aid: f32,
    /// A tributary rises up when its own defense beats its ruler's squad strength
    /// times this, shared among all the ruler's tributaries.
    pub garrison_strength: f32,
    /// Yearly chance a conquered town regains its independence peacefully.
    pub independence_rate: f32,
}

/// Player levers (`policy.rs`).
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyParams {
    /// Highest subsidy, as a share of the product's market value.
    pub max_subsidy: f32,
    /// Children this old and up to `life.apprentice_age` go to school.
    pub school_age: f32,
    /// Speed of school learning relative to learning on the job from a master.
    pub school_rate: f32,
    /// Paid by the treasury to the teacher per pupil and day.
    pub school_fee: f32,
    /// Pull of a chartered guild's trade in job choice (as being a member is 1).
    pub charter_pull: f32,
    /// Extra teaching speed of a chartered guild's masters (1.0 doubles it).
    pub charter_teaching: f32,
    /// Appeal a town with encouraged immigration adds for migrants.
    pub immigration_bonus: f32,
    /// Paid by the treasury to each newcomer under encouraged immigration.
    pub settlement_grant: f32,
    /// Highest share of adults a town may keep under arms.
    pub max_army_share: f32,
}

/// Personal ties between agents (`trust.rs`).
#[derive(Debug, Clone, Deserialize)]
pub struct TrustParams {
    /// Trust gained by each pair of coworkers when a job succeeds.
    pub cowork_gain: f32,
    /// Share of that gain when the job fails.
    pub failure_factor: f32,
    /// Trust change from a chance meeting: +gain between like minds, -gain between opposites.
    pub meet_gain: f32,
    /// Share of trust lost per day without contact.
    pub fade: f32,
    /// Added to a team's proficiency at full cohesion (lower coordination costs).
    pub team_bonus: f32,
    /// Extra learning from a fully trusted master (1.0 doubles it).
    pub teaching_bonus: f32,
    /// Extra fighting power of a squad at full cohesion.
    pub loyalty: f32,
    /// Weight in the migration utility of trusted people living in a town.
    pub migration_weight: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MigrationParams {
    /// Days between migration rounds.
    pub interval: u32,
    /// Chance an idle adult weighs moving in a round.
    pub consider: f32,
    /// Utility a better town must offer before anyone leaves home.
    pub min_gain: f32,
    /// Oldest age at which people still move.
    pub max_age: f32,
    /// Weight of food per capita (relative to `food_reference`, capped at 2).
    pub food_weight: f32,
    pub food_reference: f32,
    /// Weight of what the agent's own trade sells for there, relative to the
    /// average over towns.
    pub trade_weight: f32,
    /// Weight of ideological closeness to the town's mean.
    pub ideology_weight: f32,
    /// Utility lost for a town raided this year.
    pub danger_weight: f32,
    /// Utility lost per 1000 map units of road.
    pub distance_cost: f32,
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
    /// The town the player governs; none for a pure simulation.
    #[serde(default)]
    pub player: Option<String>,
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

/// Replace one value in a RON document, addressed by a dotted path such as
/// `military.tribute_share` or `success.steepness` (fields of an enum variant
/// are reached the same way). Comments and formatting elsewhere are kept, so
/// the patched text still parses with the same types; used for `--set` and
/// parameter sweeps.
pub fn set_ron_field(text: &str, path: &str, value: &str) -> Result<String, ConfigError> {
    let b = text.as_bytes();
    let missing = || ConfigError(format!("no field `{path}` in the parameter file"));
    // The document itself is one parenthesized struct.
    let mut scope = inner(b, 0, b.len()).ok_or_else(missing)?;
    let keys: Vec<&str> = path.split('.').collect();
    for (n, key) in keys.iter().enumerate() {
        let span = find_field(b, scope, key).ok_or_else(missing)?;
        if n + 1 == keys.len() {
            return Ok(format!("{}{}{}", &text[..span.0], value, &text[span.1..]));
        }
        scope = inner(b, span.0, span.1).ok_or_else(missing)?;
    }
    Err(missing())
}

/// Skip a comment or string starting at `i`; returns the index after it.
fn skip_trivia(b: &[u8], i: usize) -> Option<usize> {
    match (b[i], b.get(i + 1)) {
        (b'/', Some(b'/')) => Some(
            b[i..]
                .iter()
                .position(|&c| c == b'\n')
                .map_or(b.len(), |p| i + p),
        ),
        (b'/', Some(b'*')) => Some(
            b[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(b.len(), |p| i + 2 + p + 2),
        ),
        (b'"', _) => {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            Some(j + 1)
        }
        _ => None,
    }
}

/// The span inside the first `(` ... matching `)` within `[from, to)`.
fn inner(b: &[u8], from: usize, to: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i < to {
        if let Some(j) = skip_trivia(b, i) {
            i = j;
            continue;
        }
        if b[i] == b'(' {
            // Walk value by value to the closing bracket.
            let mut end = value_end(b, i + 1, to);
            while end < to && b[end] == b',' {
                end = value_end(b, end + 1, to);
            }
            return Some((i + 1, end));
        }
        i += 1;
    }
    None
}

/// Index of the `,` or closing bracket that ends a value starting at `i`.
fn value_end(b: &[u8], mut i: usize, to: usize) -> usize {
    let mut depth = 0i32;
    while i < to {
        if let Some(j) = skip_trivia(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => return i,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    to
}

/// The value span of `key: value` directly inside `scope` (not nested deeper).
fn find_field(b: &[u8], scope: (usize, usize), key: &str) -> Option<(usize, usize)> {
    let (mut i, to) = scope;
    let mut depth = 0i32;
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < to {
        if let Some(j) = skip_trivia(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            c if depth == 0 && ident(c) && (i == 0 || !ident(b[i - 1])) => {
                let mut j = i;
                while j < to && ident(b[j]) {
                    j += 1;
                }
                let mut k = j;
                while k < to && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                if &b[i..j] == key.as_bytes() && k < to && b[k] == b':' {
                    let mut s = k + 1;
                    while s < to && b[s].is_ascii_whitespace() {
                        s += 1;
                    }
                    let mut e = value_end(b, s, to);
                    while e > s && b[e - 1].is_ascii_whitespace() {
                        e -= 1;
                    }
                    return Some((s, e));
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::set_ron_field;

    #[test]
    fn patches_nested_and_variant_fields() {
        let text = "// c: 1\n(\n  a: (x: 1.0, y: [1, 2]), // x: 9\n  s: Sigmoid(steepness: 8.0),\n  x: 3,\n)";
        let t = set_ron_field(text, "a.x", "2.5").unwrap();
        assert!(t.contains("a: (x: 2.5, y: [1, 2])"));
        let t = set_ron_field(&t, "s.steepness", "4.0").unwrap();
        assert!(t.contains("Sigmoid(steepness: 4.0)"));
        let t = set_ron_field(&t, "x", "7").unwrap();
        assert!(t.contains("  x: 7,"));
        assert!(set_ron_field(text, "a.z", "1").is_err());
        assert!(set_ron_field(text, "y", "1").is_err());
    }
}
