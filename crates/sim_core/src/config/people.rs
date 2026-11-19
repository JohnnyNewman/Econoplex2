//! People: decisions, needs, body, life course, opinions, ties, migration and guilds.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionParams {
    /// Softmax temperature: higher = less rational choices.
    pub temperature: f32,
    /// How many candidate jobs each agent considers per decision (sampled), plus resting.
    pub candidates: usize,
    /// Scale from satisfaction per day to the choice rule's units (how
    /// sharply people pick the best option).
    #[serde(default = "ladder_scale")]
    pub ladder_scale: f32,
    /// Component: how agents form price and pay expectations.
    #[serde(default)]
    pub expectations: crate::decision::expectations::ExpectationsParams,
    /// Component: how agents value the state of their needs.
    #[serde(default)]
    pub utility: crate::decision::utility::UtilityParams,
    /// Component: how option values become a choice.
    #[serde(default)]
    pub choice: crate::decision::choice::ChoiceParams,
    /// Look-ahead and discounting, the typical success chance, and posted pay.
    #[serde(default)]
    pub valuation: crate::decision::valuation::ValuationParams,
    /// Component: how each job's pay moves with the balance of workers and places.
    #[serde(default)]
    pub wages: crate::decision::wages::WageParams,
    /// Component: the value of the skill a job teaches.
    #[serde(default)]
    pub learning: crate::decision::learning::LearningValueParams,
    /// Component: who gets which place, and apprentice places.
    #[serde(default)]
    pub matching: crate::decision::matching::MatchingParams,
    /// Component: what living in another town is worth to an agent.
    #[serde(default)]
    pub migration: crate::decision::migration::MigrationValueParams,
    /// Component: food workers' households keep part of what they make.
    #[serde(default)]
    pub subsistence: crate::decision::subsistence::SubsistenceParams,
    /// Component: the rent a town's crowding costs its residents.
    #[serde(default)]
    pub crowding: crate::decision::crowding::CrowdingParams,
    /// How towns' culture vectors follow their residents.
    #[serde(default)]
    pub culture: crate::decision::culture::CultureParams,
    /// Component: which towns agents know about, and how well.
    #[serde(default)]
    pub knowledge: crate::decision::knowledge::KnowledgeParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneticsParams {
    /// 1 = child takes each gene from one parent; 0 = child gets the parents' mean.
    pub heritability: f32,
    pub mutation: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifeParams {
    pub adult_age: f32,
    pub apprentice_age: f32,
    pub min_birth_age: f32,
    pub max_birth_age: f32,
    pub births_per_year: f32,
    /// Gompertz mortality: yearly hazard = a * exp(b * age).
    pub mortality_a: f32,
    pub mortality_b: f32,
    pub partner_min_score: f32,
    /// Months (simulated days) between rounds of partnering.
    pub partner_interval: u32,
    /// Food stock per capita at which births run at full rate.
    pub food_for_births: f32,
    /// Parents judge the food stock this many days ahead at its recent trend, so
    /// births slow while stores are still full but shrinking.
    pub food_foresight_days: f32,
    /// How much more likely founders start in a trade per unit of its food output.
    pub starting_food_bias: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizeParams {
    /// Months (simulated days) between rounds of trades and guilds.
    pub interval: u32,
    /// Skill at which an adult with no trade joins the guild of their best
    /// skill (someone who never apprenticed).
    pub guild_threshold: f32,
    /// Apprentices one master may take at once (a hard limit).
    #[serde(default = "apprentices_per_master")]
    pub apprentices_per_master: u32,
    /// Years in a trade after which an artisan is a master.
    #[serde(default = "master_years")]
    pub master_years: f32,
    /// How much less each learner gets from a teacher for every other learner
    /// beside them: a teacher's lesson is shared, 1 / (1 + crowding x (n - 1)).
    #[serde(default = "teaching_crowding")]
    pub teaching_crowding: f32,
}

fn apprentices_per_master() -> u32 {
    3
}

fn master_years() -> f32 {
    8.0
}

fn teaching_crowding() -> f32 {
    0.3
}

/// Personal ties between agents (`trust.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MigrationParams {
    /// Months (simulated days) between migration rounds.
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
    /// Weight of cultural fit with the town (see `decision::culture`).
    #[serde(alias = "ideology_weight")]
    pub culture_weight: f32,
    /// Utility lost for a town raided this year.
    pub danger_weight: f32,
    /// Utility lost per 1000 map units of road.
    pub distance_cost: f32,
    /// Trust one family member (parent or child) living in a town counts as, for
    /// the social network there.
    #[serde(default = "kin_tie_default")]
    pub kin_tie: f32,
    /// Soldiers per resident at which a town counts as fully guarded (for safety).
    #[serde(default = "guard_share_default")]
    pub guard_share: f32,
    /// How much a dependent child's gain counts in its household's decision to
    /// move (adults count 1).
    #[serde(default = "child_weight_default")]
    pub child_weight: f32,
}

fn child_weight_default() -> f32 {
    1.0
}

fn kin_tie_default() -> f32 {
    0.5
}

fn guard_share_default() -> f32 {
    0.05
}

fn ladder_scale() -> f32 {
    200.0
}
