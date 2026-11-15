//! What squads bring to a battle: their kit and their members.

use super::*;

/// What a squad's members carry. Later this comes from the items in their
/// inventories; for now the presets stand in for them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Kit {
    pub name: String,
    /// Melee: reach in metres, health taken per hit, strikes per second per
    /// fighter in the front rank, and the share of strikes lost once an enemy
    /// with a shorter weapon is inside the reach.
    pub reach: f32,
    pub damage: f32,
    pub strike_rate: f32,
    pub close_penalty: f32,
    /// Bows: range in metres, shots per second, health per hit, arrows per fighter.
    pub range: f32,
    pub shot_rate: f32,
    pub shot_damage: f32,
    pub arrows: u32,
    /// Share of a hit's damage armour stops, and share of arrows a shield stops.
    pub armour: f32,
    pub shield: f32,
    pub mounted: bool,
}

impl Kit {
    /// Kits for the unit types in the design note. Numbers are physical guesses:
    /// a 3 m spear, a 1.2 m sword, a 150 m bow with 24 arrows, mail stopping half.
    pub fn preset(name: &str) -> Option<Kit> {
        let k = |reach, damage, strike_rate, close_penalty, armour, shield, mounted| Kit {
            name: name.to_string(),
            reach,
            damage,
            strike_rate,
            close_penalty,
            range: 0.0,
            shot_rate: 0.0,
            shot_damage: 0.0,
            arrows: 0,
            armour,
            shield,
            mounted,
        };
        Some(match name {
            "spearmen" => k(3.0, 0.45, 0.30, 0.4, 0.2, 0.4, false),
            "swordsmen" => k(1.2, 0.6, 0.35, 0.0, 0.5, 0.3, false),
            "cavalry" => k(3.0, 0.5, 0.30, 0.3, 0.5, 0.3, true),
            "levy" => k(1.5, 0.3, 0.25, 0.0, 0.0, 0.0, false),
            "heavy" => k(3.0, 0.55, 0.32, 0.1, 0.5, 0.4, false),
            "archers" => Kit {
                range: 150.0,
                shot_rate: 0.1,
                shot_damage: 0.4,
                arrows: 24,
                ..k(0.5, 0.25, 0.3, 0.0, 0.15, 0.0, false)
            },
            "horse_archers" => Kit {
                range: 120.0,
                shot_rate: 0.08,
                shot_damage: 0.35,
                arrows: 20,
                ..k(0.5, 0.25, 0.3, 0.0, 0.2, 0.0, true)
            },
            _ => return None,
        })
    }

    /// The kit a squad fights with, from what its members carry: for each
    /// member, the `fight` values of their items (weapon, armour, shield, mount).
    /// Melee values are averaged over the squad; a member without a weapon
    /// fights with fists. The squad shoots if at least half its members carry a
    /// bow, and fights mounted if at least half ride.
    pub fn from_gear(name: &str, members: &[Vec<&sim_data::FightDef>]) -> Kit {
        let fists = sim_data::FightDef {
            reach: 0.6,
            damage: 0.15,
            strike_rate: 0.25,
            ..Default::default()
        };
        let mut kit = Kit {
            name: name.to_string(),
            reach: 0.0,
            damage: 0.0,
            strike_rate: 0.0,
            close_penalty: 0.0,
            range: 0.0,
            shot_rate: 0.0,
            shot_damage: 0.0,
            arrows: 0,
            armour: 0.0,
            shield: 0.0,
            mounted: false,
        };
        let n = members.len().max(1) as f32;
        let (mut shooters, mut riders) = (0.0f32, 0.0f32);
        let (mut range, mut shot_rate, mut shot_damage, mut arrows) = (0.0, 0.0, 0.0, 0.0);
        for gear in members {
            let weapon = gear
                .iter()
                .copied()
                .find(|f| f.damage > 0.0 || f.range > 0.0)
                .unwrap_or(&fists);
            kit.reach += weapon.reach / n;
            kit.damage += weapon.damage / n;
            kit.strike_rate += weapon.strike_rate / n;
            kit.close_penalty += weapon.close_penalty / n;
            if weapon.range > 0.0 {
                shooters += 1.0;
                range += weapon.range;
                shot_rate += weapon.shot_rate;
                shot_damage += weapon.shot_damage;
                arrows += weapon.arrows as f32;
            }
            // Layers of armour each stop a share of what gets through.
            let through: f32 = gear.iter().map(|f| 1.0 - f.armour).product();
            kit.armour += (1.0 - through) / n;
            kit.shield += gear.iter().map(|f| f.shield).fold(0.0, f32::max) / n;
            if gear.iter().any(|f| f.mounted) {
                riders += 1.0;
            }
        }
        if shooters > 0.0 && shooters >= n / 2.0 {
            kit.range = range / shooters;
            kit.shot_rate = shot_rate / shooters;
            kit.shot_damage = shot_damage / shooters;
            kit.arrows = (arrows / shooters).round() as u32;
        }
        kit.mounted = riders >= n / 2.0 && riders > 0.0;
        kit
    }

    /// The preset names, for help texts.
    pub const PRESETS: [&'static str; 7] = [
        "spearmen",
        "swordsmen",
        "archers",
        "cavalry",
        "levy",
        "heavy",
        "horse_archers",
    ];
}

/// One fighter.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Member {
    /// 1 is unhurt; at or below 0 the fighter is out of the battle.
    pub health: f32,
    /// Skill with the squad's weapon (the economy's skill level, roughly 0..1).
    pub skill: f32,
    /// Battles survived.
    pub experience: f32,
}

/// A squad as it enters the battle.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SquadSpec {
    pub name: String,
    /// 0 attacks, 1 defends.
    pub side: u8,
    pub kit: Kit,
    pub members: Vec<Member>,
    /// How drilled the squad is, 0..1 (adds to discipline with experience).
    pub drill: f32,
    /// Starting morale 0..1, from the members' needs, pay owed and cohesion.
    pub morale: f32,
    /// Position on the field, metres.
    pub pos: (f32, f32),
    /// Stands behind the town's wall (defenders only).
    pub behind_wall: bool,
}

impl SquadSpec {
    /// A squad of `n` fighters of a preset kit with even skill and experience.
    #[allow(clippy::too_many_arguments)]
    pub fn uniform(
        name: &str,
        side: u8,
        kit: Kit,
        n: usize,
        skill: f32,
        experience: f32,
        morale: f32,
        pos: (f32, f32),
    ) -> Self {
        Self {
            name: name.to_string(),
            side,
            kit,
            members: vec![
                Member {
                    health: 1.0,
                    skill,
                    experience,
                };
                n
            ],
            drill: 0.5,
            morale,
            pos,
            behind_wall: false,
        }
    }
}
