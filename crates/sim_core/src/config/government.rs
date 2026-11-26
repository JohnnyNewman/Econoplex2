//! Player levers (`policy`) and AI town governments (`governor`).

use serde::{Deserialize, Serialize};

/// Player levers (`policy.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyParams {
    /// Highest subsidy, as a share of the product's market value.
    pub max_subsidy: f32,
    /// Children this old and up to `life.apprentice_age` go to school.
    pub school_age: f32,
    /// Speed of school learning relative to learning on the job from a master.
    pub school_rate: f32,
    /// Paid by the treasury to the teacher per pupil and day.
    pub school_fee: f32,
    /// Extra teaching speed of a chartered guild's masters (1.0 doubles it).
    pub charter_teaching: f32,
    /// Appeal a town with encouraged immigration adds for migrants.
    pub immigration_bonus: f32,
    /// Paid by the treasury to each newcomer under encouraged immigration.
    pub settlement_grant: f32,
    /// Highest share of adults a town may keep under arms.
    pub max_army_share: f32,
    /// Bounds of the economic levers (`tax`, `wages`, `spending` orders).
    #[serde(default = "default_max_wealth_tax")]
    pub max_wealth_tax: f32,
    #[serde(default = "default_max_income_tax")]
    pub max_income_tax: f32,
    #[serde(default = "default_min_wage_share")]
    pub min_wage_share: f32,
    #[serde(default = "default_max_wage_share")]
    pub max_wage_share: f32,
    #[serde(default = "default_max_public_spending")]
    pub max_public_spending: f32,
}

fn default_max_wealth_tax() -> f32 {
    3.0
}
fn default_max_income_tax() -> f32 {
    0.6
}
fn default_min_wage_share() -> f32 {
    0.1
}
fn default_max_wage_share() -> f32 {
    0.9
}
fn default_max_public_spending() -> f32 {
    12.0
}

/// Rules of the AI town governments (see `governor.rs`). Thresholds are per resident
/// or relative to the market's targets; steps are how far a lever moves per decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GovernorParams {
    pub enabled: bool,
    /// Months (simulated days) between decisions.
    pub interval: u32,
    /// Days over which observations are averaged.
    pub smoothing_days: f32,
    /// Also govern the player's town (for headless comparisons).
    pub govern_player_town: bool,
    /// Food per resident, as a share of the market's food target, below which the
    /// town subsidizes food, and above which it winds the subsidy down.
    pub food_low: f32,
    pub food_ok: f32,
    pub subsidy_step: f32,
    pub max_subsidy: f32,
    /// A good the town makes counts as short at this price / base price, and as
    /// recovered at the second.
    pub shortage_ratio: f32,
    pub recovered_ratio: f32,
    /// Most non-food goods subsidized at once.
    pub max_subsidized: usize,
    /// Treasury per resident below which taxes rise, and above which they fall and
    /// spending rises.
    pub treasury_low: f32,
    pub treasury_high: f32,
    /// When the town issues its own coin: taxes rise once what households
    /// buy costs `prices_high` times what it did at the start, and fall
    /// below `prices_low` times.
    #[serde(default = "prices_high")]
    pub prices_high: f32,
    #[serde(default = "prices_low")]
    pub prices_low: f32,
    pub tax_step: f32,
    pub income_tax_step: f32,
    pub max_income_tax: f32,
    pub spending_step: f32,
    /// Highest spending rate, as a multiple of the default.
    pub max_spending_factor: f32,
    /// Decisions in a row with a low treasury before the mint is set to strike.
    pub mint_after_checks: u32,
    /// Share of adults idle above which the town orders public works.
    pub idle_high: f32,
    /// Goods ordered for public works (the first one the town makes) and how many.
    pub public_works: Vec<String>,
    pub works_amount: f32,
    /// Named personalities a town can have.
    pub personalities: Vec<(String, crate::governor::Personality)>,
    /// Loans to firms (with `market.firms: Workshops`): a governor lends
    /// `loan_amount` at `loan_rate` over `loan_days` to a firm short of cash in a
    /// trade whose goods sell at `shortage_ratio` of base price or more, while
    /// the treasury is above its high mark and total lending stays under
    /// `max_loan_share` of the treasury.
    pub loan_amount: f32,
    pub loan_rate: f32,
    pub loan_days: u32,
    pub max_loan_share: f32,
    /// Highest profit tax a governor sets (steps of `income_tax_step`).
    pub max_profit_tax: f32,
    /// Days of food the granary aims to hold.
    #[serde(default = "granary_days")]
    pub granary_days: f32,
    /// Days over which food made and eaten are averaged for the forecast.
    #[serde(default = "forecast_days")]
    pub forecast_days: f32,
    /// A governor treats the town as hungry while its food in store would
    /// last fewer days than this at the forecast's rate.
    #[serde(default = "food_horizon_days")]
    pub food_horizon_days: f32,
}

fn forecast_days() -> f32 {
    120.0
}

fn food_horizon_days() -> f32 {
    90.0
}

fn granary_days() -> f32 {
    40.0
}

impl Default for GovernorParams {
    fn default() -> Self {
        GovernorParams {
            enabled: true,
            interval: 3,
            smoothing_days: 30.0,
            govern_player_town: false,
            food_low: 0.5,
            food_ok: 1.0,
            subsidy_step: 0.2,
            max_subsidy: 0.6,
            shortage_ratio: 3.0,
            recovered_ratio: 1.3,
            max_subsidized: 3,
            treasury_low: 1.0,
            treasury_high: 2.4,
            prices_high: prices_high(),
            prices_low: prices_low(),
            tax_step: 0.25,
            income_tax_step: 0.05,
            max_income_tax: 0.25,
            spending_step: 0.5,
            max_spending_factor: 2.0,
            mint_after_checks: 3,
            idle_high: 0.15,
            public_works: vec!["cut_stone".into(), "brick".into(), "plank".into()],
            works_amount: 60.0,
            loan_amount: 100.0,
            loan_rate: 0.1,
            loan_days: 240,
            max_loan_share: 0.3,
            max_profit_tax: 0.5,
            granary_days: granary_days(),
            forecast_days: forecast_days(),
            food_horizon_days: food_horizon_days(),
            personalities: vec![
                ("balanced".into(), crate::governor::Personality::default()),
                (
                    "frugal".into(),
                    crate::governor::Personality {
                        thrift: 1.5,
                        ambition: 0.6,
                    },
                ),
                (
                    "expansionist".into(),
                    crate::governor::Personality {
                        thrift: 0.7,
                        ambition: 1.5,
                    },
                ),
            ],
        }
    }
}

fn prices_high() -> f32 {
    1.3
}

fn prices_low() -> f32 {
    1.1
}
