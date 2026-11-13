//! Time of day, seasons and weather.
//!
//! The clock ([`SimClock`]) cuts each day into `time.ticks_per_day` ticks. The
//! economy runs on the first tick of each day; the other ticks only move the
//! hours and the weather on (see `build_schedule`). With time compression
//! (`time.economy_days_per_day` above 1) one game day runs that many days of
//! economy, so a short calendar (12 game days a year) keeps the economy's
//! rates per day.
//!
//! The weather is a pure function of the world seed, the time and the place:
//! seeded value noise drifting across the map makes weather fronts, so towns
//! close together share a cold snap or a wet week, and a run replays exactly.
//! Only the ground (snow cover, wet soil) keeps state from tick to tick.
//!
//! [`Environment`] mirrors the stand-in that `game3d` draws from
//! (`game3d/src/env.rs`): hour, season (0 = midwinter, 0.5 = midsummer),
//! continuous day, weather and ground, so the view can switch to the sim's.

use crate::world::{Params, SimClock, SimSeed, Towns};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

// ---------------------------------------------------------------- parameters

/// `time` in models.ron: how finely a day is cut, and how long months and years are.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeParams {
    /// Ticks per day (6, 8, 12 or 24 are sensible; 1 = the old daily clock).
    pub ticks_per_day: u32,
    pub days_per_month: u32,
    pub months_per_year: u32,
    /// Where in the seasons the calendar year starts (0 = midwinter,
    /// 0.25 = spring equinox).
    #[serde(default = "spring")]
    pub year_starts_at: f32,
    #[serde(default)]
    pub day_length: DayLength,
    /// Days of economy that one game day stands for (time compression). With
    /// 12 game days a year and 10 here, the economy still has 120 days a year.
    #[serde(default = "one")]
    pub economy_days_per_day: u32,
    /// How long people work at outdoor jobs.
    #[serde(default)]
    pub work_hours: WorkHours,
}

fn one() -> u32 {
    1
}

/// Hours of outdoor work (fields, forests, mines, building, clearing land).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WorkHours {
    /// A full working day all year, whatever the light (the old model).
    #[default]
    AllDay,
    /// Outdoor work goes on while it is light: a day's work at the equinox,
    /// two thirds of it in midwinter (8 hours against 12), a third more in
    /// midsummer. Indoor work keeps a full day.
    Daylight,
}

fn spring() -> f32 {
    0.25
}

impl Default for TimeParams {
    fn default() -> Self {
        TimeParams {
            ticks_per_day: 1,
            days_per_month: 10,
            months_per_year: 12,
            year_starts_at: spring(),
            day_length: DayLength::default(),
            economy_days_per_day: 1,
            work_hours: WorkHours::AllDay,
        }
    }
}

impl TimeParams {
    /// Game days per year.
    pub fn game_days_per_year(&self) -> u32 {
        self.days_per_month * self.months_per_year
    }

    /// Days of economy per year (what `life.days_per_year` must be).
    pub fn days_per_year(&self) -> u32 {
        self.game_days_per_year() * self.economy_days_per_day.max(1)
    }
}

/// Hours of daylight through the year.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayLength {
    /// Sun up to sun down at the equinoxes.
    pub equinox_hours: f32,
    /// How much longer midsummer is (and midwinter shorter), in hours.
    pub swing_hours: f32,
    /// Dawn before sunrise and dusk after sunset, each this long.
    pub twilight_hours: f32,
}

impl Default for DayLength {
    fn default() -> Self {
        DayLength {
            equinox_hours: 12.0,
            swing_hours: 4.0,
            twilight_hours: 2.0,
        }
    }
}

/// `climate` in models.ron: the averages the weather varies around.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClimateParams {
    /// The same temperature always and no rain (tests).
    Constant { temperature: f32 },
    Seasonal {
        /// Yearly mean air temperature, °C.
        mean_c: f32,
        /// Midsummer is this much warmer than the mean, midwinter this much colder.
        seasonal_swing_c: f32,
        /// Afternoon to pre-dawn difference on a clear day, °C.
        daily_swing_c: f32,
        /// Share of the time it rains or snows, over the year.
        wet: f32,
        /// Season of the wettest weather (0 = midwinter, 0.75 = autumn equinox).
        wet_peak: f32,
        /// How much wetter the wettest season is than the driest (0 = even).
        wet_swing: f32,
    },
}

