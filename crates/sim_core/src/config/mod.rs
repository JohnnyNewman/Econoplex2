//! Tunable parameters (`assets/config/models.ron`) and scenarios (`assets/config/scenario.ron`).
//!
//! Every formula in the simulation reads its constants from here. Formulas with
//! alternative shapes are enums: pick a variant in the RON file to swap the model.
//! The parameters are grouped by system in the submodules; everything is
//! re-exported here, so paths stay `crate::config::X`.

mod government;
mod market;
mod military;
mod people;
mod ron_patch;
mod scenario;
mod skills;

pub use government::*;
pub use market::*;
pub use military::*;
pub use people::*;
pub use ron_patch::*;
pub use scenario::*;
pub use skills::*;

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Squad battles on a field (`battle.rs`), fought by raids.
    #[serde(default)]
    pub battle: crate::battle::BattleParams,
    pub migration: MigrationParams,
    pub trust: TrustParams,
    pub policy: PolicyParams,
    /// AI town governments (`governor.rs`).
    #[serde(default)]
    pub governor: GovernorParams,
    /// Ticks per day, days per month, months per year (`env.rs`).
    #[serde(default)]
    pub time: crate::env::TimeParams,
    /// Average weather through the year (`env.rs`).
    #[serde(default)]
    pub climate: crate::env::ClimateParams,
    /// How the weather varies around the climate (`env.rs`).
    #[serde(default)]
    pub weather: crate::env::WeatherParams,
    /// Crops through the seasons (`farming.rs`).
    #[serde(default)]
    pub farming: crate::farming::FarmingParams,
}

impl ModelParams {
    /// Check the parameters so a typo in the RON file fails loudly at load time.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let t = &self.time;
        if t.ticks_per_day == 0
            || t.days_per_month == 0
            || t.months_per_year == 0
            || t.economy_days_per_day == 0
        {
            return Err(ConfigError(
                "time: ticks_per_day, days_per_month, months_per_year and economy_days_per_day must be at least 1".into(),
            ));
        }
        if t.days_per_year() != self.life.days_per_year {
            return Err(ConfigError(format!(
                "time: {} days per month times {} months times {} economy days per day is {} days, but life.days_per_year is {}",
                t.days_per_month,
                t.months_per_year,
                t.economy_days_per_day,
                t.days_per_year(),
                self.life.days_per_year
            )));
        }
        Ok(())
    }
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
