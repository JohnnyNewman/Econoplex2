//! The year and the weather on the 2D map, as the 3D client shows them
//! (docs/design/client-parity.md, groups 2 and 3):
//!
//! - **Ground:** the terrain image tinted by the season (autumn colour, dull
//!   winter), each cell by its nearest town's snow cover and wet soil;
//!   rivers freeze pale when that town's day is below freezing.
//! - **Fields:** crop sites change colour through the farming year
//!   (ploughed, green, gold, stubble). Each town's harvest is reaped nearest
//!   first, by how far its standing crop fell below this year's peak; a gold
//!   ring around the town shows the share in, and the `T` report says it.
//! - **Weather:** rain or snow falls over the view by the weather of the
//!   town nearest the camera's centre.
//! - **Night:** a dark veil from dusk to dawn, at the speeds that let the
//!   hours be seen (up to 12 s a day).
//! - **Why? rings:** with the `Y` report open, the buildings its findings
//!   name pulse in red.
//!
//! All of it only reads the simulation.

use crate::reports::{Report, Reports};
use crate::terrain_view::{Cell, TerrainLook};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::components::{NaturalResource, Pos, Product, Structure, TownId};
use sim_core::env::Environment;
use sim_core::{Db, SimClock, Towns};

/// Each town's standing crop and this year's peak of it, once a day.
#[derive(Resource, Default)]
pub struct FieldYear {
    pub crops: Vec<f32>,
    pub peak: Vec<f32>,
    season: f32,
    day: Option<u64>,
}

