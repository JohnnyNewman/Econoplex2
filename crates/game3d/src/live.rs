//! The simulation running behind the 3D view (milestone M2).
//!
//! The boundary of docs/design/graphics-ui.md, section 1:
//! - The simulation runs on **its own thread** at the game speed. It is the only
//!   code that touches the sim's ECS world.
//! - After each tick it publishes a **snapshot** (`sim_core::view::ViewSnapshot`)
//!   and appends new **events** to a queue.
//! - The view only reads snapshots and events. Player actions go back as
//!   **orders**, applied at the start of the next tick.
//!
//! The view then keeps the scene in step with the newest snapshot:
//! - buildings appear when the sim builds them and go when they are gone;
//! - each town shows one house per six residents (at least twelve);
//! - the sim's clock and weather drive day and night, seasons and weather
//!   (`publish_env`, see env.rs);
//! - a status line shows the date, the speed, the population and the latest
//!   events.
//!
//! Keys: `Space` pause, `,` / `.` slower / faster.

use crate::buildings::{spawn_building, spawn_house, BuildingSpec, Kit, Style};
use crate::env::{Ground, SimEnv, Weather};
use crate::outline::Outlines;
use crate::toon::ToonMaterial;
use crate::{height, World3d, SCALE};
use bevy::prelude::*;
use sim_core::history::{Event, Sample};
use sim_core::policy::{Order, OrderQueue};
use sim_core::view::ViewSnapshot;
use sim_core::Simulation;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Game speeds in simulated days per second. Play is about 5 minutes a day
/// (Nils's choice), so day and night pass at a watchable pace; the faster
/// speeds (1 minute, 12 s, then 1 to 120 days a second) are for letting the
/// economy run.
pub const SPEEDS: [f32; 9] = [
    1.0 / 300.0,
    1.0 / 60.0,
    1.0 / 12.0,
    1.0,
    4.0,
    12.0,
    30.0,
    60.0,
    120.0,
];
/// The speed the view starts at (5 minutes a day).
pub const DEFAULT_SPEED: usize = 0;

/// A speed as people read it: `5 min/day`, `12 s/day`, `30 days/s`.
pub fn speed_label(days_per_sec: f32) -> String {
    if days_per_sec >= 1.0 {
        format!("{days_per_sec:.0} days/s")
    } else {
        let secs = 1.0 / days_per_sec;
        if secs >= 60.0 {
            format!("{:.0} min/day", secs / 60.0)
        } else {
            format!("{secs:.0} s/day")
        }
    }
}

pub struct LivePlugin;

impl Plugin for LivePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shown>()
            .add_systems(Startup, (spawn_status, init_kit))
            .add_systems(Update, (live_keys, sync_scene, update_status).chain())
            .add_systems(Update, publish_env.before(crate::env::update_environment));
    }
}

/// What both threads share. The sim thread writes snapshots and events; the
/// view writes speed, pause and orders.
struct Shared {
    latest: Arc<ViewSnapshot>,
    events: Vec<Event>,
    speed: usize,
    paused: bool,
    orders: Vec<Order>,
    /// Orders as text (`tax Morstead wealth 0.75`), parsed on the sim thread.
    text_orders: Vec<String>,
    /// The town the view wants explained (the "why" view), if any.
    why_town: Option<u16>,
    /// History samples since the view last took them.
    samples: Vec<Sample>,
    /// Orders the simulation could not parse, with the reason.
    errors: Vec<String>,
    quit: bool,
    /// The sim's time of day and weather after its latest tick (it changes
    /// every tick; the snapshot only once a day), when that tick ran, and how
    /// long a tick lasts at the current speed (the view smooths the hour).
    env: Arc<sim_core::env::Environment>,
    env_at: Instant,
    tick_secs: f32,
    ticks_per_day: u32,
}

/// The view's handle on the running simulation.
#[derive(Resource)]
pub struct LiveSim {
    shared: Arc<Mutex<Shared>>,
    /// The newest snapshot the view has taken over.
    pub snapshot: Arc<ViewSnapshot>,
    /// The tick the scene was last brought up to date for.
    shown_tick: Option<u64>,
    /// The latest events, newest first (the status line shows four, the event
    /// log more).
    pub recent: VecDeque<Event>,
    /// The monthly history so far (one sample per town and month).
    pub history: Vec<Sample>,
    /// Product names by def, and the building types a town can commission
    /// (product id, name).
    pub product_names: Vec<String>,
    pub building_kinds: Vec<(String, String)>,
}

