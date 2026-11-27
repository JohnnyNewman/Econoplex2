//! The combat rules both models fight by, in the terms of the open
//! fifth-edition system reference document (its ideas, none of its text):
//!
//! - **Attack roll.** A blow or an arrow hits when d20 + the attack bonus
//!   reaches the target's armour class (AC). A natural 20 always hits and is
//!   a critical hit (the damage dice are rolled twice); a natural 1 always
//!   misses.
//! - **Attack bonus**: proficiency (from skill and battles survived) plus the
//!   weapon's quality, less one per level of exhaustion; an arrow loses more
//!   the further it flies.
//! - **Armour class**: 10 + armour + shield (a shield only from the front),
//!   + cover for a man on a wall when shot at from below it.
//! - **Damage**: the weapon's dice, by damage type (piercing, slashing,
//!   bludgeoning). Armour resists some types (half damage) and may be
//!   vulnerable to others (double).
//! - **Hit points**: a townsperson has about 8, more with experience. A
//!   member's `health` (1 unhurt, 0 down) is the share of them left.
//! - **Advantage and disadvantage**: roll two d20 and take the higher or
//!   the lower. A blow in the flank or the rear, or on a man in a crush, has
//!   advantage; a frightened attacker, or one held in a crush, has
//!   disadvantage. One of each cancel out.
//! - **Conditions**: *prone* (down, out of the fight), *frightened*
//!   (wavering), *exhausted* (a level per fifth of fatigue), *restrained*
//!   (pressed in a crush, soldier model only).
//!
//! The soldier model rolls every attack; the squad model takes each blow's
//! chance from the same numbers and deals the expected damage of a hit.

use super::BattleParams;
pub use sim_data::Dice;

/// How an attack is rolled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Roll {
    /// One d20.
    Normal,
    /// Two d20, the higher.
    Advantage,
    /// Two d20, the lower.
    Disadvantage,
}

impl Roll {
    /// The roll for an attack with a reason for advantage `adv` and one for
    /// disadvantage `dis`: one of each cancel out.
    pub fn of(adv: bool, dis: bool) -> Self {
        match (adv, dis) {
            (true, false) => Self::Advantage,
            (false, true) => Self::Disadvantage,
            _ => Self::Normal,
        }
    }
}

/// The face of the d20 an attack needs to hit, 2 to 20: "hits on 14+".
pub fn needs(bonus: f32, ac: f32) -> i32 {
    ((ac - bonus).ceil() as i32).clamp(2, 20)
}

/// The chance one d20 hits: a natural 20 always does, a natural 1 never.
/// A bonus between whole numbers counts its share of the next face, so the
/// squad model's average bonus moves the chance smoothly.
fn one_die(bonus: f32, ac: f32) -> f32 {
    ((21.0 + bonus - ac) / 20.0).clamp(0.05, 0.95)
}

/// The chance an attack with `bonus` hits armour class `ac`.
pub fn hit_chance(bonus: f32, ac: f32, roll: Roll) -> f32 {
    let p = one_die(bonus, ac);
    match roll {
        Roll::Normal => p,
        Roll::Advantage => 1.0 - (1.0 - p) * (1.0 - p),
        Roll::Disadvantage => p * p,
    }
}

/// The chance of a natural 20: a critical hit.
pub fn crit_chance(roll: Roll) -> f32 {
    match roll {
        Roll::Normal => 0.05,
        Roll::Advantage => 1.0 - 0.95 * 0.95,
        Roll::Disadvantage => 0.05 * 0.05,
    }
}

/// The mean damage of a hit with `dice` (before armour): the dice and their
/// bonus, and the dice once more on the share of hits that are critical.
pub fn mean_hit(dice: Dice, hit: f32, roll: Roll) -> f32 {
    if hit <= 0.0 {
        return 0.0;
    }
    dice.mean() + crit_chance(roll).min(hit) / hit * dice.dice_mean()
}

