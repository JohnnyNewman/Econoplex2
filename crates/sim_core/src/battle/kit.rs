//! What squads bring to a battle: their kit and their members.

use super::*;
use sim_data::DamageType;

/// An amount per damage type, in [`DamageType`] order: pierce, slash, blunt.
pub type PerType = [f32; 3];

/// What padded coats and mail stop, per damage type (as in the content): a
/// padded coat soaks up a blow but a point goes through it; mail turns an
/// edge but not a point or a weight. Their means are the single shares they
/// had before damage types (0.2 and 0.5), so only the split is new.
pub const PADDED: PerType = [0.1, 0.2, 0.3];
pub const MAIL: PerType = [0.45, 0.65, 0.4];

/// `amount` of one damage type.
pub fn typed(t: DamageType, amount: f32) -> PerType {
    let mut v = [0.0; 3];
    v[t.index()] = amount;
    v
}

/// What a squad's members carry. Later this comes from the items in their
/// inventories; for now the presets stand in for them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Kit {
    pub name: String,
    /// Melee: reach in metres, health taken per hit split by damage type (a
    /// squad with swords and spears has some of each), strikes per second per
    /// fighter in the front rank, and the share of strikes lost once an enemy
    /// with a shorter weapon is inside the reach.
    pub reach: f32,
    pub damage: PerType,
    pub strike_rate: f32,
    pub close_penalty: f32,
    /// Bows: range in metres, shots per second, health per hit by damage
    /// type, arrows per fighter.
    pub range: f32,
    pub shot_rate: f32,
    pub shot_damage: PerType,
    pub arrows: u32,
    /// Share of a hit's damage armour stops per damage type, and share of
    /// arrows a shield stops.
    pub armour: PerType,
    pub shield: f32,
    pub mounted: bool,
    /// Kilograms carried (weapon, armour, shield; a horse carries itself),
    /// which tires a fighter on the soldier model.
    pub weight: f32,
}

impl Kit {
    /// Health one blow takes before armour, over all damage types.
    pub fn melee_damage(&self) -> f32 {
        self.damage.iter().sum()
    }

    /// Health one blow takes from a fighter wearing `armour`: the sum over
    /// damage types of damage × (1 − armour). A charge's `extra` is split
    /// like the blow.
    pub fn blow(&self, extra: f32, armour: &PerType) -> f32 {
        let total = self.melee_damage();
        if total <= 0.0 {
            return 0.0;
        }
        let scale = 1.0 + extra / total;
        (0..3)
            .map(|t| self.damage[t] * scale * (1.0 - armour[t]))
            .sum()
    }

    /// Health one arrow takes from a fighter wearing `armour`.
    pub fn shot(&self, armour: &PerType) -> f32 {
        (0..3)
            .map(|t| self.shot_damage[t] * (1.0 - armour[t]))
            .sum()
    }

    /// Kits for the unit types in the design note. Numbers are physical guesses:
    /// a 3 m spear, a 1.2 m sword, a 150 m bow with 24 arrows, mail, padded coats.
    pub fn preset(name: &str) -> Option<Kit> {
        use DamageType::{Blunt, Pierce, Slash};
        let k =
            |reach, (t, damage), strike_rate, close_penalty, armour, shield, mounted, weight| Kit {
                name: name.to_string(),
                reach,
                damage: typed(t, damage),
                strike_rate,
                close_penalty,
                range: 0.0,
                shot_rate: 0.0,
                shot_damage: [0.0; 3],
                arrows: 0,
                armour,
                shield,
                mounted,
                weight,
            };
        let none = [0.0; 3];
        Some(match name {
            // Weights from the content: spear 2.5 kg, sword 1.4, lance 4,
            // bow 1, shield 5, padded coat 4, mail 10.
            "spearmen" => k(3.0, (Pierce, 0.45), 0.30, 0.4, PADDED, 0.4, false, 11.5),
            "swordsmen" => k(1.2, (Slash, 0.6), 0.35, 0.0, MAIL, 0.3, false, 16.4),
            "cavalry" => k(3.0, (Pierce, 0.5), 0.30, 0.3, MAIL, 0.3, true, 19.0),
            // Tools from home: flails, clubs, mallets.
            "levy" => k(1.5, (Blunt, 0.3), 0.25, 0.0, none, 0.0, false, 3.0),
            "heavy" => k(3.0, (Pierce, 0.55), 0.32, 0.1, MAIL, 0.4, false, 17.5),
            // Archers fight hand to hand with a knife.
            "archers" => Kit {
                range: 150.0,
                shot_rate: 0.1,
                shot_damage: typed(Pierce, 0.4),
                arrows: 24,
                ..k(0.5, (Slash, 0.25), 0.3, 0.0, PADDED, 0.0, false, 5.0)
            },
            "horse_archers" => Kit {
                range: 120.0,
                shot_rate: 0.08,
                shot_damage: typed(Pierce, 0.35),
                arrows: 20,
                ..k(0.5, (Slash, 0.25), 0.3, 0.0, PADDED, 0.0, true, 5.0)
            },
            _ => return None,
        })
    }