impl Drop for LiveSim {
    fn drop(&mut self) {
        if let Ok(mut s) = self.shared.lock() {
            s.quit = true;
        }
    }
}

impl LiveSim {
    /// Start the simulation thread. `sim` has already run any fast-forward days.
    pub fn start(mut sim: Simulation, speed: usize, paused: bool) -> Self {
        let db = sim.world.resource::<sim_core::Db>();
        let product_names = db.content.products.iter().map(|p| p.name.clone()).collect();
        let building_kinds = (0..db.content.products.len())
            .filter(|&p| db.category(p as u32) == sim_data::Category::Building)
            .filter(|&p| db.content.products[p].opens.is_none())
            .map(|p| {
                (
                    db.content.products[p].id.clone(),
                    db.content.products[p].name.clone(),
                )
            })
            .collect();
        let mut seen = sim_core::view::Seen::default();
        let first = Arc::new(ViewSnapshot::capture(&sim.world, &mut seen));
        let history = first.samples.clone();
        let shared = Arc::new(Mutex::new(Shared {
            latest: first.clone(),
            events: Vec::new(),
            speed: speed.min(SPEEDS.len() - 1),
            paused,
            orders: Vec::new(),
            text_orders: Vec::new(),
            samples: Vec::new(),
            why_town: None,
            errors: Vec::new(),
            quit: false,
            env: Arc::new(sim.world.resource::<sim_core::env::Environment>().clone()),
            env_at: Instant::now(),
            tick_secs: 1.0,
            ticks_per_day: sim
                .world
                .resource::<sim_core::SimClock>()
                .ticks_per_day
                .max(1),
        }));
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("simulation".into())
            .spawn(move || run(&mut sim, &thread_shared, seen))
            .expect("cannot start the simulation thread");
        LiveSim {
            shared,
            snapshot: first,
            shown_tick: None,
            recent: VecDeque::new(),
            history,
            product_names,
            building_kinds,
        }
    }

    /// Queue a government order for the next tick (the M5 panels will use it).
    #[allow(dead_code)]
    pub fn order(&self, order: Order) {
        self.shared.lock().unwrap().orders.push(order);
    }

    /// Queue a text order (the CLI's syntax) for the next tick.
    pub fn order_text(&self, text: String) {
        self.shared.lock().unwrap().text_orders.push(text);
    }

    /// Orders the simulation rejected since the last call.
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut self.shared.lock().unwrap().errors)
    }

    /// Ask the simulation to explain this town after each tick (or stop).
    pub fn explain(&self, town: Option<u16>) {
        self.shared.lock().unwrap().why_town = town;
    }

    /// The sim's latest time of day and weather, how far (0..1) the view is
    /// into the tick after it, and how many real seconds a day lasts now
    /// (infinite while paused).
    pub fn env(&self) -> (Arc<sim_core::env::Environment>, f32, f32, u32) {
        let s = self.shared.lock().unwrap();
        let frac = if s.paused {
            0.5
        } else {
            (s.env_at.elapsed().as_secs_f32() / s.tick_secs.max(1e-4)).min(1.0)
        };
        let day_secs = if s.paused {
            f32::INFINITY
        } else {
            1.0 / SPEEDS[s.speed]
        };
        (s.env.clone(), frac, day_secs, s.ticks_per_day)
    }

    pub fn speed_and_pause(&self) -> (usize, bool) {
        let s = self.shared.lock().unwrap();
        (s.speed, s.paused)
    }
}