impl Default for ClimateParams {
    fn default() -> Self {
        ClimateParams::Constant { temperature: 15.0 }
    }
}

/// `weather` in models.ron: how the weather varies around the climate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum WeatherParams {
    /// Always the climate's average: no fronts, no wind, no rain.
    #[default]
    Clear,
    /// Seeded fronts that drift across the map.
    Fronts {
        /// Size of a weather front, in world units (the map is about 2000 across).
        front_size: f32,
        /// Days a front takes to form and break up.
        front_days: f32,
        /// How far the prevailing wind carries fronts per day, world units.
        drift_per_day: f32,
        /// Typical departure from the seasonal temperature (warm or cold spells), °C.
        anomaly_c: f32,
        /// Days a warm or cold spell lasts.
        anomaly_days: f32,
        /// Typical wind speed, m/s.
        wind_mean: f32,
    },
}

// ---------------------------------------------------------------- state

/// Part of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DayPhase {
    #[default]
    Night,
    Dawn,
    Day,
    Dusk,
}

/// The weather at one place and time (fields as in `game3d`'s stand-in).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Weather {
    /// Air temperature, °C.
    pub temperature: f32,
    /// Where the wind blows toward (world x, y), m/s.
    pub wind: (f32, f32),
    /// Cloud cover, 0 (clear) .. 1 (overcast).
    pub cloud: f32,
    /// Precipitation, 0 (dry) .. 1 (downpour). Snow when it is cold.
    pub precip: f32,
    /// Mist and fog, 0 .. 1.
    pub haze: f32,
}

impl Default for Weather {
    fn default() -> Self {
        Weather {
            temperature: 15.0,
            wind: (0.0, 0.0),
            cloud: 0.0,
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

    pub fn wind_speed(&self) -> f32 {
        self.wind.0.hypot(self.wind.1)
    }

    /// Heavy rain or snow with strong wind: stops outdoor work.
    pub fn storm(&self) -> bool {
        self.precip > 0.7 && self.wind_speed() > 9.0
    }
}

/// What the weather has done to the ground; it changes slowly.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Ground {
    /// Snow cover, 0 .. 1.
    pub snow: f32,
    /// Wet soil and roads, 0 .. 1.
    pub wet: f32,
}

/// Weather and ground at one town.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct LocalEnv {
    pub weather: Weather,
    pub ground: Ground,
    /// Today's mean air temperature (pre-dawn low and afternoon high), °C.
    #[serde(default)]
    pub day_mean_c: f32,
}

/// Time of day, season and the weather at every town, for this tick.
#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize)]
pub struct Environment {
    /// Hour of the day at the middle of this tick, 0 .. 24.
    pub hour: f32,
    /// Fraction of the year, 0 .. 1 (0 = midwinter, 0.5 = midsummer).
    pub season: f32,
    /// Days since the start (continuous).
    pub day: f64,
    pub phase: DayPhase,
    /// Sunrise and sunset today, hours.
    pub sunrise: f32,
    pub sunset: f32,
    /// Per town, in town order.
    pub towns: Vec<LocalEnv>,
}

impl Environment {
    pub fn at(&self, town: usize) -> LocalEnv {
        self.towns.get(town).copied().unwrap_or_default()
    }

    /// Light enough for outdoor work: full daylight, not dawn or dusk.
    pub fn daylight(&self) -> bool {
        self.phase == DayPhase::Day
    }
}

// ---------------------------------------------------------------- the model

/// Hour at the middle of the clock's current tick.
pub fn hour(clock: &SimClock) -> f32 {
    let tpd = clock.ticks_per_day.max(1) as f32;
    (clock.sub as f32 + 0.5) * 24.0 / tpd
}

/// Game days since the start, continuous: game day 1 starts at 0.
pub fn continuous_day(clock: &SimClock) -> f64 {
    (clock.game_day.max(1) - 1) as f64 + hour(clock) as f64 / 24.0
}