impl FieldYear {
    /// Share of town `t`'s harvest in, 0..1.
    pub fn reaped(&self, t: usize, season: f32) -> f32 {
        reaped(
            self.crops.get(t).copied().unwrap_or(0.0),
            self.peak.get(t).copied().unwrap_or(0.0),
            season,
        )
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The harvest by the calendar alone: none before late summer, all in by
/// mid-autumn.
pub fn calendar_reaped(season: f32) -> f32 {
    smoothstep(0.55, 0.7, season)
}

/// Share of a town's fields reaped: how far its standing crop fell below this
/// year's peak, once the peak is past (after midsummer). With no crop worth
/// the name ever standing, the calendar's. (The same rule as the 3D client's
/// fields.rs.)
pub fn reaped(crops: f32, peak: f32, season: f32) -> f32 {
    if peak < 0.02 {
        return calendar_reaped(season);
    }
    if season < 0.45 {
        return 0.0;
    }
    (1.0 - crops / peak).clamp(0.0, 1.0)
}

/// Crop kinds whose sites go through the farming year's colours (the crops
/// of `farming.crops`, but not the pasture, which is grazed, not reaped).
fn staged(db: &Db, params: &sim_core::world::Params) -> Vec<bool> {
    db.content
        .nature
        .iter()
        .map(|k| k.id != "pasture" && params.farming.crops.contains(&k.id))
        .collect()
}

/// Per town, the crop standing on its fields as a share of a year's growth
/// (as `sim_core::view`'s `TownView::crops`).
fn crops_by_town(
    towns: &Towns,
    params: &sim_core::world::Params,
    is_crop: &[bool],
    sites: &Query<&NaturalResource>,
) -> Vec<f32> {
    let cal = params.calendar();
    let year_days = cal.days() * cal.per_year as f32;
    towns
        .0
        .iter()
        .map(|t| {
            let (mut standing, mut year) = (0.0, 0.0);
            for &e in &t.nature_sites {
                let Ok(n) = sites.get(e) else { continue };
                if is_crop.get(n.kind as usize).copied().unwrap_or(false) {
                    standing += n.amount;
                    year += n.regrowth * year_days;
                }
            }
            if year > 0.0 {
                standing / year
            } else {
                0.0
            }
        })
        .collect()
}

/// Once a simulated day: the standing crop and its peak per town (the peak
/// starts over when the year turns).
pub fn update_field_year(
    clock: Res<SimClock>,
    env: Option<Res<Environment>>,
    towns: Res<Towns>,
    db: Res<Db>,
    params: Res<sim_core::world::Params>,
    sites: Query<&NaturalResource>,
    mut year: ResMut<FieldYear>,
) {
    let Some(env) = env else { return };
    if year.day == Some(clock.tick) {
        return;
    }
    year.day = Some(clock.tick);
    let is_crop: Vec<bool> = db
        .content
        .nature
        .iter()
        .map(|k| params.farming.crops.contains(&k.id))
        .collect();
    let crops = crops_by_town(&towns, &params, &is_crop, &sites);
    if env.season < year.season - 0.5 {
        year.peak.clear();
    }
    year.season = env.season;
    year.peak.resize(crops.len(), 0.0);
    for (p, &c) in year.peak.iter_mut().zip(&crops) {
        *p = p.max(c);
    }
    year.crops = crops;
}

const PLOUGHED: [f32; 3] = [0.36, 0.27, 0.18];
const SPROUT: [f32; 3] = [0.40, 0.62, 0.25];
const GREEN: [f32; 3] = [0.22, 0.55, 0.18];
const GOLD: [f32; 3] = [0.88, 0.74, 0.26];
const STUBBLE: [f32; 3] = [0.66, 0.58, 0.38];
const SNOW: [f32; 3] = [0.90, 0.92, 0.95];

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// A crop site's colour: ploughed in winter, sprouting and green in spring,
/// gold by late summer, stubble once reaped, ploughed again in late autumn.
pub fn crop_color(season: f32, reaped: bool) -> [f32; 3] {
    if season >= 0.85 || season < 0.15 {
        return PLOUGHED;
    }
    if reaped {
        return mix(STUBBLE, PLOUGHED, smoothstep(0.75, 0.85, season));
    }
    if season < 0.35 {
        mix(PLOUGHED, SPROUT, smoothstep(0.15, 0.3, season))
    } else if season < 0.45 {
        mix(SPROUT, GREEN, smoothstep(0.35, 0.45, season))
    } else {
        mix(GREEN, GOLD, smoothstep(0.45, 0.6, season))
    }
}

/// Colour each crop site by the farming year, once a day: a town's sites
/// nearest to it are reaped first. Snow lies on them like on the ground.
#[allow(clippy::too_many_arguments)]
pub fn color_fields(
    clock: Res<SimClock>,
    env: Option<Res<Environment>>,
    towns: Res<Towns>,
    db: Res<Db>,
    params: Res<sim_core::world::Params>,
    year: Res<FieldYear>,
    mut sites: Query<(&NaturalResource, &Pos, &mut Sprite)>,
    mut done: Local<Option<u64>>,
) {
    let Some(env) = env else { return };
    // Every site has its sprite once the view has run a frame.
    if *done == Some(clock.tick) || sites.iter().next().is_none() {
        return;
    }
    *done = Some(clock.tick);
    let stage = staged(&db, &params);
    for (t, town) in towns.0.iter().enumerate() {
        let mut own: Vec<(Entity, f32)> = town
            .nature_sites
            .iter()
            .filter_map(|&e| {
                let (n, p, _) = sites.get(e).ok()?;
                stage
                    .get(n.kind as usize)
                    .copied()
                    .unwrap_or(false)
                    .then(|| (e, (p.x - town.pos.0).hypot(p.y - town.pos.1)))
            })
            .collect();
        own.sort_by(|a, b| a.1.total_cmp(&b.1));
        let cut = (year.reaped(t, env.season) * own.len() as f32).round() as usize;
        let snow = env.at(t).ground.snow;
        for (k, (e, _)) in own.iter().enumerate() {
            let Ok((_, _, mut sprite)) = sites.get_mut(*e) else {
                continue;
            };
            let c = mix(crop_color(env.season, k < cut), SNOW, snow * 0.85);
            sprite.color = Color::srgb(c[0], c[1], c[2]);
        }
    }
}

/// How much autumn colour the land shows, 0..1.
fn autumn(season: f32) -> f32 {
    smoothstep(0.62, 0.74, season) * (1.0 - smoothstep(0.82, 0.92, season))
}

/// How dull the winter land is, 0..1.
fn winter(season: f32) -> f32 {
    smoothstep(0.88, 0.97, season).max(1.0 - smoothstep(0.05, 0.16, season))
}

/// How frozen the rivers are by a town's day mean temperature, 0..1.
pub fn ice(day_mean_c: f32) -> f32 {
    smoothstep(0.0, -4.0, day_mean_c)
}

/// One pixel of the terrain in its season and weather.
pub fn tint(base: [u8; 3], cell: Cell, season: f32, local: &sim_core::env::LocalEnv) -> [u8; 3] {
    let b = [
        base[0] as f32 / 255.0,
        base[1] as f32 / 255.0,
        base[2] as f32 / 255.0,
    ];
    let c = match cell {
        Cell::Water => mix(b, [0.72, 0.80, 0.86], ice(local.day_mean_c) * 0.9),
        Cell::Ford => mix(b, [0.72, 0.80, 0.86], ice(local.day_mean_c) * 0.7),
        Cell::Ground | Cell::Road => {
            let warm = [b[0] * 1.35, b[1] * 0.95, b[2] * 0.55];
            let mut c = mix(b, warm, autumn(season));
            let grey = (c[0] + c[1] + c[2]) / 3.0;
            c = mix(
                c,
                [grey * 1.05, grey * 1.05, grey * 1.12],
                winter(season) * 0.6,
            );
            let wet = 1.0 - 0.25 * local.ground.wet;
            c = [c[0] * wet, c[1] * wet, c[2] * wet];
            mix(c, SNOW, local.ground.snow * 0.85)
        }
    };
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Recolour the terrain image when the season, a town's snow, wet ground or
/// ice has moved enough to show (a few times a month).
pub fn tint_terrain(
    env: Option<Res<Environment>>,
    look: Option<Res<TerrainLook>>,
    mut images: ResMut<Assets<Image>>,
    mut key: Local<Vec<i32>>,
) {
    let (Some(env), Some(look)) = (env, look) else {
        return;
    };
    let mut want = vec![(env.season * 48.0) as i32];
    for l in &env.towns {
        want.push((l.ground.snow * 20.0) as i32);
        want.push((l.ground.wet * 8.0) as i32);
        want.push((ice(l.day_mean_c) * 8.0) as i32);
    }
    if *key == want && !look.is_changed() {
        return;
    }
    let Some(mut image) = images.get_mut(&look.image) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };
    *key = want;
    for (k, px) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (Some(&base), Some(&cell), Some(&town)) =
            (look.base.get(k), look.cell.get(k), look.town.get(k))
        else {
            continue;
        };
        let rgb = tint(base, cell, env.season, &env.at(town as usize));
        px[..3].copy_from_slice(&rgb);
    }
}

/// The dark of night, 0 (day) .. 1 (full night), from the hour and today's
/// sunrise and sunset, with an hour of twilight each side.
pub fn darkness(hour: f32, sunrise: f32, sunset: f32) -> f32 {
    let light = smoothstep(sunrise - 1.0, sunrise + 0.5, hour)
        * (1.0 - smoothstep(sunset - 0.5, sunset + 1.0, hour));
    1.0 - light
}

/// The veil over the map at night.
#[derive(Component)]
pub struct NightVeil;

pub fn spawn_night_veil(mut commands: Commands) {
    commands.spawn((
        NightVeil,
        Sprite::from_color(Color::srgba(0.02, 0.03, 0.08, 0.0), Vec2::splat(1.0)),
        Transform::from_xyz(0.0, 0.0, 6.0).with_scale(Vec3::new(1.0e5, 1.0e5, 1.0)),
    ));
}

/// The fastest speed (index into `SPEEDS`) at which the night shows; faster,
/// day and night would flicker.
const NIGHT_UP_TO_SPEED: usize = 2;

pub fn update_night_veil(
    env: Option<Res<Environment>>,
    control: Res<crate::SimControl>,
    mut veil: Single<&mut Sprite, With<NightVeil>>,
) {
    let dark = match env {
        Some(env) if control.speed <= NIGHT_UP_TO_SPEED => {
            darkness(env.hour, env.sunrise, env.sunset)
        }
        _ => 0.0,
    };
    let want = Color::srgba(0.02, 0.03, 0.08, 0.55 * dark);
    if veil.color != want {
        veil.color = want;
    }
}

/// A cheap repeatable "random" number in 0..1 for drop `k`.
fn hash(k: u32, salt: u32) -> f32 {
    let mut x = k.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// Rain or snow over the view, by the weather of the town nearest the
/// camera's centre: more drops for heavier precipitation, slanted by the wind.
pub fn draw_precipitation(
    mut gizmos: Gizmos,
    time: Res<Time>,
    env: Option<Res<Environment>>,
    towns: Res<Towns>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<&Transform, With<Camera2d>>,
) {
    let Some(env) = env else { return };
    let centre = camera.translation.truncate();
    let Some((t, _)) = towns.0.iter().enumerate().min_by(|a, b| {
        let d = |p: (f32, f32)| centre.distance(Vec2::new(p.0, p.1));
        d(a.1.pos).total_cmp(&d(b.1.pos))
    }) else {
        return;
    };
    let w = env.at(t).weather;
    if w.precip < 0.05 {
        return;
    }
    let zoom = camera.scale.x;
    let size = Vec2::new(window.width(), window.height()) * zoom;
    let corner = centre - size / 2.0;
    let n = (w.precip * 350.0) as u32;
    let snow = w.snow_fraction() > 0.5;
    let fall = if snow { 60.0 } else { 600.0 } * zoom;
    let wind = Vec2::new(w.wind.0, w.wind.1) * 6.0 * zoom;
    let secs = time.elapsed_secs();
    for k in 0..n {
        let x = hash(k, 1) * size.x;
        let speed = fall * (0.7 + 0.6 * hash(k, 2));
        let y = (hash(k, 3) * size.y - secs * speed).rem_euclid(size.y);
        let p = corner + Vec2::new((x + wind.x * secs * 0.1).rem_euclid(size.x), y);
        if snow {
            gizmos.circle_2d(p, 1.5 * zoom, Color::srgba(0.95, 0.96, 1.0, 0.8));
        } else {
            let tail = Vec2::new(wind.x * 0.02, 10.0) * zoom.max(0.5);
            gizmos.line_2d(p, p + tail, Color::srgba(0.65, 0.72, 0.85, 0.55));
        }
    }
}

/// A gold arc around each town for the share of its harvest in, from late
/// summer until it is all in.
pub fn draw_harvest_rings(
    mut gizmos: Gizmos,
    env: Option<Res<Environment>>,
    towns: Res<Towns>,
    year: Res<FieldYear>,
) {
    let Some(env) = env else { return };
    if !(0.45..0.9).contains(&env.season) {
        return;
    }
    for (t, town) in towns.0.iter().enumerate() {
        let r = year.reaped(t, env.season);
        if r <= 0.0 {
            continue;
        }
        let arc = std::f32::consts::TAU * r;
        let centre = Vec2::new(town.pos.0, town.pos.1);
        for k in 0..2 {
            gizmos.arc_2d(
                Isometry2d::new(centre, Rot2::radians(-arc / 2.0)),
                arc,
                92.0 + k as f32 * 1.5,
                Color::srgb(0.95, 0.78, 0.25),
            );
        }
    }
}

/// The building types the open Why? report names, for its town (reports.rs
/// fills it when it writes the report).
#[derive(Resource, Default)]
pub struct WhyRings {
    pub town: usize,
    pub kinds: Vec<String>,
}

/// While `Y` is open: a pulsing red ring around each building of the town of
/// a type its findings name.
pub fn draw_why_rings(
    mut gizmos: Gizmos,
    time: Res<Time>,
    reports: Res<Reports>,
    rings: Res<WhyRings>,
    db: Res<Db>,
    buildings: Query<(&Product, &Pos, &TownId), With<Structure>>,
) {
    if reports.open != Some(Report::Why) || rings.kinds.is_empty() {
        return;
    }
    let pulse = 1.0 + 0.15 * (time.elapsed_secs() * 4.0).sin();
    for (p, pos, town) in &buildings {
        if town.0 as usize != rings.town {
            continue;
        }
        let Some(def) = db.content.products.get(p.def as usize) else {
            continue;
        };
        if rings.kinds.contains(&def.id) {
            gizmos.circle_2d(
                Vec2::new(pos.x, pos.y),
                22.0 * pulse,
                Color::srgb(1.0, 0.35, 0.2),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_reaped_before_the_peak_and_all_once_the_crop_is_in() {
        assert_eq!(reaped(0.1, 0.3, 0.3), 0.0);
        assert!((reaped(0.4, 0.8, 0.6) - 0.5).abs() < 1e-6);
        assert_eq!(reaped(0.0, 0.8, 0.7), 1.0);
        // No crop ever standing: the calendar's harvest.
        assert_eq!(reaped(0.0, 0.0, 0.4), 0.0);
        assert!(reaped(0.0, 0.0, 0.8) > 0.99);
    }

    #[test]
    fn fields_go_through_the_farming_year() {
        assert_eq!(crop_color(0.0, false), PLOUGHED);
        assert_eq!(crop_color(0.6, false), GOLD);
        assert_eq!(crop_color(0.6, true), STUBBLE);
        let spring = crop_color(0.4, false);
        assert!(spring[1] > spring[0], "green in spring: {spring:?}");
    }

    #[test]
    fn it_is_dark_at_night_and_light_at_noon() {
        assert_eq!(darkness(12.0, 6.0, 18.0), 0.0);
        assert_eq!(darkness(0.0, 6.0, 18.0), 1.0);
        let dusk = darkness(18.0, 6.0, 18.0);
        assert!(dusk > 0.0 && dusk < 1.0, "{dusk}");
    }

    #[test]
    fn snow_whitens_the_ground_and_frost_freezes_rivers() {
        let mut local = sim_core::env::LocalEnv {
            day_mean_c: 10.0,
            ..Default::default()
        };
        let summer = tint([30, 40, 30], Cell::Ground, 0.5, &local);
        assert_eq!(summer, [30, 40, 30]);
        local.ground.snow = 1.0;
        let snowy = tint([30, 40, 30], Cell::Ground, 0.5, &local);
        assert!(snowy.iter().all(|&v| v > 180), "{snowy:?}");
        let open = tint([32, 62, 104], Cell::Water, 0.0, &local);
        assert_eq!(open, [32, 62, 104]);
        local.day_mean_c = -6.0;
        let frozen = tint([32, 62, 104], Cell::Water, 0.0, &local);
        assert!(frozen[0] > 150, "{frozen:?}");
    }
}