/// The simulation thread: step at the game speed, publish after every tick.
fn run(sim: &mut Simulation, shared: &Mutex<Shared>, mut seen: sim_core::view::Seen) {
    let mut next = Instant::now();
    // The town last explained, so a new choice is explained even while paused.
    let mut explained: Option<u16> = None;
    loop {
        let (speed, paused, orders, texts, quit) = {
            let mut s = shared.lock().unwrap();
            (
                s.speed,
                s.paused,
                std::mem::take(&mut s.orders),
                std::mem::take(&mut s.text_orders),
                s.quit,
            )
        };
        if quit {
            return;
        }
        // Orders go into the sim's own queue; it applies them at the start of
        // its next tick, exactly like the 2D game and the CLI.
        sim.world
            .resource_mut::<OrderQueue>()
            .pending
            .extend(orders);
        for text in texts {
            if let Err(e) = sim.order(&text) {
                shared.lock().unwrap().errors.push(format!("{text}: {e}"));
            }
        }
        let why_town = shared.lock().unwrap().why_town;
        if paused && why_town != explained {
            // Paused, but the view asks about another town: answer now, on
            // the latest snapshot (copied, with the new findings).
            explained = why_town;
            let mut s = shared.lock().unwrap();
            let mut snap = (*s.latest).clone();
            snap.events.clear();
            snap.samples.clear();
            snap.why = why_town.map(|t| (t, sim_core::explain::explain(&sim.world, t)));
            snap.skills = why_town.map(|t| (t, sim_core::view::town_skills(&sim.world, t)));
            s.latest = Arc::new(snap);
        }
        let now = Instant::now();
        if paused || now < next {
            std::thread::sleep(if paused {
                Duration::from_millis(10)
            } else {
                (next - now).min(Duration::from_millis(10))
            });
            if paused {
                next = Instant::now();
            }
            continue;
        }
        // Run the ticks that are due: a day has `ticks_per_day` of them, so at
        // high speeds several run per pass. A full snapshot at the end of
        // each day, the time of day and weather after every tick.
        let tpd = sim
            .world
            .resource::<sim_core::SimClock>()
            .ticks_per_day
            .max(1);
        let interval = Duration::from_secs_f32(1.0 / (SPEEDS[speed] * tpd as f32));
        let mut ran = 0;
        while Instant::now() >= next && ran < 4 * tpd {
            // Named spans for the profiler (`--features profile`).
            bevy::log::info_span!("sim tick").in_scope(|| sim.step_tick());
            ran += 1;
            next += interval;
            if !sim.world.resource::<sim_core::SimClock>().day_done() {
                continue;
            }
            let _snap = bevy::log::info_span!("sim snapshot").entered();
            let mut snap = ViewSnapshot::capture(&sim.world, &mut seen);
            explained = why_town;
            if let Some(t) = why_town {
                snap.why = Some((t, sim_core::explain::explain(&sim.world, t)));
                snap.skills = Some((t, sim_core::view::town_skills(&sim.world, t)));
            }
            let mut s = shared.lock().unwrap();
            s.events.extend(snap.events.iter().cloned());
            s.samples.extend(snap.samples.iter().cloned());
            s.latest = Arc::new(snap);
        }
        // Keep the pace, but don't try to catch up after a slow stretch.
        next = next.max(now);
        let env = Arc::new(sim.world.resource::<sim_core::env::Environment>().clone());
        let mut s = shared.lock().unwrap();
        s.env = env;
        s.env_at = Instant::now();
        s.tick_secs = interval.as_secs_f32();
        s.ticks_per_day = tpd;
    }
}

/// Hand the sim's time and weather to the view (env.rs): the hour moves on
/// smoothly between ticks, and the weather is that of the towns near the
/// camera's focus, weighted by closeness.
fn publish_env(
    mut commands: Commands,
    live: Res<LiveSim>,
    world: Res<World3d>,
    camera: Query<&crate::camera::WorldCamera>,
    current: Option<Res<SimEnv>>,
) {
    let (env, frac, day_secs, tpd) = live.env();
    let tick_hours = 24.0 / tpd as f32;
    // `env.hour` is the middle of the latest tick; move on through the next.
    let hour = (env.hour + (frac - 0.5) * tick_hours).rem_euclid(24.0);
    let focus = camera.iter().next().map_or(Vec2::ZERO, |c| {
        Vec2::new(c.focus.x * SCALE, -c.focus.z * SCALE)
    });
    let towns: Vec<(Vec2, &sim_core::env::LocalEnv)> = world
        .scenario
        .towns
        .iter()
        .map(|t| Vec2::new(t.position.0, t.position.1))
        .zip(env.towns.iter())
        .collect();
    let (weather, ground) = blend_towns(&towns, focus);
    let next = SimEnv {
        hour,
        season: env.season,
        day: env.day as f32 + (frac - 0.5) * tick_hours / 24.0,
        day_hours: env.sunset - env.sunrise,
        weather,
        ground,
        day_secs,
    };
    if current.as_deref() != Some(&next) {
        commands.insert_resource(next);
    }
}

