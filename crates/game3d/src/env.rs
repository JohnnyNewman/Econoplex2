//! The environment the 3D view draws: time of day, season and weather.
//!
//! Everything that changes with the clock or the weather reads one resource,
//! [`Environment`], instead of computing its own:
//! - the sky (sky.rs): sun and moon, stars, clouds, overcast;
//! - the light: sun or moon, dimmed by clouds;
//! - the fog (fog.rs): denser and greyer in rain, and the shared ground state
//!   (snow cover, wetness, autumn colour) every toon, terrain and grass
//!   material gets with it;
//! - the wind in the grass and trees;
//! - rain and snow (precip.rs).
//!
//! Where the values come from is the [`EnvSource`]:
//! - `Stub` (for now): a small made-up weather model in this file, driven by
//!   the view's clock (`SkyClock`) and the sim's calendar (the season);
//! - later the simulation's own clock and weather, published in its snapshot.
//!   Day and night and the weather change gameplay there (work hours, warmth,
//!   harvests, travel), so the view must show the sim's state, not its own.
//!   Only `update_environment` changes then.
//!
//! Keys: `Y` cycles the weather (auto, clear, cloudy, rain, storm, snow, fog),
//! `U` the season (auto, spring, summer, autumn, winter).

use crate::sky::SkyClock;
use bevy::prelude::*;
use std::f32::consts::TAU;

/// The weather right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weather {
    /// Air temperature in Â°C.
    pub temperature: f32,
    /// Where the wind blows toward (3D x, z; m/s).
    pub wind: Vec2,
    /// Cloud cover, 0 (clear) .. 1 (overcast).
    pub cloud: f32,
    /// Precipitation, 0 (dry) .. 1 (downpour). Snow when it is cold.
    pub precip: f32,
    /// Extra haze (mist, fog banks), 0 .. 1.
    pub haze: f32,
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            temperature: 18.0,
            wind: Vec2::new(2.4, 1.8),
            cloud: 0.15,
            precip: 0.0,
            haze: 0.0,
        }
    }
}

impl Weather {
    /// How much of the precipitation falls as snow, 0 .. 1.
    pub fn snow_fraction(&self) -> f32 {
        smooth(2.0, -1.0, self.temperature)
    }
}

/// What the weather has done to the ground (it changes slowly).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Ground {
    /// Snow cover, 0 .. 1.
    pub snow: f32,
    /// Wet surfaces, 0 .. 1.
    pub wet: f32,
}

/// Where the environment comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EnvSource {
    /// The stand-in weather model below.
    #[default]
    Stub,
}

/// Time of day, season and weather, as everything in the view should draw them.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Environment {
    pub source: EnvSource,
    /// Hour of the day, 0 .. 24.
    pub hour: f32,
    /// Fraction of the year, 0 .. 1 (0 = midwinter, 0.5 = midsummer).
    pub season: f32,
    /// Days since the start (continuous), for the moon's phase and the weather.
    pub day: f32,
    pub weather: Weather,
    pub ground: Ground,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            source: EnvSource::Stub,
            hour: 16.5,
            season: 0.5,
            day: 0.0,
            weather: Weather::default(),
            ground: Ground::default(),
        }
    }
}

impl Environment {
    /// Autumn colour in the leaves and grass, 0 .. 1 (peaks in mid-autumn).
    pub fn autumn(&self) -> f32 {
        bell(self.season, 0.76, 0.11)
    }
    /// Bare, brown winter vegetation, 0 .. 1.
    pub fn winter(&self) -> f32 {
        bell(self.season, 0.0, 0.17).max(bell(self.season, 1.0, 0.17))
    }
    /// Fresh spring green, 0 .. 1.
    pub fn spring(&self) -> f32 {
        bell(self.season, 0.3, 0.1)
    }
    /// Moon phase, 0 .. 1 (0 new, 0.5 full).
    pub fn moon_phase(&self) -> f32 {
        (self.day / 29.5 + 0.35).rem_euclid(1.0)
    }
}

/// Weather and season forced from the keyboard (or the command line).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct EnvOverride {
    pub weather: Option<Preset>,
    /// 0 spring, 1 summer, 2 autumn, 3 winter.
    pub season: Option<u32>,
}

/// Fixed weather for looking at one kind of weather.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Clear,
    Cloudy,
    Rain,
    Storm,
    Snow,
    Fog,
}