/// Fraction of the year, 0 = midwinter, 0.5 = midsummer.
pub fn season(clock: &SimClock, time: &TimeParams) -> f32 {
    let dpy = (clock.days_per_year / clock.economy_days_per_day.max(1)).max(1) as f64;
    let f = continuous_day(clock) / dpy + time.year_starts_at as f64;
    (f - f.floor()) as f32
}

/// The climate's mean temperature at a point of the year.
pub fn climate_c(climate: &ClimateParams, season: f32) -> f32 {
    match *climate {
        ClimateParams::Constant { temperature } => temperature,
        ClimateParams::Seasonal {
            mean_c,
            seasonal_swing_c,
            ..
        } => mean_c - seasonal_swing_c * (std::f32::consts::TAU * season).cos(),
    }
}

/// Sunrise and sunset hours at this point of the year.
pub fn sun_times(season: f32, d: &DayLength) -> (f32, f32) {
    let light = (d.equinox_hours - d.swing_hours * (TAU * season).cos()).clamp(0.0, 24.0);
    (12.0 - light / 2.0, 12.0 + light / 2.0)
}

pub fn phase(hour: f32, sunrise: f32, sunset: f32, twilight: f32) -> DayPhase {
    if hour >= sunrise && hour < sunset {
        DayPhase::Day
    } else if hour >= sunrise - twilight && hour < sunrise {
        DayPhase::Dawn
    } else if hour >= sunset && hour < sunset + twilight {
        DayPhase::Dusk
    } else {
        DayPhase::Night
    }
}

/// The weather at world position `pos`, `day` days (continuous) into the run.
pub fn weather_at(
    climate: &ClimateParams,
    weather: &WeatherParams,
    seed: u64,
    season: f32,
    day: f64,
    hour: f32,
    pos: (f32, f32),
) -> Weather {
    let (mean_c, seasonal_swing, daily_swing, wet, wet_peak, wet_swing) = match *climate {
        ClimateParams::Constant { temperature } => {
            return Weather {
                temperature,
                ..Weather::default()
            };
        }
        ClimateParams::Seasonal {
            mean_c,
            seasonal_swing_c,
            daily_swing_c,
            wet,
            wet_peak,
            wet_swing,
        } => (
            mean_c,
            seasonal_swing_c,
            daily_swing_c,
            wet,
            wet_peak,
            wet_swing,
        ),
    };
    let seasonal = mean_c - seasonal_swing * (TAU * season).cos();
    // Warmest in mid-afternoon, coldest before dawn.
    let daily =
        |cloud: f32| 0.5 * daily_swing * (TAU * (hour - 15.0) / 24.0).cos() * (1.0 - 0.6 * cloud);
    let WeatherParams::Fronts {
        front_size,
        front_days,
        drift_per_day,
        anomaly_c,
        anomaly_days,
        wind_mean,
    } = *weather
    else {
        return Weather {
            temperature: seasonal + daily(0.0),
            ..Weather::default()
        };
    };

    let t = day as f32;
    // Prevailing wind from the west: fronts drift toward +x.
    let fx = (pos.0 - drift_per_day * t) / front_size.max(1.0);
    let fy = pos.1 / front_size.max(1.0);
    let ft = t / front_days.max(0.1);
    // Two layers of noise, one drifting, one changing in place, so fronts both
    // move and form or break up.
    let front = 0.6 * noise3(seed ^ 0x51, fx, fy, ft)
        + 0.4 * noise3(seed ^ 0x52, fx * 2.1, fy * 2.1, ft * 1.7);
    // Wetter in the wet season; how much of the field is over the rain line.
    let wet_now = (wet * (1.0 + wet_swing * (TAU * (season - wet_peak)).cos())).clamp(0.0, 0.95);
    let rain_line = quantile_of_noise(1.0 - wet_now);
    let cloud = smooth(rain_line - 0.25, rain_line, front);
    let precip = smooth(rain_line, rain_line + 0.15, front);

    let anomaly = anomaly_c
        * 2.0
        * (noise3(seed ^ 0x53, fx * 0.5, fy * 0.5, t / anomaly_days.max(0.1)) - 0.5)
        * 1.7;
    let temperature = seasonal + daily(cloud) + anomaly - 2.0 * precip;

    let angle = TAU * noise3(seed ^ 0x54, 0.0, 0.0, t / front_days.max(0.1));
    let gust = 0.4 + 1.2 * noise3(seed ^ 0x55, fx, fy, t * 3.0) + 1.2 * precip;
    let speed = wind_mean * gust;
    // Mostly westerly: blend the drifting direction with the prevailing one.
    let (dx, dy) = (0.6 + 0.8 * angle.cos(), 0.8 * angle.sin());
    let norm = dx.hypot(dy).max(1e-3);
    let wind = (speed * dx / norm, speed * dy / norm);

    // Mist on calm, damp, cool mornings.
    let morning = smooth(3.0, 6.0, hour) * smooth(11.0, 8.0, hour);
    let calm = smooth(wind_mean, 0.3 * wind_mean, speed);
    let haze = (morning * calm * (0.4 + 0.6 * cloud) * (1.0 - precip)).clamp(0.0, 1.0);

    Weather {
        temperature,
        wind,
        cloud,
        precip,
        haze,
    }
}