/// The weather at `at` (sim map coordinates) from the towns' weather,
/// weighted by inverse squared distance. The sim's wind (x, y on its map)
/// becomes 3D (x, -z).
pub fn blend_towns(towns: &[(Vec2, &sim_core::env::LocalEnv)], at: Vec2) -> (Weather, Ground) {
    let mut sum = 0.0;
    let (mut w, mut g) = (
        Weather {
            temperature: 0.0,
            wind: Vec2::ZERO,
            cloud: 0.0,
            precip: 0.0,
            haze: 0.0,
        },
        Ground::default(),
    );
    for (pos, local) in towns {
        let k = 1.0 / (pos.distance_squared(at) + 100.0);
        let lw = &local.weather;
        w.temperature += lw.temperature * k;
        w.wind += Vec2::new(lw.wind.0, -lw.wind.1) * k;
        w.cloud += lw.cloud * k;
        w.precip += lw.precip * k;
        w.haze += lw.haze * k;
        g.snow += local.ground.snow * k;
        g.wet += local.ground.wet * k;
        sum += k;
    }
    if sum == 0.0 {
        return (Weather::default(), Ground::default());
    }
    let s = 1.0 / sum;
    w.temperature *= s;
    w.wind *= s;
    w.cloud *= s;
    w.precip *= s;
    w.haze *= s;
    g.snow *= s;
    g.wet *= s;
    (w, g)
}

/// Buildings and houses currently in the scene.
#[derive(Resource, Default)]
struct Shown {
    /// Building id (from the snapshot) -> its entity.
    buildings: HashMap<u64, Entity>,
    /// Per town: house entities with their map positions, and how far along
    /// its spiral we are.
    houses: Vec<(Vec<(Entity, Vec2)>, u32)>,
    /// Per town: the style it is drawn in (era and wealth tier).
    styles: Vec<Option<Style>>,
}

fn init_kit(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut toons: ResMut<Assets<ToonMaterial>>,
) {
    commands.insert_resource(Kit::new(&mut meshes, &mut toons));
}

fn live_keys(keys: Res<ButtonInput<KeyCode>>, live: Res<LiveSim>) {
    let mut s = live.shared.lock().unwrap();
    if keys.just_pressed(KeyCode::Space) {
        s.paused = !s.paused;
    }
    if keys.just_pressed(KeyCode::Period) {
        s.speed = (s.speed + 1).min(SPEEDS.len() - 1);
    }
    if keys.just_pressed(KeyCode::Comma) {
        s.speed = s.speed.saturating_sub(1);
    }
}

/// Houses a town of `population` shows.
pub fn houses_for(population: u32) -> u32 {
    (population / 6).max(12)
}

