//! Weather and terrain that bite (docs/design/war-ideas.md, idea 7): a
//! battle fought in rain, mud, snow or the dark (`Conditions`, read from
//! the world's weather, ground and hour by the war system) fights
//! differently, on both models, through the effective numbers below
//! (`BattleParams::paces`, `charge`, `shot_rate`, `shot_range`,
//! `shot_bonus`, `shot_disadvantage`). The strengths of the effects are
//! content (`battle.weather` in models.ron). Every effect scales a rate or
//! a speed, which the models integrate over their step `dt`, so a battle
//! comes out the same whatever its step.

use super::BattleParams;
use serde::{Deserialize, Serialize};

/// The conditions a battle is fought in, each from 0 (none) to 1 (the
/// worst).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conditions {
    /// Rain falling: wet bowstrings draw slower and shoot worse.
    pub rain: f32,
    /// Mud underfoot: everyone slower, horses most, and charges blunted.
    pub mud: f32,
    /// Snow on the ground: everyone a little slower.
    pub snow: f32,
    /// Dark: shooting range shortened, and shots past `night_sight` at
    /// disadvantage.
    pub dark: f32,
}

/// How hard each condition bites (`battle.weather` in models.ron).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherParams {
    /// Share of their shooting rate bows lose in full rain, the attack
    /// bonus arrows lose in it, and the rain at and above which every shot
    /// is at disadvantage (heavy rain).
    pub rain_draw: f32,
    pub rain_to_hit: f32,
    pub heavy_rain: f32,
    /// Share of their speed men on foot and horses lose in full mud, and
    /// the share of a charge's extra damage it takes away.
    pub mud_foot: f32,
    pub mud_horse: f32,
    pub mud_charge: f32,
    /// Share of their speed everyone loses in deep snow.
    pub snow_slow: f32,
    /// Share of its range a bow loses in full dark, and how far (metres) a
    /// man sees to aim in the dark: shots past it are at disadvantage once
    /// it is at least half dark.
    pub night_range: f32,
    pub night_sight: f32,
    /// Below this, a condition is not worth a word in the battle panel.
    pub notable: f32,
}

impl Default for WeatherParams {
    fn default() -> Self {
        Self {
            rain_draw: 0.3,
            rain_to_hit: 1.5,
            heavy_rain: 0.75,
            mud_foot: 0.25,
            mud_horse: 0.55,
            mud_charge: 0.7,
            snow_slow: 0.15,
            night_range: 0.4,
            night_sight: 50.0,
            notable: 0.2,
        }
    }
}

impl BattleParams {
    fn cond(&self) -> Conditions {
        let c = self.conditions;
        let k = |v: f32| v.clamp(0.0, 1.0);
        Conditions {
            rain: k(c.rain),
            mud: k(c.mud),
            snow: k(c.snow),
            dark: k(c.dark),
        }
    }

    /// Marching and running speed (walking and galloping for riders) in
    /// the battle's conditions, metres per second.
    pub fn paces(&self, mounted: bool) -> (f32, f32) {
        let c = self.cond();
        let w = &self.weather;
        let (walk, run, mud) = if mounted {
            (self.horse_walk_speed, self.horse_charge_speed, w.mud_horse)
        } else {
            (self.walk_speed, self.run_speed, w.mud_foot)
        };
        let f = ((1.0 - mud * c.mud) * (1.0 - w.snow_slow * c.snow)).max(0.2);
        (walk * f, run * f)
    }

    /// Extra damage per metre per second of a charge (`charge_damage`),
    /// blunted by mud.
    pub fn charge(&self) -> f32 {
        self.charge_damage * (1.0 - self.weather.mud_charge * self.cond().mud).max(0.0)
    }

    /// A bow's shots per second `rate` in the rain.
    pub fn shot_rate(&self, rate: f32) -> f32 {
        rate * (1.0 - self.weather.rain_draw * self.cond().rain).max(0.05)
    }

    /// A bow's range `range` (metres) in the dark.
    pub fn shot_range(&self, range: f32) -> f32 {
        range * (1.0 - self.weather.night_range * self.cond().dark).max(0.1)
    }

    /// Attack bonus an arrow gains (less than 0: loses) in the conditions.
    pub fn shot_bonus(&self) -> f32 {
        -self.weather.rain_to_hit * self.cond().rain
    }

