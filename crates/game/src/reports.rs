//! Reports: what the 3D client shows that the 2D panels did not
//! (docs/design/client-parity.md), one at a time in a panel in the middle of
//! the screen.
//!
//! `Y` Why? for the town · `T` Town: weather, money, buildings, food
//! security, needs, armory ·
//! `H` History per town, as line charts · `L` Event log · `I` Person (opens itself when you
//! click someone). The same key again closes the report. Reports follow the
//! economy panel's town (`Tab`) and the selected person, and are computed once
//! per simulated day or when their subject changes, not every frame.

use crate::economy_ui::EconomyView;
use crate::Selection;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;
use sim_core::explain::Finding;
use sim_core::history::{History, Sample};
use sim_core::{Db, SimClock, Towns};

/// Which report is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Why,
    Town,
    History,
    Log,
    Person,
}

impl Report {
    /// By name, for `--report`.
    pub fn named(name: &str) -> Option<Report> {
        match name {
            "why" => Some(Report::Why),
            "town" => Some(Report::Town),
            "history" => Some(Report::History),
            "log" => Some(Report::Log),
            "person" => Some(Report::Person),
            _ => None,
        }
    }

    const KEYS: [(KeyCode, Report); 5] = [
        (KeyCode::KeyY, Report::Why),
        (KeyCode::KeyT, Report::Town),
        (KeyCode::KeyH, Report::History),
        (KeyCode::KeyL, Report::Log),
        (KeyCode::KeyI, Report::Person),
    ];
}

#[derive(Resource, Default)]
pub struct Reports {
    pub open: Option<Report>,
    /// What the shown text was made from: report, town, person, day.
    made: Option<(Report, usize, Option<u32>, u64)>,
    /// The selection last seen, to open the person report on a new click.
    last_selected: Option<u32>,
}

#[derive(Component)]
pub struct ReportRoot;

#[derive(Component)]
pub struct ReportText;

/// `--report` and `--select` from the command line.
pub fn apply_args(
    args: Res<crate::autorun::GameArgs>,
    mut reports: ResMut<Reports>,
    mut selection: ResMut<Selection>,
) {
    if let Some(id) = args.select {
        selection.0 = Some(id);
        reports.last_selected = Some(id);
    }
    if args.report.is_some() {
        reports.open = args.report;
    }
}

pub fn spawn_reports(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(660.0),
                top: Val::Px(8.0),
                max_width: Val::Px(600.0),
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.02, 0.97)),
            Visibility::Hidden,
            // Above the other panels, which it may cover while open.
            GlobalZIndex(10),
            ReportRoot,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.92, 0.92)),
                ReportText,
            ));
            p.spawn((Text::default(), ChartLegend));
            p.spawn((
                Node {
                    display: Display::None,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(8.0),
                    row_gap: Val::Px(4.0),
                    width: Val::Px(584.0),
                    margin: UiRect::top(Val::Px(6.0)),
                    ..default()
                },
                ChartGrid,
            ))
            .with_children(|g| {
                for (i, (name, _)) in SERIES.iter().enumerate() {
                    g.spawn(Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Px(288.0),
                        ..default()
                    })
                    .with_children(|c| {
                        c.spawn((
                            Text::new(*name),
                            TextFont {
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.8, 0.8, 0.8)),
                            ChartTitle(i),
                        ));
                        c.spawn((
                            Node {
                                height: Val::Px(84.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.05)),
                            ChartArea(i),
                        ));
                    });
                }
            });
        });
}