/// One attack, rolled from uniform draws `u` and `v` in [0, 1) (`v` only
/// for advantage and disadvantage): whether it hits, and whether it is a
/// critical hit.
pub fn attack(bonus: f32, ac: f32, roll: Roll, u: f32, v: f32) -> (bool, bool) {
    let face = |x: f32| ((x * 20.0) as i32 + 1).clamp(1, 20);
    let face = match roll {
        Roll::Normal => face(u),
        Roll::Advantage => face(u).max(face(v)),
        Roll::Disadvantage => face(u).min(face(v)),
    };
    match face {
        20 => (true, true),
        1 => (false, false),
        f => (f as f32 + bonus >= ac, false),
    }
}

/// A roll of `dice` from uniform draws (one per die, twice as many on a
/// critical hit), never below 0.
pub fn roll_dice(dice: Dice, crit: bool, mut draw: impl FnMut() -> f32) -> f32 {
    let n = dice.n as u32 * if crit { 2 } else { 1 };
    let faces: u32 = (0..n)
        .map(|_| ((draw() * dice.sides as f32) as u32).min(dice.sides as u32 - 1) + 1)
        .sum();
    (faces as f32 + dice.plus as f32).max(0.0)
}

/// Proficiency: `proficiency` for a fighter of skill 0.5 and no battles,
/// `skill_bonus` more per unit of skill above that, and `experience_bonus`
/// per ln(1 + battles survived). Never below 0.
pub fn proficiency(p: &BattleParams, skill: f32, experience: f32) -> f32 {
    (p.proficiency
        + p.skill_bonus * (skill - 0.5)
        + p.experience_bonus * (1.0 + experience.max(0.0)).ln())
    .max(0.0)
}

/// Hit points when unhurt: `hit_points`, and `hp_per_experience` per
/// ln(1 + battles survived).
pub fn max_hp(p: &BattleParams, experience: f32) -> f32 {
    p.hit_points + p.hp_per_experience * (1.0 + experience.max(0.0)).ln()
}

/// Levels of exhaustion, 0 to 5: one per fifth of fatigue.
pub fn exhaustion(fatigue: f32) -> u8 {
    (fatigue.clamp(0.0, 1.0) * 5.0).floor() as u8
}

/// What a soldier is, in the rules' terms, for the views: armour class
/// from the front, attack bonus, hit points left and when unhurt, and their
/// weapon's dice and damage type.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sheet {
    pub ac: u8,
    pub to_hit: i8,
    pub hp: u8,
    pub max_hp: u8,
    pub dice: Dice,
    pub damage_type: sim_data::DamageType,
}

impl Sheet {
    /// The sheet in words: "AC 17, +3 to hit, HP 6/8, 1d8+1 slashing".
    pub fn words(&self) -> String {
        format!(
            "AC {}, {:+} to hit, HP {}/{}, {} {}",
            self.ac,
            self.to_hit,
            self.hp,
            self.max_hp,
            self.dice,
            self.damage_type.word()
        )
    }
}

/// Each member's sheet of squad `spec`, their fatigue from `condition`
/// (the soldier model's, as it is now) or from the members, and the mean
/// attack bonus of those still fighting.
pub fn sheets(
    p: &BattleParams,
    spec: &super::SquadSpec,
    condition: Option<&Vec<super::Condition>>,
) -> (Vec<Sheet>, f32) {
    let (mut sum, mut n) = (0.0, 0.0);
    let sheets = spec
        .members
        .iter()
        .enumerate()
        .map(|(k, m)| {
            let kit = spec.kits.get(m.kit as usize).unwrap_or(&spec.kit);
            let fatigue = condition
                .and_then(|c| c.get(k))
                .map_or(m.fatigue, |c| c.fatigue());
            let bonus = proficiency(p, m.skill, m.experience) + kit.quality
                - p.exhaustion_penalty * exhaustion(fatigue) as f32;
            if m.fights() {
                sum += bonus;
                n += 1.0;
            }
            let max = max_hp(p, m.experience);
            Sheet {
                ac: kit.armour_class(true).round().clamp(0.0, 255.0) as u8,
                to_hit: bonus.round().clamp(-99.0, 99.0) as i8,
                hp: (m.health.max(0.0) * max).round().min(255.0) as u8,
                max_hp: max.round().min(255.0) as u8,
                dice: kit.dice,
                damage_type: kit.damage_type,
            }
        })
        .collect();
    (sheets, if n > 0.0 { sum / n } else { 0.0 })
}