/// Snow and wet ground after `days` (fraction of a day) of this weather.
pub fn ground_step(ground: Ground, w: &Weather, days: f32) -> Ground {
    let snow_part = w.snow_fraction();
    // A day of steady snow covers the ground; a day well above freezing melts it.
    let snow = ground.snow + days * (w.precip * snow_part * 1.5 - 0.12 * w.temperature.max(0.0));
    // Rain soaks the ground in half a day; it dries over about three days in
    // mild weather, faster when warm, hardly at all when freezing.
    let dry = 0.33 * (1.0 + w.temperature.max(0.0) / 15.0) * (1.0 - w.cloud * 0.5);
    let wet = ground.wet
        + days * (w.precip * (1.0 - snow_part) * 2.0 - dry * (w.temperature > 0.0) as u8 as f32);
    Ground {
        snow: snow.clamp(0.0, 1.0),
        wet: wet.clamp(0.0, 1.0),
    }
}

/// Moves the clock on by one tick. A new game day starts when the last tick
/// of the old one has run; the economy runs on that first tick.
pub fn advance(mut clock: ResMut<SimClock>) {
    let tpd = clock.ticks_per_day.max(1);
    if clock.sub + 1 >= tpd {
        clock.sub = 0;
        clock.game_day += 1;
    } else {
        clock.sub += 1;
    }
}

/// Starts a day of economy (the first system of the economy schedule).
pub fn new_economy_day(mut clock: ResMut<SimClock>) {
    clock.tick += 1;
}

/// On the first tick of a game day, runs `time.economy_days_per_day` days of
/// economy (the `EconomyDay` schedule).
pub fn run_economy(world: &mut World) {
    let clock = *world.resource::<SimClock>();
    if clock.sub != 0 {
        return;
    }
    for _ in 0..clock.economy_days_per_day.max(1) {
        world.run_schedule(crate::EconomyDay);
    }
}

/// How much of a full day's work an outdoor job gets done today (1 = a full
/// day). Indoor jobs always get 1.
pub fn outdoor_work_share(env: &Environment, time: &TimeParams) -> f32 {
    match time.work_hours {
        WorkHours::AllDay => 1.0,
        WorkHours::Daylight => {
            ((env.sunset - env.sunrise) / time.day_length.equinox_hours.max(1.0)).max(0.0)
        }
    }
}