/// Keys, and the text of the open report (an exclusive system: the reports
/// read the whole simulation world).
pub fn update_reports(world: &mut World) {
    let pressed: Option<Report> = {
        let keys = world.resource::<ButtonInput<KeyCode>>();
        Report::KEYS
            .iter()
            .find(|(k, _)| keys.just_pressed(*k))
            .map(|&(_, r)| r)
    };
    let selected = world.resource::<Selection>().0;
    let town = world.resource::<EconomyView>().town;
    let day = world.resource::<SimClock>().tick;
    {
        let mut reports = world.resource_mut::<Reports>();
        if let Some(r) = pressed {
            reports.open = if reports.open == Some(r) {
                None
            } else {
                Some(r)
            };
        }
        // A new click on someone opens their report.
        if selected != reports.last_selected {
            reports.last_selected = selected;
            if selected.is_some() {
                reports.open = Some(Report::Person);
            } else if reports.open == Some(Report::Person) {
                reports.open = None;
            }
        }
    }
    let open = world.resource::<Reports>().open;
    let visible = if open.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    let mut roots = world.query_filtered::<&mut Visibility, With<ReportRoot>>();
    for mut v in roots.iter_mut(world) {
        v.set_if_neq(visible);
    }
    let Some(report) = open else {
        world.resource_mut::<Reports>().made = None;
        return;
    };
    let key = (report, town, selected, day);
    if world.resource::<Reports>().made == Some(key) {
        return;
    }
    world.resource_mut::<Reports>().made = Some(key);
    let text = match report {
        Report::Why => {
            let (text, kinds) = why(world, town);
            if let Some(mut rings) = world.get_resource_mut::<crate::seasons::WhyRings>() {
                rings.town = town;
                rings.kinds = kinds;
            }
            text
        }
        Report::Town => town_report(world, town),
        Report::History => {
            let view = history(world, town);
            show_charts(world, Some(&view));
            view.text
        }
        Report::Log => log(world),
        Report::Person => person(world, selected),
    };
    if report != Report::History {
        show_charts(world, None);
    }
    let text = fit(&text, MAX_LINES);
    let mut texts = world.query_filtered::<&mut Text, With<ReportText>>();
    for mut t in texts.iter_mut(world) {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}

/// The most lines a report shows, so it stays on screen.
const MAX_LINES: usize = 40;

/// At most max lines; the last one says how many more there were.
fn fit(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max {
        return text.to_string();
    }
    let mut s = lines[..max - 1].join("\n");
    s.push_str(&format!("\n... {} more lines", lines.len() - (max - 1)));
    s
}

fn town_name(world: &World, town: usize) -> String {
    world
        .resource::<Towns>()
        .0
        .get(town)
        .map_or_else(|| format!("town {town}"), |t| t.name.clone())
}

fn footer(s: &mut String) {
    s.push_str("\nY why | T town | H history | L log | I person | same key closes");
}

// ------------------------------------------------------------------ Why?

/// The Why? report, and the building types its findings name (ringed on the
/// map, seasons.rs).
fn why(world: &World, town: usize) -> (String, Vec<String>) {
    let mut s = format!("WHY? {}   (Tab: next town)\n\n", town_name(world, town));
    let found = sim_core::explain::explain(world, town as u16);
    let mut kinds = Vec::new();
    fn named(f: &Finding, kinds: &mut Vec<String>) {
        if let Some(k) = &f.building {
            if !kinds.contains(k) {
                kinds.push(k.clone());
            }
        }
        for b in &f.because {
            named(b, kinds);
        }
    }
    for f in &found {
        named(f, &mut kinds);
    }
    if found.is_empty() {
        s.push_str("Nothing stands out: food, goods, treasury and people look fine.\n");
    }
    for f in found.iter().take(6) {
        finding(&mut s, f, 0);
    }
    s.push_str("\nRule-based: what is short, what stops it, where the money goes.\n");
    if !kinds.is_empty() {
        s.push_str("The buildings it names are ringed in red on the map.\n");
    }
    footer(&mut s);
    (s, kinds)
}

fn finding(s: &mut String, f: &Finding, depth: usize) {
    let mark = if f.severity >= 0.66 {
        "!!"
    } else if f.severity >= 0.33 {
        "! "
    } else {
        "  "
    };
    s.push_str(&format!("{}{mark} {}\n", "    ".repeat(depth), f.text));
    if depth < 2 {
        for b in f.because.iter().take(3) {
            finding(s, b, depth + 1);
        }
    }
}

// ------------------------------------------------------------------ Town

const SEASONS: [&str; 4] = ["winter", "spring", "summer", "autumn"];

fn season_name(season: f32) -> &'static str {
    SEASONS[((season * 4.0 + 0.5) as usize) % 4]
}

