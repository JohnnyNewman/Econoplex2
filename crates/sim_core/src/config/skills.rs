//! Skills, teams and learning: the capability embedding and how teams work.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingParams {
    /// Weight of an ancestor skill per level of depth in the closure vector.
    pub ancestor_decay: f32,
    /// `Auto`, `Closure` or `Nmf`.
    pub method: String,
    /// Capability dimensions. Leave out to size them from the content: one per
    /// skill, rounded up to a multiple of 8. Fewer dimensions than skills compress
    /// the product space (NMF).
    #[serde(default)]
    pub dims: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TeamPoolingParams {
    /// Best member per capability dimension, minus a cost per extra member.
    ElementwiseMax { coordination_cost: f32 },
    /// Best member plus `assist` times the sum of the others.
    LeaderPlusAssistants { assist: f32, coordination_cost: f32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuccessParams {
    /// p = sigmoid(steepness * (effective - difficulty))
    Sigmoid { steepness: f32 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum QualityCombine {
    /// Weakest link: final quality is the worst of step and inputs.
    Min,
    GeometricMean,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QualityParams {
    /// Step quality = sigmoid(steepness * (effective - difficulty) + offset), combined with input quality.
    Margin {
        steepness: f32,
        offset: f32,
        combine: QualityCombine,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LearningParams {
    /// a += rate * aptitude * max(0, 1 - a·s / mastery) * s; failure teaches `failure_factor` as much.
    Diminishing {
        rate: f32,
        mastery: f32,
        failure_factor: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiffusionParams {
    /// Apprentice moves toward the best team member along the skill direction only.
    AlongSkill { rate: f32, teacher_gain: f32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CapacityParams {
    /// Daily multiplicative forgetting, then renormalize to at most `budget`.
    NormBudget { budget: f32, forgetting: f32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkParams {
    /// Team work rate per day = members ^ exponent (below 1 = diminishing returns).
    pub work_exponent: f32,
    /// Share of the output's market value paid to the workers.
    pub wage_share: f32,
}