/// Works out the hour, season and the weather at every town for this tick.
pub fn update(
    clock: Res<SimClock>,
    params: Res<Params>,
    seed: Res<SimSeed>,
    towns: Res<Towns>,
    mut env: ResMut<Environment>,
) {
    let time = &params.time;
    let h = hour(&clock);
    let s = season(&clock, time);
    let day = continuous_day(&clock);
    let (sunrise, sunset) = sun_times(s, &time.day_length);
    env.hour = h;
    env.season = s;
    env.day = day;
    env.sunrise = sunrise;
    env.sunset = sunset;
    env.phase = phase(h, sunrise, sunset, time.day_length.twilight_hours);
    let dt = 1.0 / clock.ticks_per_day.max(1) as f32;
    let n = towns.0.len();
    env.towns.resize(n, LocalEnv::default());
    for (i, town) in towns.0.iter().enumerate() {
        let w = weather_at(
            &params.climate,
            &params.weather,
            seed.0,
            s,
            day,
            h,
            town.pos,
        );
        let ground = ground_step(env.towns[i].ground, &w, dt);
        let at = |hour: f32| {
            weather_at(
                &params.climate,
                &params.weather,
                seed.0,
                s,
                day.floor() + hour as f64 / 24.0,
                hour,
                town.pos,
            )
            .temperature
        };
        let day_mean_c = 0.5 * (at(4.0) + at(15.0));
        env.towns[i] = LocalEnv {
            weather: w,
            ground,
            day_mean_c,
        };
    }
}

// ---------------------------------------------------------------- helpers