fn town_report(world: &World, town: usize) -> String {
    let mut s = format!("TOWN {}   (Tab: next town)\n\n", town_name(world, town));
    if let Some(env) = world.get_resource::<sim_core::env::Environment>() {
        s.push_str(&format!(
            "{}, day {:.0}; daylight {:.1} h (sunrise {:.1}, sunset {:.1})\n",
            season_name(env.season),
            env.day,
            env.sunset - env.sunrise,
            env.sunrise,
            env.sunset
        ));
        if let Some(local) = env.towns.get(town) {
            let w = &local.weather;
            let sky = if w.precip >= 0.05 {
                if w.snow_fraction() > 0.5 {
                    "snow"
                } else {
                    "rain"
                }
            } else if w.cloud > 0.6 {
                "overcast"
            } else {
                "dry"
            };
            s.push_str(&format!(
                "{:.0} C (day mean {:.0} C), {sky}, cloud {:.0} %, wind {:.0} m/s, snow cover {:.0} %\n",
                w.temperature,
                local.day_mean_c,
                w.cloud * 100.0,
                w.wind_speed(),
                local.ground.snow * 100.0
            ));
        }
    }
    let snap = sim_core::view::ViewSnapshot::capture(world, &mut Default::default());
    let Some(t) = snap.towns.get(town) else {
        footer(&mut s);
        return s;
    };
    s.push_str(&format!(
        "\n{} people, treasury {:.0}, mean wealth {:.1}\n",
        t.population, t.treasury, t.mean_wealth
    ));
    s.push_str(&format!("Buildings: {}\n", building_counts(world, t)));
    if let (Some(env), Some(year)) = (
        world.get_resource::<sim_core::env::Environment>(),
        world.get_resource::<crate::seasons::FieldYear>(),
    ) {
        s.push_str(&format!(
            "Harvest: {:.0} % in; crop standing {:.0} % of a year's growth (peak {:.0} %)\n",
            year.reaped(town, env.season) * 100.0,
            year.crops.get(town).copied().unwrap_or(0.0) * 100.0,
            year.peak.get(town).copied().unwrap_or(0.0) * 100.0
        ));
    }
    s.push_str("\nFood security\n");
    s.push_str(&format!("  food in stock      {:.0} days\n", t.food_days));
    s.push_str(&format!(
        "  granary            {}\n",
        if t.granary_days > 0.0 {
            format!("{:.0} days ({:.0} food)", t.granary_days, t.granary_food)
        } else {
            "none".into()
        }
    ));
    s.push_str(&format!(
        "  seed grain         {}\n",
        if t.seed_reserve {
            "kept back"
        } else {
            "not kept"
        }
    ));
    s.push_str(&format!(
        "  starving this year {} person-days\n",
        t.starving
    ));
    s.push_str(&format!(
        "  cold so far        {} person-days\n",
        t.cold_days
    ));
    if !t.needs.is_empty() {
        s.push_str("\nNeeds (mean satisfaction of residents)\n");
        for (need, v) in sim_data::Need::ALL.iter().zip(&t.needs) {
            s.push_str(&format!(
                "  {:<10} {}\n",
                format!("{need:?}").to_lowercase(),
                bar(*v)
            ));
        }
    }
    if !t.armory.is_empty() {
        let db = world.resource::<Db>();
        let items: Vec<String> = t
            .armory
            .iter()
            .map(|&(p, n)| {
                let name = db
                    .content
                    .products
                    .get(p as usize)
                    .map_or("?", |d| d.name.as_str());
                format!("{n} {name}")
            })
            .collect();
        s.push_str(&format!("\nArmory: {}\n", items.join(", ")));
    }
    footer(&mut s);
    s
}