    /// The kit a squad fights with, from what its members carry: for each
    /// member, the `fight` values of their items (weapon, armour, shield, mount).
    /// Melee values are averaged over the squad, each weapon's damage under its
    /// own damage type, so a squad's mix of weapons gives a share per type; a
    /// member without a weapon fights with fists. Armour is averaged per type.
    /// The squad shoots if at least half its members carry a bow, and fights
    /// mounted if at least half ride.
    pub fn from_gear(name: &str, members: &[Vec<&sim_data::FightDef>]) -> Kit {
        let fists = sim_data::FightDef {
            reach: 0.6,
            damage: 0.15,
            damage_type: Some(DamageType::Blunt),
            strike_rate: 0.25,
            ..Default::default()
        };
        let mut kit = Kit {
            name: name.to_string(),
            reach: 0.0,
            damage: [0.0; 3],
            strike_rate: 0.0,
            close_penalty: 0.0,
            range: 0.0,
            shot_rate: 0.0,
            shot_damage: [0.0; 3],
            arrows: 0,
            armour: [0.0; 3],
            shield: 0.0,
            mounted: false,
            weight: 0.0,
        };
        let n = members.len().max(1) as f32;
        let (mut shooters, mut riders) = (0.0f32, 0.0f32);
        let (mut range, mut shot_rate, mut arrows) = (0.0, 0.0, 0.0);
        let mut shot_damage = [0.0f32; 3];
        for gear in members {
            let weapon = gear
                .iter()
                .copied()
                .find(|f| f.damage > 0.0 || f.range > 0.0)
                .unwrap_or(&fists);
            kit.reach += weapon.reach / n;
            if let Some(t) = weapon.damage_type {
                kit.damage[t.index()] += weapon.damage / n;
            }
            kit.strike_rate += weapon.strike_rate / n;
            kit.close_penalty += weapon.close_penalty / n;
            if weapon.range > 0.0 {
                shooters += 1.0;
                range += weapon.range;
                shot_rate += weapon.shot_rate;
                shot_damage[weapon.shot_type.index()] += weapon.shot_damage;
                arrows += weapon.arrows as f32;
            }
            // Layers of armour each stop a share of what gets through, per type.
            for (t, a) in kit.armour.iter_mut().enumerate() {
                let through: f32 = gear.iter().map(|f| 1.0 - f.armour.shares()[t]).product();
                *a += (1.0 - through) / n;
            }
            kit.shield += gear.iter().map(|f| f.shield).fold(0.0, f32::max) / n;
            if gear.iter().any(|f| f.mounted) {
                riders += 1.0;
            }
        }
        if shooters > 0.0 && shooters >= n / 2.0 {
            kit.range = range / shooters;
            kit.shot_rate = shot_rate / shooters;
            kit.shot_damage = shot_damage.map(|d| d / shooters);
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
    /// 0 is rested, 1 is spent (the person's `fatigue` state).
    pub fatigue: f32,
    /// Their own kit: an index into the squad's [`SquadSpec::kits`].
    pub kit: u16,
    /// Taken prisoner while running (by a squad whose pursuit is
    /// `Pursuit::Capture`): out of the battle, unhurt by it.
    #[serde(default)]
    pub captured: bool,
}

impl Member {
    /// Still in the battle: on their feet and not taken.
    pub fn fights(&self) -> bool {
        self.health > 0.0 && !self.captured
    }
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
    /// Each member's own kit, as [`Member::kit`] indexes it (the soldier
    /// model fights with these; `kit` is their squad's average).
    pub kits: Vec<Kit>,
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
            kits: vec![kit.clone()],
            kit,
            members: vec![
                Member {
                    health: 1.0,
                    skill,
                    experience,
                    fatigue: 0.0,
                    kit: 0,
                    captured: false,
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