/// 0 at `a`, 1 at `b` (either order), smooth in between.
pub fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lattice(seed: u64, i: i64, j: i64, k: i64) -> f32 {
    let mut h = seed
        ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (k as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Smooth value noise in 3D, in [0, 1].
fn noise3(seed: u64, x: f32, y: f32, z: f32) -> f32 {
    let (i, j, k) = (x.floor() as i64, y.floor() as i64, z.floor() as i64);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy, sz) = (s(x - x.floor()), s(y - y.floor()), s(z - z.floor()));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let plane = |k: i64| {
        let top = lerp(lattice(seed, i, j, k), lattice(seed, i + 1, j, k), sx);
        let bottom = lerp(
            lattice(seed, i, j + 1, k),
            lattice(seed, i + 1, j + 1, k),
            sx,
        );
        lerp(top, bottom, sy)
    };
    lerp(plane(k), plane(k + 1), sz)
}

/// The value of our blended noise that a share `p` of all samples fall below.
/// Value noise clusters around 0.5, so a plain threshold at `p` would make
/// rain far rarer than `wet` asks for. Fitted to the noise's distribution
/// (see the test `wet_share_matches_climate`).
fn quantile_of_noise(p: f32) -> f32 {
    // The blend is close to normal with mean 0.5 and sd about 0.14.
    0.5 + 0.14 * probit(p.clamp(0.001, 0.999))
}

/// Inverse of the standard normal distribution (Acklam's approximation).
fn probit(p: f32) -> f32 {
    let p = p as f64;
    let a = [
        -39.696_830_286_653_76,
        220.946_098_424_520_5,
        -275.928_510_446_968_7,
        138.357_751_867_269,
        -30.664_798_066_147_16,
        2.506_628_277_459_239,
    ];
    let b = [
        -54.476_098_798_224_06,
        161.585_836_858_040_9,
        -155.698_979_859_886_6,
        66.801_311_887_719_72,
        -13.280_681_552_885_72,
    ];
    let c = [
        -0.007_784_894_002_430_293,
        -0.322_396_458_041_136_5,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    let d = [
        0.007_784_695_709_041_462,
        0.322_467_129_070_039_8,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    let lo = 0.02425;
    let x = if p < lo {
        let q = (-2.0 * p.ln()).sqrt();
        (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
            / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    } else if p <= 1.0 - lo {
        let q = p - 0.5;
        let r = q * q;
        (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q
            / (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
            / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    };
    x as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temperate() -> (ClimateParams, WeatherParams) {
        (
            ClimateParams::Seasonal {
                mean_c: 10.0,
                seasonal_swing_c: 10.0,
                daily_swing_c: 8.0,
                wet: 0.3,
                wet_peak: 0.75,
                wet_swing: 0.3,
            },
            WeatherParams::Fronts {
                front_size: 1500.0,
                front_days: 4.0,
                drift_per_day: 300.0,
                anomaly_c: 3.0,
                anomaly_days: 5.0,
                wind_mean: 4.0,
            },
        )
    }

    #[test]
    fn day_length_and_phases() {
        let d = DayLength::default();
        let (rise, set) = sun_times(0.25, &d);
        assert!((rise - 6.0).abs() < 1e-4 && (set - 18.0).abs() < 1e-4);
        let (rise, set) = sun_times(0.0, &d);
        assert!(
            (set - rise - 8.0).abs() < 1e-4,
            "midwinter has 8 hours of light"
        );
        let (rise, set) = sun_times(0.5, &d);
        assert!(
            (set - rise - 16.0).abs() < 1e-4,
            "midsummer has 16 hours of light"
        );
        let (rise, set) = sun_times(0.25, &d);
        assert_eq!(phase(3.0, rise, set, 2.0), DayPhase::Night);
        assert_eq!(phase(5.0, rise, set, 2.0), DayPhase::Dawn);
        assert_eq!(phase(12.0, rise, set, 2.0), DayPhase::Day);
        assert_eq!(phase(19.0, rise, set, 2.0), DayPhase::Dusk);
        assert_eq!(phase(21.0, rise, set, 2.0), DayPhase::Night);
    }

    #[test]
    fn winter_is_colder_than_summer() {
        let (c, w) = temperate();
        let mean = |season: f32| {
            (0..200)
                .map(|d| {
                    weather_at(&c, &w, 7, season, d as f64 * 3.1, 12.0, (0.0, 0.0)).temperature
                })
                .sum::<f32>()
                / 200.0
        };
        let (winter, summer) = (mean(0.0), mean(0.5));
        assert!(
            winter < 3.0 && summer > 17.0,
            "winter {winter}, summer {summer}"
        );
    }

    #[test]
    fn wet_share_matches_climate() {
        let (c, w) = temperate();
        // Over a year and many places the wet share should be close to `wet`.
        let mut wet = 0;
        let mut n = 0;
        for d in 0..400 {
            for p in 0..5 {
                let season = (d as f32 / 400.0).fract();
                let x = weather_at(
                    &c,
                    &w,
                    11,
                    season,
                    d as f64 * 0.77,
                    12.0,
                    (p as f32 * 900.0, 0.0),
                );
                wet += (x.precip > 0.05) as u32;
                n += 1;
            }
        }
        let share = wet as f32 / n as f32;
        assert!((0.15..0.45).contains(&share), "wet share {share}");
    }

    #[test]
    fn neighbours_share_weather_more_than_distant_towns() {
        let (c, w) = temperate();
        let corr = |dx: f32| {
            let xs: Vec<(f32, f32)> = (0..300)
                .map(|d| {
                    let a = weather_at(&c, &w, 3, 0.3, d as f64 * 0.5, 12.0, (0.0, 0.0));
                    let b = weather_at(&c, &w, 3, 0.3, d as f64 * 0.5, 12.0, (dx, 0.0));
                    (a.cloud, b.cloud)
                })
                .collect();
            let n = xs.len() as f32;
            let (ma, mb) = (
                xs.iter().map(|x| x.0).sum::<f32>() / n,
                xs.iter().map(|x| x.1).sum::<f32>() / n,
            );
            let cov: f32 = xs.iter().map(|x| (x.0 - ma) * (x.1 - mb)).sum();
            let va: f32 = xs.iter().map(|x| (x.0 - ma).powi(2)).sum();
            let vb: f32 = xs.iter().map(|x| (x.1 - mb).powi(2)).sum();
            cov / (va * vb).sqrt().max(1e-6)
        };
        let (near, far) = (corr(100.0), corr(6000.0));
        assert!(near > 0.8 && near > far + 0.3, "near {near}, far {far}");
    }

    #[test]
    fn snow_builds_in_cold_wet_weather_and_melts_in_warm() {
        let cold = Weather {
            temperature: -5.0,
            precip: 0.8,
            ..Weather::default()
        };
        let mut g = Ground::default();
        for _ in 0..12 {
            g = ground_step(g, &cold, 1.0 / 12.0);
        }
        assert!(g.snow > 0.9, "a day of snow covers the ground: {}", g.snow);
        let warm = Weather {
            temperature: 10.0,
            ..Weather::default()
        };
        for _ in 0..12 {
            g = ground_step(g, &warm, 1.0 / 12.0);
        }
        assert!(g.snow < 0.1, "a warm day melts it: {}", g.snow);
    }
}