impl Preset {
    pub const ALL: [Preset; 6] = [
        Preset::Clear,
        Preset::Cloudy,
        Preset::Rain,
        Preset::Storm,
        Preset::Snow,
        Preset::Fog,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Clear => "clear",
            Preset::Cloudy => "cloudy",
            Preset::Rain => "rain",
            Preset::Storm => "storm",
            Preset::Snow => "snow",
            Preset::Fog => "fog",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == s)
    }

    /// The weather of this preset at a given seasonal temperature.
    fn weather(self, base: Weather) -> Weather {
        let w = |cloud, precip, haze, wind: f32| Weather {
            cloud,
            precip,
            haze,
            wind: base.wind.normalize_or(Vec2::X) * wind,
            ..base
        };
        match self {
            Preset::Clear => w(0.05, 0.0, 0.0, 2.0),
            Preset::Cloudy => w(0.75, 0.0, 0.1, 5.0),
            Preset::Rain => Weather {
                temperature: base.temperature.max(6.0),
                ..w(0.9, 0.55, 0.25, 6.0)
            },
            Preset::Storm => Weather {
                temperature: base.temperature.max(8.0),
                ..w(1.0, 1.0, 0.35, 14.0)
            },
            Preset::Snow => Weather {
                temperature: base.temperature.min(-4.0),
                ..w(0.85, 0.6, 0.2, 3.0)
            },
            Preset::Fog => w(0.6, 0.0, 1.0, 0.5),
        }
    }
}

pub const SEASON_NAMES: [&str; 4] = ["spring", "summer", "autumn", "winter"];
/// The `season` value in the middle of each named season.
const SEASON_MID: [f32; 4] = [0.25, 0.5, 0.75, 0.0];

pub fn env_keys(keys: Res<ButtonInput<KeyCode>>, mut over: ResMut<EnvOverride>) {
    if keys.just_pressed(KeyCode::KeyY) {
        over.weather = match over.weather {
            None => Some(Preset::ALL[0]),
            Some(p) => {
                let i = Preset::ALL.iter().position(|&q| q == p).unwrap_or(0);
                Preset::ALL.get(i + 1).copied()
            }
        };
    }
    if keys.just_pressed(KeyCode::KeyU) {
        over.season = match over.season {
            None => Some(0),
            Some(3) => None,
            Some(s) => Some(s + 1),
        };
    }
}

/// Once per frame: the environment from the clock (and, later, the sim).
pub fn update_environment(
    over: Res<EnvOverride>,
    mut clock: ResMut<SkyClock>,
    mut env: ResMut<Environment>,
    mut last: Local<Option<(f32, f32)>>,
) {
    // A forced season moves the sun too, so it goes into the clock.
    if let Some(s) = over.season {
        let want = SEASON_MID[s as usize % 4];
        if clock.season != want {
            clock.season = want;
        }
    }
    // Days passed since the last frame: the hour (wrapping at midnight) plus
    // the calendar (the season, wrapping at new year).
    let wrap = |x: f32, period: f32| (x + period * 0.5).rem_euclid(period) - period * 0.5;
    let dt_days = match *last {
        Some((hour, season)) => {
            wrap(clock.hour - hour, 24.0) / 24.0 + wrap(clock.season - season, 1.0) * DAYS_PER_YEAR
        }
        None => 0.0,
    };
    *last = Some((clock.hour, clock.season));
    // Jumps (the season set by hand, fast-forward) settle the ground at once.
    let first = env.day == 0.0;
    let jump = first || over.is_changed() || !(-0.5..=1.5).contains(&dt_days);
    let mut next = env.clone();
    next.day = if first {
        clock.season * DAYS_PER_YEAR + clock.hour / 24.0 + 1.0
    } else {
        env.day + dt_days.max(0.0)
    };
    next.hour = clock.hour;
    next.season = clock.season;
    let base = stub_weather(next.day, next.season, next.hour);
    next.weather = match over.weather {
        Some(p) => p.weather(base),
        None => base,
    };
    next.ground = if jump {
        settled_ground(&next.weather, next.season)
    } else {
        step_ground(env.ground, &next.weather, dt_days.max(0.0))
    };
    if *env != next {
        *env = next;
    }
}
/// Days in the stub's year (only for turning the season into days).
const DAYS_PER_YEAR: f32 = 360.0;