/// Bring the scene up to date with the newest snapshot.
#[allow(clippy::too_many_arguments)]
fn sync_scene(
    mut commands: Commands,
    mut live: ResMut<LiveSim>,
    mut shown: ResMut<Shown>,
    kit: Option<ResMut<Kit>>,
    mut meshes: ResMut<Assets<Mesh>>,
    outlines: Option<Res<Outlines>>,
    world: Res<World3d>,
) {
    let (Some(mut kit), Some(outlines)) = (kit, outlines) else {
        return;
    };
    // Take over the newest snapshot and the events since the last frame.
    {
        let mut s = live.shared.lock().unwrap();
        let latest = s.latest.clone();
        let events = std::mem::take(&mut s.events);
        let samples = std::mem::take(&mut s.samples);
        drop(s);
        live.snapshot = latest;
        live.history.extend(samples);
        for e in events {
            live.recent.push_front(e);
        }
        live.recent.truncate(60);
    }
    let snap = live.snapshot.clone();
    if live.shown_tick == Some(snap.tick) {
        return;
    }
    live.shown_tick = Some(snap.tick);
    let t = &world.terrain;
    let ground = |p: (f32, f32)| Vec3::new(p.0 / SCALE, height(t, p), -p.1 / SCALE);

    // Styles (M8): a town whose era or wealth tier changed is rebuilt in its
    // new style (its buildings respawn below, its houses further down).
    if shown.styles.len() < snap.towns.len() {
        shown.styles.resize(snap.towns.len(), None);
        shown.houses.resize_with(snap.towns.len(), Default::default);
    }
    for (ti, town) in snap.towns.iter().enumerate() {
        let now = Style::of(town.era, town.mean_wealth, shown.styles[ti]);
        if shown.styles[ti] == Some(now) {
            continue;
        }
        shown.styles[ti] = Some(now);
        for b in &town.buildings {
            if let Some(e) = shown.buildings.remove(&b.id) {
                commands.entity(e).despawn();
            }
        }
        let (list, next_k) = &mut shown.houses[ti];
        for (e, _) in list.drain(..) {
            commands.entity(e).despawn();
        }
        *next_k = 0;
    }

    // Buildings: add the new ones, remove the ones that are gone.
    let mut alive = Vec::new();
    for (ti, town) in snap.towns.iter().enumerate() {
        for b in &town.buildings {
            alive.push(b.id);
            if shown.buildings.contains_key(&b.id) {
                continue;
            }
            let spec = BuildingSpec {
                kind: b.kind.clone(),
                at: ground(b.pos),
                seed: (b.id as u32).wrapping_mul(2_654_435_761) >> 8 ^ ti as u32,
            };
            let style = shown.styles[ti].unwrap_or_default();
            if let Some(e) = spawn_building(
                &mut commands,
                &mut kit,
                &mut meshes,
                &outlines,
                &spec,
                style,
            ) {
                shown.buildings.insert(b.id, e);
            }
        }
    }
    shown.buildings.retain(|id, e| {
        let keep = alive.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });

    // Houses: a spiral around each town centre, one house per six residents.
    let taken: Vec<Vec2> = snap
        .towns
        .iter()
        .flat_map(|t| t.buildings.iter().map(|b| Vec2::new(b.pos.0, b.pos.1)))
        .collect();
    for (ti, town) in snap.towns.iter().enumerate() {
        let want = houses_for(town.population) as usize;
        let style = shown.styles[ti].unwrap_or_default();
        let (list, next_k) = &mut shown.houses[ti];
        let clear = |p: Vec2| !taken.iter().any(|b| b.distance(p) < 3.2 * SCALE);
        // A new building moves the houses standing in its way further out.
        list.retain(|&(e, p)| {
            let keep = clear(p);
            if !keep {
                commands.entity(e).despawn();
            }
            keep
        });
        let mut tries = 0;
        while list.len() < want && tries < 400 {
            tries += 1;
            let k = *next_k;
            *next_k += 1;
            let a = k as f32 * 2.399_963 + ti as f32;
            let r = 1.8 + (k as f32).sqrt() * 1.05;
            let p = (
                town.pos.0 + r * SCALE * a.cos(),
                town.pos.1 + r * SCALE * a.sin(),
            );
            if t.cell_at(p).is_some_and(|c| t.water[c]) {
                continue;
            }
            if !clear(Vec2::new(p.0, p.1)) {
                continue;
            }
            let rot = Quat::from_rotation_y(a);
            let house = spawn_house(
                &mut commands,
                &mut kit,
                &mut meshes,
                &outlines,
                ground(p),
                rot,
                k,
                style,
            );
            list.push((house, Vec2::new(p.0, p.1)));
        }
        // A shrinking town loses its outermost houses first.
        while list.len() > want {
            if let Some((e, _)) = list.pop() {
                commands.entity(e).despawn();
            }
        }
    }
}

/// Marks the status line.
#[derive(Component)]
struct Status;

fn spawn_status(mut commands: Commands) {
    commands.spawn((
        Status,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            bottom: Val::Px(10.0),
            ..default()
        },
    ));
}

