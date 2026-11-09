//! Agent decision-making as swappable components (see docs/design/agent-decisions.md).
//!
//! Each component is a trait with a narrow interface, one or more implementations,
//! and its own params block under `decision` in `models.ron`:
//!
//! - [`expectations::Expectations`]: forecast a price or pay from what was observed.
//! - [`utility::Utility`]: how much an agent values the state of its needs.
//! - [`choice::ChoiceRule`]: how the values of the options become a decision.
//! - [`valuation`]: what working a job or resting is worth to an agent (the
//!   utility model's wiring of expectations and utility).
//!
//! Further components (learning value, choice rule, wage adjustment, job matching,
//! migration) follow the same pattern. A component sees only its inputs; the systems
//! that use it gather them.

pub mod choice;
pub mod expectations;
pub mod utility;
pub mod valuation;

use bevy_ecs::prelude::*;

/// The active decision components, built from the parameters.
#[derive(Resource)]
pub struct DecisionModels {
    pub expectations: Box<dyn expectations::Expectations>,
    pub utility: Box<dyn utility::Utility>,
    pub choice: Box<dyn choice::ChoiceRule>,
}

impl DecisionModels {
    pub fn from_params(p: &crate::config::DecisionParams) -> Self {
        DecisionModels {
            expectations: expectations::build(&p.expectations),
            utility: utility::build(&p.utility),
            choice: choice::build(&p.choice, p.temperature),
        }
    }
}
