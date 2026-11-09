//! Agent decision-making as swappable components (see docs/design/agent-decisions.md).
//!
//! Each component is a trait with a narrow interface, one or more implementations,
//! and its own params block under `decision` in `models.ron`:
//!
//! - [`expectations::Expectations`]: forecast a price or pay from what was observed.
//! - [`utility::Utility`]: how much an agent values the state of its needs.
//! - [`choice::ChoiceRule`]: how the values of the options become a decision.
//! - [`wages::WageAdjustment`]: how a job's pay moves with workers and places.
//! - [`learning::LearningValue`]: what the skill a job teaches is worth later.
//! - [`matching`]: who gets which place, with apprentice places.
//! - [`migration::Migration`]: what living in another town is worth.
//! - [`subsistence`]: how much of their food output workers' households keep.
//! - [`crowding`]: the rent a town's crowding costs its residents.
//! - [`culture`]: each town's culture vector and an agent's fit with it.
//! - [`valuation`]: what working a job or resting is worth to an agent (the
//!   utility model's wiring of expectations and utility).
//!
//! Further components (learning value, choice rule, wage adjustment, job matching,
//! migration) follow the same pattern. A component sees only its inputs; the systems
//! that use it gather them.

pub mod choice;
pub mod crowding;
pub mod culture;
pub mod expectations;
pub mod learning;
pub mod matching;
pub mod migration;
pub mod subsistence;
pub mod utility;
pub mod valuation;
pub mod wages;

use bevy_ecs::prelude::*;

/// The active decision components, built from the parameters.
#[derive(Resource)]
pub struct DecisionModels {
    pub expectations: Box<dyn expectations::Expectations>,
    pub utility: Box<dyn utility::Utility>,
    pub choice: Box<dyn choice::ChoiceRule>,
    pub wages: Box<dyn wages::WageAdjustment>,
    pub learning: Box<dyn learning::LearningValue>,
    pub migration: Box<dyn migration::Migration>,
}

impl DecisionModels {
    pub fn from_model_params(m: &crate::config::ModelParams) -> Self {
        let mut d = Self::from_params(&m.decision);
        d.migration = migration::build(
            &m.decision.migration,
            &m.migration,
            m.trust.migration_weight,
        );
        d
    }

    pub fn from_params(p: &crate::config::DecisionParams) -> Self {
        DecisionModels {
            expectations: expectations::build(&p.expectations),
            utility: utility::build(&p.utility),
            choice: choice::build(&p.choice, p.temperature),
            wages: wages::build(&p.wages),
            learning: learning::build(&p.learning),
            migration: migration::build(
                &p.migration,
                &crate::config::MigrationParams::default(),
                0.0,
            ),
        }
    }
}