fn update_status(live: Res<LiveSim>, mut text: Query<&mut Text, With<Status>>) {
    let s = &live.snapshot;
    let (speed, paused) = live.speed_and_pause();
    let year = s.tick / s.days_per_year.max(1) as u64 + 1;
    let season = ["winter", "spring", "summer", "autumn"][((s.season() * 4.0 + 0.5) as usize) % 4];
    let pop: u32 = s.towns.iter().map(|t| t.population).sum();
    let towns: Vec<String> = s
        .towns
        .iter()
        .map(|t| format!("{} {}", t.name, t.population))
        .collect();
    let mut line = format!(
        "Year {year}, day {} ({season})   {}   {}   Space pause, , . speed\n\
         {pop} people: {}",
        s.day_of_year + 1,
        if paused { "PAUSED" } else { "running" },
        speed_label(SPEEDS[speed]),
        towns.join(", "),
    );
    for e in live.recent.iter().take(4) {
        line.push_str(&format!("\n  day {}: {}", e.tick, e.text));
    }
    for mut t in &mut text {
        t.0 = line.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panel's text order reaches the simulation thread, is applied at its
    /// next tick, and shows in a later snapshot; a bad order comes back as an
    /// error instead.
    #[test]
    fn text_orders_reach_the_simulation() {
        let paths = sim_core::AssetPaths::discover();
        let (db, params, scenario) = sim_core::load(&paths).unwrap();
        let sim = Simulation::new(db, params, &scenario);
        let live = LiveSim::start(sim, SPEEDS.len() - 1, false);
        let town = live.snapshot.towns[0].name.clone();
        live.order_text(format!("tax {town} wealth 1.75"));
        live.order_text("tax Nowhere wealth 1".into());
        let deadline = Instant::now() + Duration::from_secs(30);
        let applied = loop {
            let snap = live.shared.lock().unwrap().latest.clone();
            if (snap.towns[0].policy.wealth_tax - 1.75).abs() < 1e-4 {
                break true;
            }
            if Instant::now() > deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(applied, "the order never showed in a snapshot");
        // The "why" view: ask about a town, an answer for it arrives.
        live.explain(Some(1));
        let deadline = Instant::now() + Duration::from_secs(30);
        let answered = loop {
            let snap = live.shared.lock().unwrap().latest.clone();
            if snap.why.as_ref().is_some_and(|(t, _)| *t == 1) {
                break true;
            }
            if Instant::now() > deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(answered, "no why answer arrived");
        let errors = live.take_errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("tax Nowhere"));
    }

    /// The weather where the camera looks is mostly that of the nearest town;
    /// the sim's wind (map x, y) becomes 3D (x, -z).
    #[test]
    fn weather_follows_the_nearest_town() {
        let local = |t: f32, wind: (f32, f32), snow: f32| sim_core::env::LocalEnv {
            weather: sim_core::env::Weather {
                temperature: t,
                wind,
                ..Default::default()
            },
            ground: sim_core::env::Ground { snow, wet: 0.0 },
            day_mean_c: t,
        };
        let (a, b) = (local(-5.0, (0.0, 4.0), 1.0), local(15.0, (0.0, 4.0), 0.0));
        let towns = [(Vec2::ZERO, &a), (Vec2::new(1000.0, 0.0), &b)];
        let (w, g) = blend_towns(&towns, Vec2::new(50.0, 0.0));
        assert!(w.temperature < -4.0 && g.snow > 0.95, "{w:?} {g:?}");
        assert!((w.wind - Vec2::new(0.0, -4.0)).length() < 1e-3);
        let (mid, _) = blend_towns(&towns, Vec2::new(500.0, 0.0));
        assert!((mid.temperature - 5.0).abs() < 1e-3);
    }

    /// The sim's clock reaches the view: its hour moves on tick by tick.
    #[test]
    fn the_view_gets_the_sims_hours() {
        let paths = sim_core::AssetPaths::discover();
        let (db, params, scenario) = sim_core::load(&paths).unwrap();
        let sim = Simulation::new(db, params, &scenario);
        let live = LiveSim::start(sim, 2, false); // 12 s a day: a tick a second
        let first = live.env().0.hour;
        let deadline = Instant::now() + Duration::from_secs(30);
        let moved = loop {
            let (env, _, day_secs, tpd) = live.env();
            if env.hour != first {
                assert!(tpd > 1 && day_secs.is_finite());
                break true;
            }
            if Instant::now() > deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(moved, "the hour never changed");
    }

    #[test]
    fn speeds_read_as_people_say_them() {
        assert_eq!(speed_label(SPEEDS[DEFAULT_SPEED]), "5 min/day");
        assert_eq!(speed_label(1.0 / 12.0), "12 s/day");
        assert_eq!(speed_label(30.0), "30 days/s");
        assert!(SPEEDS.windows(2).all(|w| w[0] < w[1]));
    }
}
