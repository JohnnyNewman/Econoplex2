//! Squads and raids (`military` in models.ron).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilitaryParams {
    /// Months (simulated days) between musters (recruiting, releasing, arming
    /// soldiers).
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
    /// Months (simulated days) a town waits after a raid before the next one.
    pub raid_cooldown: u32,
    pub min_raid_squad: usize,
    /// A tenth of it is the chance each civilian of a town whose defense
    /// breaks is hurt.
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
    /// Months (simulated days) between tribute payments, and the share of its treasury a conquered
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
    /// Hours of use a battle puts on every fighter's kit (`goods::wear`).
    #[serde(default = "default_battle_hours")]
    pub battle_hours: f32,
    /// Share of its residents a town keeps kit for beyond its soldiers, to arm
    /// a levy. A town keeps in store its `stockpile` target for each piece of
    /// kit, or by default one piece per soldier it means to keep plus this
    /// levy reserve, shared among the goods that fill the same slot (see
    /// `military::kit_reserve`). Also how many civilians take up arms when the
    /// town is attacked.
    #[serde(default)]
    pub levy_reserve: f32,
    /// Rations a squad takes along, as a multiple of what the trip there and
    /// back should take.
    #[serde(default = "default_ration_margin")]
    pub ration_margin: f32,
}

fn default_ration_margin() -> f32 {
    1.25
}

fn default_battle_hours() -> f32 {
    4.0
}