/// The stand-in weather: a seasonal temperature with a daily swing, and slow
/// noise for clouds, rain and wind. Deterministic in (day, season, hour).
pub fn stub_weather(day: f32, season: f32, hour: f32) -> Weather {
    let seasonal = 9.0 - 12.0 * (season * TAU).cos(); // -3 in midwinter, 21 in midsummer
    let daily = 4.0 * ((hour - 9.0) / 24.0 * TAU).sin(); // warmest mid-afternoon
    let cloud = smooth(0.25, 0.85, noise1(day / 2.3, 1));
    let precip = smooth(0.62, 0.9, cloud * 0.7 + noise1(day / 0.9, 2) * 0.3);
    let angle = noise1(day / 5.0, 3) * TAU * 1.5;
    let speed = 1.5 + 7.0 * noise1(day / 1.7, 4) + 6.0 * precip;
    let haze = smooth(0.7, 1.0, noise1(day / 3.1, 5))
        * smooth(10.0, 4.0, hour.min(24.0 - hour).abs() + 4.0);
    Weather {
        temperature: seasonal + daily + (noise1(day / 4.0, 6) - 0.5) * 8.0 - cloud * 2.0,
        wind: Vec2::new(angle.cos(), angle.sin()) * speed,
        cloud,
        precip,
        haze,
    }
}

/// The ground after a while in this weather and season (no history).
pub fn settled_ground(w: &Weather, season: f32) -> Ground {
    let cold = smooth(3.0, -3.0, 9.0 - 12.0 * (season * TAU).cos());
    let snowing = w.precip * w.snow_fraction();
    Ground {
        snow: (cold * 0.9 + snowing).clamp(0.0, 1.0) * smooth(4.0, 0.0, w.temperature).max(snowing),
        wet: (w.precip * (1.0 - w.snow_fraction())).clamp(0.0, 1.0),
    }
}

/// The ground `dt` days later: snow piles up while it snows and melts above
/// freezing; surfaces get wet in rain and dry out in a few hours.
pub fn step_ground(g: Ground, w: &Weather, dt: f32) -> Ground {
    let snowfall = w.precip * w.snow_fraction();
    let rain = w.precip - snowfall;
    let melt = w.temperature.max(0.0) * 0.35;
    let snow = (g.snow + (snowfall * 2.5 - melt) * dt).clamp(0.0, 1.0);
    let target = rain.min(1.0);
    let wet = if target > g.wet {
        g.wet + (target - g.wet) * (dt * 12.0).min(1.0)
    } else {
        g.wet + (target - g.wet) * (dt * 4.0).min(1.0)
    };
    Ground { snow, wet }
}

/// Smoothstep from `a` to `b` (either way round).
pub fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 1 at `centre`, falling smoothly to 0 at `width` away (on the circle of the year).
fn bell(x: f32, centre: f32, width: f32) -> f32 {
    let d = (x - centre).abs();
    let d = d.min(1.0 - d);
    smooth(width, 0.0, d)
}

/// Smooth 1D value noise in 0 .. 1.
fn noise1(x: f32, seed: u32) -> f32 {
    let h = |i: i64| {
        let mut z = (i as u64) ^ ((seed as u64) << 40) ^ 0x9E37_79B9_7F4A_7C15;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    };
    let i = x.floor();
    let f = x - i;
    let u = f * f * (3.0 - 2.0 * f);
    let i = i as i64;
    h(i) * (1.0 - u) + h(i + 1) * u
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winter_is_colder_than_summer() {
        let mean = |season: f32| {
            (0..200)
                .map(|d| stub_weather(d as f32, season, 12.0).temperature)
                .sum::<f32>()
                / 200.0
        };
        assert!(mean(0.0) < 2.0, "winter {}", mean(0.0));
        assert!(mean(0.5) > 15.0, "summer {}", mean(0.5));
    }

    #[test]
    fn snow_piles_up_and_melts() {
        let snowing = Weather {
            temperature: -5.0,
            precip: 0.8,
            ..default()
        };
        let mut g = Ground::default();
        for _ in 0..24 {
            g = step_ground(g, &snowing, 1.0 / 24.0);
        }
        assert!(g.snow > 0.9);
        let thaw = Weather {
            temperature: 8.0,
            ..default()
        };
        for _ in 0..24 {
            g = step_ground(g, &thaw, 1.0 / 24.0);
        }
        assert!(g.snow < 0.1, "{}", g.snow);
    }

    #[test]
    fn the_weather_is_varied() {
        let days: Vec<Weather> = (0..300)
            .map(|d| stub_weather(d as f32 * 0.5, 0.4, 14.0))
            .collect();
        let rainy = days.iter().filter(|w| w.precip > 0.2).count();
        let clear = days.iter().filter(|w| w.cloud < 0.3).count();
        assert!(rainy > 10 && rainy < 150, "rainy {rainy}");
        assert!(clear > 30, "clear {clear}");
    }

    #[test]
    fn seasons_peak_where_named() {
        let e = |season| Environment {
            season,
            ..default()
        };
        assert!(e(0.76).autumn() > 0.99 && e(0.5).autumn() == 0.0);
        assert!(e(0.0).winter() > 0.99 && e(0.98).winter() > 0.9);
        assert!(e(0.5).winter() == 0.0);
    }
}