/// The town's buildings by type, most first: "3 × Workshop, 1 × Mill".
fn building_counts(world: &World, t: &sim_core::view::TownView) -> String {
    let db = world.resource::<Db>();
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for b in &t.buildings {
        let name = db
            .content
            .products
            .iter()
            .find(|p| p.id == b.kind)
            .map_or(b.kind.clone(), |p| p.name.clone());
        match kinds.iter_mut().find(|(k, _)| *k == name) {
            Some((_, n)) => *n += 1,
            None => kinds.push((name, 1)),
        }
    }
    if kinds.is_empty() {
        return "none".into();
    }
    kinds.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let parts: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} × {k}")).collect();
    parts.join(", ")
}

fn bar(v: f32) -> String {
    let n = (v.clamp(0.0, 1.0) * 20.0).round() as usize;
    format!(
        "[{}{}] {:>3.0} %",
        "#".repeat(n),
        ".".repeat(20 - n),
        v * 100.0
    )
}

// ------------------------------------------------------------------ History

/// How many monthly samples a chart shows per town.
const CHART_SAMPLES: usize = 36;

type Series = fn(&Sample) -> f32;

/// The seven history series, as in the 3D client's chart.
const SERIES: [(&str, Series); 7] = [
    ("people", |x| x.population as f32),
    ("treasury", |x| x.treasury),
    ("food / resident", |x| x.food_per_capita),
    ("mean wealth", |x| x.mean_wealth),
    ("bread price", |x| x.bread_price),
    ("soldiers", |x| x.soldiers as f32),
    ("buildings", |x| x.buildings as f32),
];

/// The render layer the history charts are drawn on, which only the chart
/// camera sees.
const CHART_LAYER: usize = 1;

/// A second camera for the history charts. It renders after the world camera
/// and its panels, so the lines sit on top of the report panel; its world
/// units are screen pixels.
#[derive(Component)]
pub struct ChartCamera;

/// The gizmos of the history charts, on [`CHART_LAYER`].
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct ChartGizmos;

/// The cells of the history charts, shown while `H` is open.
#[derive(Component)]
pub struct ChartGrid;

/// The town names in their chart colours.
#[derive(Component)]
pub struct ChartLegend;

/// The title of chart `i`: series name, range, the selected town's value.
#[derive(Component)]
pub struct ChartTitle(usize);

/// The plot area of chart `i`, which [`draw_charts`] fills.
#[derive(Component)]
pub struct ChartArea(usize);

pub fn spawn_chart_camera(mut commands: Commands, mut store: ResMut<GizmoConfigStore>) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        RenderLayers::layer(CHART_LAYER),
        ChartCamera,
    ));
    let (config, _) = store.config_mut::<ChartGizmos>();
    config.render_layers = RenderLayers::layer(CHART_LAYER);
    config.line.width = 1.5;
}

/// A town's colour in the charts: evenly spaced hues.
pub fn town_color(town: usize, towns: usize) -> Color {
    Color::hsl(30.0 + 360.0 * town as f32 / towns.max(1) as f32, 0.75, 0.62)
}

/// Each town's last `n` values of one series, and the range they share.
pub fn chart_series(
    samples: &[Sample],
    towns: usize,
    f: Series,
    n: usize,
) -> (Vec<Vec<f32>>, f32, f32) {
    let mut per_town: Vec<Vec<f32>> = (0..towns)
        .map(|t| {
            samples
                .iter()
                .filter(|x| x.town as usize == t)
                .map(f)
                .collect()
        })
        .collect();
    for v in &mut per_town {
        v.drain(..v.len().saturating_sub(n));
    }
    let (lo, hi) = per_town
        .iter()
        .flatten()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    if lo > hi {
        return (per_town, 0.0, 0.0);
    }
    (per_town, lo, hi)
}

/// The points of a line in `rect`, spread over `n` monthly slots so every
/// town's last sample sits at the right edge. A flat range draws mid-height.
pub fn plot(values: &[f32], lo: f32, hi: f32, n: usize, rect: Rect) -> Vec<Vec2> {
    let start = n.saturating_sub(values.len());
    let step = rect.width() / n.saturating_sub(1).max(1) as f32;
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let y = if hi > lo { (v - lo) / (hi - lo) } else { 0.5 };
            Vec2::new(
                rect.min.x + (start + i) as f32 * step,
                rect.min.y + y * rect.height(),
            )
        })
        .collect()
}