    /// Whether a shot at `dist` metres is at disadvantage: in heavy rain,
    /// or past what a man sees in the dark.
    pub fn shot_disadvantage(&self, dist: f32) -> bool {
        let c = self.cond();
        let w = &self.weather;
        (c.rain > 0.0 && c.rain >= w.heavy_rain) || (c.dark >= 0.5 && dist > w.night_sight)
    }
}

impl Conditions {
    /// The conditions at town `town` now: the rain falling there (not the
    /// snow), its wet ground as mud unless snow covers it, its snow cover,
    /// and the dark of night (half at dawn and dusk).
    pub fn at(env: &crate::env::Environment, town: usize) -> Self {
        let local = env.at(town);
        let w = &local.weather;
        let g = &local.ground;
        Self {
            rain: w.precip * (1.0 - w.snow_fraction()),
            mud: g.wet * (1.0 - g.snow),
            snow: g.snow,
            dark: match env.phase {
                crate::env::DayPhase::Day => 0.0,
                crate::env::DayPhase::Dawn | crate::env::DayPhase::Dusk => 0.3,
                crate::env::DayPhase::Night => 1.0,
            },
        }
    }

    /// The conditions as the battle panels and the CLI say them, "Rain:
    /// bows at disadvantage, mud: horses slowed"; `None` in fair weather.
    pub fn words(&self, w: &WeatherParams) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        if self.rain >= w.notable {
            parts.push(if self.rain >= w.heavy_rain {
                "rain: bows at disadvantage"
            } else {
                "rain: bows slowed"
            });
        }
        if self.mud >= w.notable {
            parts.push("mud: horses slowed, charges blunted");
        }
        if self.snow >= w.notable {
            parts.push("snow: everyone slowed");
        }
        if self.dark >= 0.5 {
            parts.push("night: short sight and range");
        }
        let s = parts.join(", ");
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn params(c: Conditions) -> BattleParams {
        BattleParams {
            conditions: c,
            ..BattleParams::default()
        }
    }

    #[test]
    fn the_words_name_what_bites() {
        let w = WeatherParams::default();
        assert_eq!(Conditions::default().words(&w), None);
        let c = Conditions {
            rain: 1.0,
            mud: 0.8,
            ..Default::default()
        };
        assert_eq!(
            c.words(&w).unwrap(),
            "Rain: bows at disadvantage, mud: horses slowed, charges blunted"
        );
        let c = Conditions {
            dark: 1.0,
            snow: 0.5,
            ..Default::default()
        };
        assert_eq!(
            c.words(&w).unwrap(),
            "Snow: everyone slowed, night: short sight and range"
        );
    }

    #[test]
    fn each_condition_moves_its_numbers_the_right_way() {
        let dry = params(Conditions::default());
        let mud = params(Conditions {
            mud: 1.0,
            ..Default::default()
        });
        // Mud slows horses more than men, and blunts charges.
        let foot = mud.paces(false).0 / dry.paces(false).0;
        let horse = mud.paces(true).1 / dry.paces(true).1;
        assert!(horse < foot && foot < 1.0, "{horse} {foot}");
        assert!(mud.charge() < dry.charge());
        let snow = params(Conditions {
            snow: 1.0,
            ..Default::default()
        });
        assert!(snow.paces(false).0 < dry.paces(false).0);
        assert!(snow.paces(false).0 > mud.paces(false).0);
        let rain = params(Conditions {
            rain: 1.0,
            ..Default::default()
        });
        assert!(rain.shot_rate(0.2) < dry.shot_rate(0.2));
        assert!(rain.shot_bonus() < 0.0 && dry.shot_bonus() == 0.0);
        assert!(rain.shot_disadvantage(10.0) && !dry.shot_disadvantage(10.0));
        let night = params(Conditions {
            dark: 1.0,
            ..Default::default()
        });
        assert!(night.shot_range(150.0) < dry.shot_range(150.0));
        assert!(night.shot_disadvantage(60.0) && !night.shot_disadvantage(20.0));
        // Fair weather changes nothing.
        assert_eq!(
            dry.paces(true),
            (dry.horse_walk_speed, dry.horse_charge_speed)
        );
        assert_eq!(dry.shot_range(150.0), 150.0);
        assert_eq!(dry.charge(), dry.charge_damage);
    }

