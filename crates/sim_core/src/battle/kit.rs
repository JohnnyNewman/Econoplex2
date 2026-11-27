//! What squads bring to a battle: their kit and their members.

use super::*;
use sim_data::{DamageType, Dice};

/// An amount per damage type, in [`DamageType`] order: pierce, slash, blunt.
pub type PerType = [f32; 3];

/// The share of each type's damage that gets through padded coats and mail
/// (as in the content): a padded coat resists bludgeoning, mail resists
/// slashing; neither stops a point.
pub const PADDED: PerType = [1.0, 1.0, 0.5];
pub const MAIL: PerType = [1.0, 0.5, 1.0];
/// Nothing resisted.
pub const BARE: PerType = [1.0; 3];

/// `amount` of one damage type.
pub fn typed(t: DamageType, amount: f32) -> PerType {
    let mut v = [0.0; 3];
    v[t.index()] = amount;
    v
}

/// What a squad's members carry, in the terms of the combat rules
/// ([`rules`]). Later this comes from the items in their inventories; for
/// now the presets stand in for them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Kit {
    pub name: String,
    /// Melee: reach in metres, attacks per second per fighter in the front
    /// rank, and the share of attacks lost once an enemy with a shorter
    /// weapon is inside the reach.
    pub reach: f32,
    pub strike_rate: f32,
    pub close_penalty: f32,
    /// The damage dice of the squad's main weapon (its quality included),
    /// their type, and the dice it rolls instead on a charge (a lance).
    pub dice: Dice,
    pub damage_type: DamageType,
    pub charge_dice: Dice,
    /// Mean hit points of a hit by damage type, over the squad's weapons (a
    /// squad with swords and spears has some of each), and the same on a
    /// charge.
    pub damage: PerType,
    pub charge: PerType,
    /// Added to attack rolls: the weapons' quality, over the squad.
    pub quality: f32,
    /// Bows: range in metres, shots per second, the dice and type of an
    /// arrow, its mean hit points by type, arrows per fighter.
    pub range: f32,
    pub shot_rate: f32,
    pub shot_dice: Dice,
    pub shot_type: DamageType,
    pub shot_damage: PerType,
    pub arrows: u32,
    /// Armour class from armour (over 10) and from a shield (only from the
    /// front), and the share of each type's damage that gets through the
    /// armour (0.5 resisted, 1, 2 vulnerable).
    pub ac: f32,
    pub shield: f32,
    pub taken: PerType,
    pub mounted: bool,
    /// Kilograms carried (weapon, armour, shield; a horse carries itself),
    /// which tires a fighter on the soldier model.
    pub weight: f32,
}

/// Fists: a fighter without a weapon.
const FISTS: Dice = Dice::new(1, 2, 0);

/// `damage` with `extra` hit points split like it, times what armour lets
/// through of each type.
fn scaled(damage: &PerType, extra: f32, taken: &PerType) -> PerType {
    let total: f32 = damage.iter().sum();
    if total <= 0.0 {
        return [0.0; 3];
    }
    let scale = 1.0 + extra / total;
    std::array::from_fn(|t| damage[t] * scale * taken[t])
}

impl Kit {
    /// Mean hit points of a hit before armour, over all damage types.
    pub fn melee_damage(&self) -> f32 {
        self.damage.iter().sum()
    }

    /// Armour class against a blow or an arrow, from the front (with the
    /// shield) or not.
    pub fn armour_class(&self, front: bool) -> f32 {
        10.0 + self.ac + if front { self.shield } else { 0.0 }
    }

    /// Mean hit points of a hit on a fighter whose armour lets `taken`
    /// through, a critical's extra dice counted by `crit` (the share of
    /// hits that are critical); a charge's own dice and its `extra` impact
    /// split like the blow.
    pub fn blow(&self, charge: bool, extra: f32, crit: f32, taken: &PerType) -> f32 {
        self.blow_by_type(charge, extra, crit, taken).iter().sum()
    }

    /// [`Self::blow`] per damage type.
    pub fn blow_by_type(&self, charge: bool, extra: f32, crit: f32, taken: &PerType) -> PerType {
        let damage = if charge { &self.charge } else { &self.damage };
        let dice = if charge { self.charge_dice } else { self.dice };
        scaled(damage, extra + crit * dice.dice_mean(), taken)
    }

    /// Mean hit points of an arrow that hits a fighter whose armour lets
    /// `taken` through, criticals counted by `crit`.
    pub fn shot(&self, crit: f32, taken: &PerType) -> f32 {
        self.shot_by_type(crit, taken).iter().sum()
    }