/// A value short enough for a chart title.
fn num(v: f32) -> String {
    if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// What the history report shows besides the charts.
struct HistoryView {
    text: String,
    titles: Vec<String>,
    /// Each town's name, colour and whether it is the selected one.
    legend: Vec<(String, Color, bool)>,
}

fn history(world: &World, town: usize) -> HistoryView {
    let towns = world.resource::<Towns>();
    let samples = &world.resource::<History>().samples;
    let mut text = format!(
        "HISTORY   monthly, the last {CHART_SAMPLES} samples per town\n\
         Bold: {} (Tab for the next town)\n",
        town_name(world, town)
    );
    if samples.is_empty() {
        text.push_str("\nThe first samples come after a month.\n");
    }
    footer(&mut text);
    let titles = SERIES
        .iter()
        .map(|&(name, f)| {
            let (series, lo, hi) = chart_series(samples, towns.0.len(), f, CHART_SAMPLES);
            match series.get(town).and_then(|v| v.last()) {
                Some(&last) => format!("{name}   {} to {}   now {}", num(lo), num(hi), num(last)),
                None => name.to_string(),
            }
        })
        .collect();
    let legend = towns
        .0
        .iter()
        .enumerate()
        .map(|(t, x)| (x.name.clone(), town_color(t, towns.0.len()), t == town))
        .collect();
    HistoryView {
        text,
        titles,
        legend,
    }
}

/// Shows the chart cells for the history report and hides them for the
/// others; fills their titles and the legend.
fn show_charts(world: &mut World, view: Option<&HistoryView>) {
    let display = if view.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    let mut grids = world.query_filtered::<&mut Node, With<ChartGrid>>();
    for mut node in grids.iter_mut(world) {
        if node.display != display {
            node.display = display;
        }
    }
    let Some(view) = view else { return };
    let mut titles = world.query::<(&ChartTitle, &mut Text)>();
    for (title, mut text) in titles.iter_mut(world) {
        let want = &view.titles[title.0];
        if &text.0 != want {
            text.0 = want.clone();
        }
    }
    let legends: Vec<Entity> = world
        .query_filtered::<Entity, With<ChartLegend>>()
        .iter(world)
        .collect();
    for legend in legends {
        let mut entity = world.entity_mut(legend);
        entity.despawn_related::<Children>();
        entity.with_children(|p| {
            for (name, color, bold) in &view.legend {
                let name = if *bold {
                    format!("[{name}]   ")
                } else {
                    format!("{name}   ")
                };
                p.spawn((
                    TextSpan::new(name),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(*color),
                ));
            }
        });
    }
}

/// The history charts: one line per town in each cell, the economy panel's
/// town in bold and drawn last, on top.
pub fn draw_charts(
    mut gizmos: Gizmos<ChartGizmos>,
    reports: Res<Reports>,
    history: Res<History>,
    towns: Res<Towns>,
    view: Res<EconomyView>,
    camera: Single<(&Camera, &GlobalTransform), With<ChartCamera>>,
    areas: Query<(
        &ChartArea,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
    )>,
) {
    if reports.open != Some(Report::History) {
        return;
    }
    let (camera, at) = *camera;
    let n = towns.0.len();
    for (area, node, tf, visible) in &areas {
        if !visible.get() || node.is_empty() {
            continue;
        }
        // UI positions are physical pixels from the top left; the camera
        // wants logical ones.
        let scale = node.inverse_scale_factor();
        let center = tf.translation * scale;
        let half = node.size() * scale / 2.0 - Vec2::splat(6.0);
        let (Ok(a), Ok(b)) = (
            camera.viewport_to_world_2d(at, center - half),
            camera.viewport_to_world_2d(at, center + half),
        ) else {
            continue;
        };
        let rect = Rect::from_corners(a, b);
        let (series, lo, hi) = chart_series(&history.samples, n, SERIES[area.0].1, CHART_SAMPLES);
        let bold = view.town;
        for (t, values) in series.iter().enumerate() {
            if t == bold || values.len() < 2 {
                continue;
            }
            let points = plot(values, lo, hi, CHART_SAMPLES, rect);
            gizmos.linestrip_2d(points, town_color(t, n).with_alpha(0.6));
        }
        if let Some(values) = series.get(bold).filter(|v| v.len() >= 2) {
            let points = plot(values, lo, hi, CHART_SAMPLES, rect);
            for dy in [-1.0, 0.0, 1.0] {
                let off = Vec2::new(0.0, dy);
                gizmos.linestrip_2d(points.iter().map(|&p| p + off), town_color(bold, n));
            }
        }
    }
}

// ------------------------------------------------------------------ Log

fn log(world: &World) -> String {
    let per_year = world.resource::<SimClock>().months_per_year;
    let towns = world.resource::<Towns>();
    let events = &world.resource::<History>().events;
    let mut s = format!("EVENT LOG   {} events, newest first\n\n", events.len());
    for e in events.iter().rev().take(30) {
        let town = towns.0.get(e.town as usize).map_or("", |t| t.name.as_str());
        s.push_str(&format!(
            "  {:<7}  {:<10} {}\n",
            sim_core::calendar::date(e.tick, per_year),
            town,
            e.text
        ));
    }
    if events.is_empty() {
        s.push_str("Nothing has happened yet.\n");
    }
    footer(&mut s);
    s
}

// ------------------------------------------------------------------ Person

fn person(world: &World, selected: Option<u32>) -> String {
    let Some(id) = selected else {
        let mut s = String::from(
            "PERSON\n\nClick someone on the map to see their needs, goods and household.\n",
        );
        footer(&mut s);
        return s;
    };
    let Some(p) = sim_core::view::person(world, id) else {
        let mut s = format!("PERSON #{id}\n\nNo longer alive.\n");
        footer(&mut s);
        return s;
    };
    let mut s = format!(
        "PERSON #{}  {}, {:.0} years, {}\n",
        p.id,
        if p.female { "woman" } else { "man" },
        p.age_years,
        town_name(world, p.town as usize)
    );
    s.push_str(&match &p.job {
        Some(job) if p.activity == "working" => format!("works: {job}\n"),
        Some(job) => format!("{} (last worked: {job})\n", p.activity),
        None => format!("{}\n", p.activity),
    });
    if !p.needs.is_empty() {
        s.push_str("\nNeeds\n");
        for (need, v) in &p.needs {
            s.push_str(&format!("  {need:<10} {}\n", bar(*v)));
        }
    }
    let list = |s: &mut String, title: &str, items: &[(String, f32)]| {
        if items.is_empty() {
            return;
        }
        let parts: Vec<String> = items.iter().map(|(n, q)| format!("{q:.1} {n}")).collect();
        s.push_str(&format!("\n{title}: {}\n", parts.join(", ")));
    };
    list(&mut s, "Larder", &p.larder);
    list(&mut s, "Household goods", &p.owned);
    if !p.gear.is_empty() {
        let parts: Vec<String> = p
            .gear
            .iter()
            .map(|(slot, item)| format!("{slot}: {item}"))
            .collect();
        s.push_str(&format!("\nEquipment: {}\n", parts.join(", ")));
    }
    s.push_str("\nHousehold\n");
    let mut any = false;
    if let Some(partner) = p.partner {
        s.push_str(&format!("  partner #{partner}\n"));
        any = true;
    }
    if let Some((a, b)) = p.parents {
        s.push_str(&format!("  parents #{a} and #{b}\n"));
        any = true;
    }
    if !p.children.is_empty() {
        let c: Vec<String> = p.children.iter().map(|c| format!("#{c}")).collect();
        s.push_str(&format!("  children {}\n", c.join(", ")));
        any = true;
    }
    // Others at the same hearth (sim_core::ladder::keepers).
    let store = world.resource::<sim_core::AgentStore>();
    let keepers = sim_core::ladder::keepers(store);
    let mine = keepers.get(id as usize).copied();
    let mates = (0..store.len())
        .filter(|&k| store.alive[k] && k as u32 != id && Some(keepers[k]) == mine)
        .count();
    s.push_str(&format!(
        "  {} at this hearth\n",
        if mates == 0 {
            "lives alone".to_string()
        } else {
            format!("{} people", mates + 1)
        }
    ));
    if !any && mates == 0 {
        s.push_str("  no close family\n");
    }
    footer(&mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(town: u16, population: u32) -> Sample {
        Sample {
            tick: 0,
            town,
            population,
            food_per_capita: 0.0,
            treasury: 0.0,
            mean_wealth: 0.0,
            soldiers: 0,
            buildings: 0,
            starving: 0,
            bread_price: 0.0,
            ruler: None,
        }
    }

    #[test]
    fn charts_share_a_range_and_end_at_the_right_edge() {
        let samples: Vec<Sample> = (0..5)
            .flat_map(|k| [sample(0, 10 + k), sample(1, 40)])
            .collect();
        let (series, lo, hi) = chart_series(&samples, 2, SERIES[0].1, 3);
        assert_eq!(series, vec![vec![12.0, 13.0, 14.0], vec![40.0; 3]]);
        assert_eq!((lo, hi), (12.0, 40.0));
        let rect = Rect::new(0.0, 0.0, 100.0, 50.0);
        let points = plot(&[12.0, 40.0], lo, hi, 3, rect);
        assert_eq!(points, vec![Vec2::new(50.0, 0.0), Vec2::new(100.0, 50.0)]);
        assert_eq!(
            plot(&[5.0], 5.0, 5.0, 3, rect),
            vec![Vec2::new(100.0, 25.0)]
        );
        let (empty, lo, hi) = chart_series(&[], 2, SERIES[0].1, 3);
        assert_eq!((empty.len(), lo, hi), (2, 0.0, 0.0));
    }

    fn world() -> World {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let (db, params, mut scenario) =
            sim_core::load(&sim_core::AssetPaths::from_root(&root)).unwrap();
        for t in &mut scenario.towns {
            t.population = 40;
        }
        let mut world = World::new();
        sim_core::world::setup(&mut world, db, params, &scenario);
        sim_core::add_schedules(&mut world);
        world
    }

    #[test]
    fn the_town_and_history_reports_show_money_and_buildings() {
        let mut w = world();
        for _ in 0..40 {
            sim_core::run_day(&mut w);
        }
        w.insert_resource(crate::seasons::FieldYear::default());
        let town = town_report(&w, 0);
        assert!(town.contains("Harvest: "), "{town}");
        assert!(town.contains("treasury"), "{town}");
        assert!(town.contains("mean wealth"), "{town}");
        assert!(
            town.contains("Buildings: ") && town.contains(" × "),
            "{town}"
        );
        let hist = history(&w, 0);
        assert_eq!(hist.titles.len(), 7);
        for (title, series) in hist.titles.iter().zip(["people", "treasury"]) {
            assert!(
                title.starts_with(series) && title.contains(" now "),
                "{title}"
            );
        }
        assert!(hist.legend[0].2 && !hist.legend[1].2, "{:?}", hist.legend);
        assert!(hist.text.contains("Bold: "), "{}", hist.text);
    }

    #[test]
    fn roads_join_the_towns() {
        let w = world();
        let terrain = w.resource::<sim_core::terrain::Terrain>();
        let towns: Vec<(f32, f32)> = w.resource::<Towns>().0.iter().map(|t| t.pos).collect();
        let cells = crate::terrain_view::road_cells(terrain, &towns);
        assert!(cells.len() > towns.len(), "{} road cells", cells.len());
    }

    #[test]
    fn seasons_follow_the_year() {
        assert_eq!(season_name(0.0), "winter");
        assert_eq!(season_name(0.5), "summer");
        assert_eq!(season_name(0.99), "winter");
    }
}