/// The F1 help's page on the combat rules, for both clients.
pub const HOW_COMBAT_WORKS: &str = "\
How combat works (the open 5e SRD ideas):
an attack hits when d20 + attack bonus reaches the target's armour class (AC)
natural 20: a critical hit, the damage dice twice; natural 1: always a miss
attack bonus: proficiency (skill and battles survived) + weapon quality, less exhaustion
AC: 10 + armour + shield (a shield only from the front) + cover on a wall
damage: the weapon's dice (mace 1d6 bludgeoning, longsword 1d8 slashing, lance 1d12 on a charge)
armour resists some damage types (half): mail slashing, padded coats bludgeoning
hit points: about 8 for a townsperson, more with experience; at 0 they are down (prone)
advantage (two d20, the higher): blows in the flank or rear, or on a man in a crush
disadvantage (two d20, the lower): frightened (wavering), or restrained in a crush
exhaustion: a level per fifth of fatigue, each -1 to attack rolls";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_d20_hits_as_its_faces_say() {
        // Needs 14 or more: 7 faces of 20.
        assert!((hit_chance(3.0, 17.0, Roll::Normal) - 0.35).abs() < 1e-6);
        assert_eq!(needs(3.0, 17.0), 14);
        // A natural 20 always hits, a natural 1 always misses.
        assert!((hit_chance(-30.0, 17.0, Roll::Normal) - 0.05).abs() < 1e-6);
        assert!((hit_chance(30.0, 17.0, Roll::Normal) - 0.95).abs() < 1e-6);
        // Two dice: the higher or the lower.
        let p = 0.35f32;
        assert!(
            (hit_chance(3.0, 17.0, Roll::Advantage) - (1.0 - (1.0 - p) * (1.0 - p))).abs() < 1e-6
        );
        assert!((hit_chance(3.0, 17.0, Roll::Disadvantage) - p * p).abs() < 1e-6);
        assert_eq!(Roll::of(true, true), Roll::Normal);
        // The rolled attack hits as often as the chance says.
        let mut hits = 0;
        for a in 0..20 {
            for b in 0..20 {
                let (u, v) = ((a as f32 + 0.5) / 20.0, (b as f32 + 0.5) / 20.0);
                hits += attack(3.0, 17.0, Roll::Advantage, u, v).0 as u32;
            }
        }
        assert!((hits as f32 / 400.0 - hit_chance(3.0, 17.0, Roll::Advantage)).abs() < 1e-6);
    }

    #[test]
    fn dice_roll_their_faces_and_a_critical_rolls_them_twice() {
        let d: Dice = "1d8+1".parse().unwrap();
        assert_eq!(d, Dice::new(1, 8, 1));
        assert_eq!(d.to_string(), "1d8+1");
        assert!((d.mean() - 5.5).abs() < 1e-6);
        assert_eq!(roll_dice(d, false, || 0.999), 9.0);
        assert_eq!(roll_dice(d, true, || 0.0), 3.0);
        // A hit's mean counts the share of criticals.
        let hit = hit_chance(3.0, 17.0, Roll::Normal);
        assert!((mean_hit(d, hit, Roll::Normal) - (5.5 + 0.05 / 0.35 * 4.5)).abs() < 1e-5);
        assert!("d8".parse::<Dice>().is_err() && "1d0".parse::<Dice>().is_err());
    }
}