    /// [`Self::shot`] per damage type.
    pub fn shot_by_type(&self, crit: f32, taken: &PerType) -> PerType {
        scaled(&self.shot_damage, crit * self.shot_dice.dice_mean(), taken)
    }

    /// A kit of one weapon of `dice` (quality `quality`, which is added to
    /// them) of type `t`, the armour and shield given, no bow.
    #[allow(clippy::too_many_arguments)]
    fn plain(
        name: &str,
        reach: f32,
        (dice, t, quality): (Dice, DamageType, i8),
        strike_rate: f32,
        close_penalty: f32,
        (ac, taken): (f32, PerType),
        shield: f32,
        mounted: bool,
        weight: f32,
    ) -> Kit {
        let dice = dice.plus(quality);
        Kit {
            name: name.to_string(),
            reach,
            strike_rate,
            close_penalty,
            dice,
            damage_type: t,
            charge_dice: dice,
            damage: typed(t, dice.mean()),
            charge: typed(t, dice.mean()),
            quality: quality as f32,
            range: 0.0,
            shot_rate: 0.0,
            shot_dice: Dice::default(),
            shot_type: DamageType::Pierce,
            shot_damage: [0.0; 3],
            arrows: 0,
            ac,
            shield,
            taken,
            mounted,
            weight,
        }
    }

    /// The same kit with a bow: range, shots per second, dice, arrows.
    fn with_bow(self, range: f32, shot_rate: f32, dice: Dice, arrows: u32) -> Kit {
        Kit {
            range,
            shot_rate,
            shot_dice: dice,
            shot_type: DamageType::Pierce,
            shot_damage: typed(DamageType::Pierce, dice.mean()),
            arrows,
            ..self
        }
    }

    /// Kits for the unit types in the design note, as the content's items
    /// make them: a 3 m spear, a 1.2 m sword, a 150 m bow with 24 arrows,
    /// mail, padded coats.
    pub fn preset(name: &str) -> Option<Kit> {
        use DamageType::{Blunt, Pierce, Slash};
        let k = Self::plain;
        let d = Dice::new;
        let (padded, mail, none) = ((1.0, PADDED), (5.0, MAIL), (0.0, BARE));
        Some(match name {
            // Weights from the content: spear 2.5 kg, sword 1.4, lance 4,
            // bow 1, shield 5, padded coat 4, mail 10.
            "spearmen" => k(
                name,
                3.0,
                (d(1, 6, 0), Pierce, 1),
                0.15,
                0.4,
                padded,
                2.0,
                false,
                11.5,
            ),
            "swordsmen" => k(
                name,
                1.2,
                (d(1, 8, 0), Slash, 1),
                0.17,
                0.0,
                mail,
                2.0,
                false,
                16.4,
            ),
            "cavalry" => Kit {
                charge_dice: d(1, 12, 1),
                charge: typed(Pierce, d(1, 12, 1).mean()),
                ..k(
                    name,
                    3.0,
                    (d(1, 8, 0), Pierce, 1),
                    0.15,
                    0.3,
                    mail,
                    2.0,
                    true,
                    19.0,
                )
            },
            // Tools from home: flails, clubs, mallets.
            "levy" => k(
                name,
                1.5,
                (d(1, 4, 0), Blunt, 0),
                0.12,
                0.0,
                none,
                0.0,
                false,
                3.0,
            ),
            "heavy" => k(
                name,
                3.0,
                (d(1, 8, 0), Pierce, 1),
                0.16,
                0.1,
                mail,
                2.0,
                false,
                17.5,
            ),
            // Archers fight hand to hand with a knife.
            "archers" => k(
                name,
                0.5,
                (d(1, 4, 0), Slash, 0),
                0.15,
                0.0,
                padded,
                0.0,
                false,
                5.0,
            )
            .with_bow(150.0, 0.1, d(1, 6, 0), 24),
            "horse_archers" => k(
                name,
                0.5,
                (d(1, 4, 0), Slash, 0),
                0.15,
                0.0,
                padded,
                0.0,
                true,
                5.0,
            )
            .with_bow(120.0, 0.08, d(1, 6, 0), 20),
            _ => return None,
        })
    }