    /// `lineup` fought over `n` seeds on `model` in `c`: the defenders'
    /// losses (down or dead) and the battle's seconds, per battle.
    fn fight(lineup: &str, model: BattleModel, c: Conditions, n: u64) -> (f32, f32) {
        let (mut lost, mut secs) = (0.0, 0.0);
        for seed in 1..=n {
            let (field, specs) = parse_lineup(lineup).unwrap();
            let p = BattleParams {
                model,
                conditions: c,
                ..BattleParams::default()
            };
            let r = Battle::new(p, field, specs, seed).run();
            lost += r
                .squads
                .iter()
                .filter(|s| s.side == 1)
                .map(|s| (s.wounded + s.dead) as f32)
                .sum::<f32>();
            secs += r.seconds;
        }
        (lost / n as f32, secs / n as f32)
    }

    const MODELS: [BattleModel; 2] = [BattleModel::Squads, BattleModel::Soldiers];

    /// Rain and night spoil the archers' shooting: the swordsmen they shoot
    /// at lose fewer, on both models.
    #[test]
    fn rain_and_night_spare_those_the_archers_shoot() {
        let lineup = "archers*30 vs swordsmen*25";
        for model in MODELS {
            let dry = fight(lineup, model, Conditions::default(), 6).0;
            for c in [
                Conditions {
                    rain: 1.0,
                    ..Default::default()
                },
                Conditions {
                    dark: 1.0,
                    ..Default::default()
                },
            ] {
                let wet = fight(lineup, model, c, 6).0;
                assert!(wet < dry, "{model:?} {c:?}: {wet} vs dry {dry}");
            }
        }
    }

    /// Mud slows the horse archers' dance round the spearmen: they bring
    /// down fewer, and it takes them longer, on both models.
    #[test]
    fn mud_holds_horse_archers_back() {
        let lineup = "horse_archers*30 vs spearmen*40";
        let mud = Conditions {
            mud: 1.0,
            ..Default::default()
        };
        for model in MODELS {
            let (dry_lost, dry_secs) = fight(lineup, model, Conditions::default(), 6);
            let (mud_lost, mud_secs) = fight(lineup, model, mud, 6);
            assert!(
                mud_lost < dry_lost,
                "{model:?}: {mud_lost} vs dry {dry_lost}"
            );
            assert!(
                mud_secs > dry_secs,
                "{model:?}: {mud_secs} s vs dry {dry_secs} s"
            );
        }
    }

    /// Snow slows everyone a little: the same fight takes longer.
    #[test]
    fn snow_draws_a_battle_out() {
        let lineup = "swordsmen*30 vs spearmen*30";
        let snow = Conditions {
            snow: 1.0,
            ..Default::default()
        };
        for model in MODELS {
            let dry = fight(lineup, model, Conditions::default(), 4).1;
            let slow = fight(lineup, model, snow, 4).1;
            assert!(slow > dry, "{model:?}: {slow} s vs dry {dry} s");
        }
    }

    /// The effects are rates and speeds, so the step does not change them:
    /// on the squad model, mud slows a march by the same share at a step
    /// of half a second as of one.
    #[test]
    fn the_effects_do_not_depend_on_the_step() {
        let lineup = "swordsmen*30 vs spearmen*30";
        let mud = Conditions {
            mud: 1.0,
            ..Default::default()
        };
        let ratio = |dt: f32| {
            let secs = |c: Conditions| {
                let (field, specs) = parse_lineup(lineup).unwrap();
                let p = BattleParams {
                    dt,
                    conditions: c,
                    ..BattleParams::default()
                };
                // Until the lines first meet: the march alone.
                let mut b = Battle::new(p, field, specs, 1);
                while !b.finished()
                    && !b.report().events.iter().any(|e| {
                        matches!(e.kind, EventKind::Contact { .. } | EventKind::Charge { .. })
                    })
                {
                    b.step();
                }
                b.seconds()
            };
            secs(mud) / secs(Conditions::default())
        };
        let (a, b) = (ratio(1.0), ratio(0.5));
        assert!(a > 1.1 && (a - b).abs() < 0.1, "{a} vs {b}");
    }
}
