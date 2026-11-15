//! Scenarios (`assets/config/scenario.ron`): towns, terrain and the goal.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub seed: u64,
    /// The town the player governs; none for a pure simulation.
    #[serde(default)]
    pub player: Option<String>,
    pub towns: Vec<TownSpec>,
    /// Hills and rivers; flat open ground when left out.
    #[serde(default)]
    pub terrain: Option<TerrainSpec>,
    /// How the match is won; none for endless play.
    #[serde(default)]
    pub goal: Option<GoalSpec>,
    /// Generate the towns and terrain from the seed instead (see `worldgen`).
    #[serde(default)]
    pub generate: Option<crate::worldgen::GenSpec>,
}

/// How a match is won (see [`crate::goal`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalSpec {
    /// Product id to make, e.g. "longsword_3".
    pub product: String,
    /// How many a town must have made in total.
    pub amount: f32,
    /// Most of a year's person-days a winning town may spend starving (0.02 = 2%).
    #[serde(default = "default_max_starving")]
    pub max_starving: f32,
    /// The match ends after this many years; then the town closest to the goal wins.
    pub years: u32,
}

fn default_max_starving() -> f32 {
    0.02
}

/// The lay of the land (see [`crate::terrain`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerrainSpec {
    /// Size of one grid cell.
    pub cell: f32,
    /// The map reaches at least this far from the origin; it always covers the towns.
    pub half_size: (f32, f32),
    /// Extra travel cost on the highest hills (1 = twice as slow).
    pub hill_cost: f32,
    /// Typical distance between hills.
    pub hill_scale: f32,
    pub rivers: Vec<RiverSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiverSpec {
    /// The river's course, as a polyline.
    pub points: Vec<(f32, f32)>,
    pub width: f32,
    /// Places where the river can be crossed.
    #[serde(default)]
    pub fords: Vec<(f32, f32)>,
    #[serde(default = "default_ford_width")]
    pub ford_width: f32,
    /// Travel cost per unit of distance at a ford.
    #[serde(default = "default_ford_cost")]
    pub ford_cost: f32,
}

fn default_ford_width() -> f32 {
    60.0
}

fn default_ford_cost() -> f32 {
    3.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// The personality of the town's AI government (a name from `governor.personalities`
    /// in models.ron); none is balanced.
    #[serde(default)]
    pub governor: Option<String>,
    /// Starting culture (one value 0..1 per ideological axis: tradition, faith,
    /// liberty). The first residents are drawn around it; none picks a distinct one.
    #[serde(default)]
    pub culture: Option<Vec<f32>>,
}