    /// The kit a squad fights with, from what its members carry: for each
    /// member, the `fight` values of their items (weapon, armour, shield,
    /// mount). Melee values are averaged over the squad, each weapon's mean
    /// damage under its own damage type, so a squad's mix of weapons gives a
    /// share per type; the dice shown and rolled are the most common
    /// weapon's. A member without a weapon fights with fists. Armour class
    /// and the share of each type let through are averaged; layers of armour
    /// add their class and multiply their shares. The squad shoots if at
    /// least half its members carry a bow, and fights mounted if at least
    /// half ride.
    pub fn from_gear(name: &str, members: &[Vec<&sim_data::FightDef>]) -> Kit {
        let fists = sim_data::FightDef {
            reach: 0.6,
            dice: Some(FISTS),
            damage_type: Some(DamageType::Blunt),
            strike_rate: 0.12,
            ..Default::default()
        };
        let mut kit = Kit {
            name: name.to_string(),
            reach: 0.0,
            strike_rate: 0.0,
            close_penalty: 0.0,
            dice: FISTS,
            damage_type: DamageType::Blunt,
            charge_dice: FISTS,
            damage: [0.0; 3],
            charge: [0.0; 3],
            quality: 0.0,
            range: 0.0,
            shot_rate: 0.0,
            shot_dice: Dice::default(),
            shot_type: DamageType::Pierce,
            shot_damage: [0.0; 3],
            arrows: 0,
            ac: 0.0,
            shield: 0.0,
            taken: [0.0; 3],
            mounted: false,
            weight: 0.0,
        };
        let n = members.len().max(1) as f32;
        let (mut shooters, mut riders) = (0.0f32, 0.0f32);
        let (mut range, mut shot_rate, mut arrows) = (0.0, 0.0, 0.0);
        let mut shot_damage = [0.0f32; 3];
        // How many carry each weapon: the most common gives the dice.
        let mut weapons: Vec<(&sim_data::FightDef, u32)> = Vec::new();
        let mut bows: Vec<(&sim_data::FightDef, u32)> = Vec::new();
        fn count<'a>(list: &mut Vec<(&'a sim_data::FightDef, u32)>, w: &'a sim_data::FightDef) {
            match list.iter_mut().find(|(f, _)| *f == w) {
                Some(e) => e.1 += 1,
                None => list.push((w, 1)),
            }
        }
        for gear in members {
            let weapon = gear
                .iter()
                .copied()
                .find(|f| f.dice.is_some() || f.range > 0.0)
                .unwrap_or(&fists);
            count(&mut weapons, weapon);
            let dice = weapon.dice.unwrap_or(FISTS).plus(weapon.quality);
            let charge = weapon.charge_dice.map_or(dice, |c| c.plus(weapon.quality));
            kit.reach += weapon.reach / n;
            if let Some(t) = weapon.damage_type {
                kit.damage[t.index()] += dice.mean() / n;
                kit.charge[t.index()] += charge.mean() / n;
            }
            kit.quality += weapon.quality as f32 / n;
            kit.strike_rate += weapon.strike_rate / n;
            kit.close_penalty += weapon.close_penalty / n;
            if weapon.range > 0.0 {
                count(&mut bows, weapon);
                shooters += 1.0;
                range += weapon.range;
                shot_rate += weapon.shot_rate;
                let shot = weapon.shot_dice.unwrap_or_default().plus(weapon.quality);
                shot_damage[weapon.shot_type.index()] += shot.mean();
                arrows += weapon.arrows as f32;
            }
            kit.ac += gear.iter().map(|f| f.ac as f32).sum::<f32>() / n;
            for (t, a) in kit.taken.iter_mut().enumerate() {
                *a += gear.iter().map(|f| f.taken()[t]).product::<f32>() / n;
            }
            kit.shield += gear.iter().map(|f| f.shield as f32).fold(0.0, f32::max) / n;
            if gear.iter().any(|f| f.mounted) {
                riders += 1.0;
            }
        }
        let main = |list: &[(&sim_data::FightDef, u32)]| {
            list.iter()
                .max_by_key(|(_, c)| *c)
                .map(|(f, _)| *f)
                .cloned()
        };
        if let Some(w) = main(&weapons) {
            kit.dice = w.dice.unwrap_or(FISTS).plus(w.quality);
            kit.charge_dice = w.charge_dice.map_or(kit.dice, |c| c.plus(w.quality));
            kit.damage_type = w.damage_type.unwrap_or(DamageType::Blunt);
        }
        if shooters > 0.0 && shooters >= n / 2.0 {
            kit.range = range / shooters;
            kit.shot_rate = shot_rate / shooters;
            kit.shot_damage = shot_damage.map(|d| d / shooters);
            kit.arrows = (arrows / shooters).round() as u32;
            if let Some(b) = main(&bows) {
                kit.shot_dice = b.shot_dice.unwrap_or_default().plus(b.quality);
                kit.shot_type = b.shot_type;
            }
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
    /// The share of their hit points left (`rules::max_hp`): 1 is unhurt;
    /// at or below 0 the fighter is down, out of the battle.
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
