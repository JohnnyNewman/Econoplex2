//! Reports: what the 3D client shows that the 2D panels did not
//! (docs/design/client-parity.md), one at a time in a panel in the middle of
//! the screen.
//!
//! `Y` Why? for the town · `T` Town: weather, money, buildings, food
//! security, needs, armory ·
//! `H` History per town · `L` Event log · `I` Person (opens itself when you
//! click someone). The same key again closes the report. Reports follow the
//! economy panel's town (`Tab`) and the selected person, and are computed once
//! per simulated day or when their subject changes, not every frame.

use crate::economy_ui::EconomyView;
use crate::Selection;
use bevy::prelude::*;
use sim_core::explain::Finding;
use sim_core::history::History;
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
        Report::Why => why(world, town),
        Report::Town => town_report(world, town),
        Report::History => history(world),
        Report::Log => log(world),
        Report::Person => person(world, selected),
    };
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

fn why(world: &World, town: usize) -> String {
    let mut s = format!("WHY? {}   (Tab: next town)\n\n", town_name(world, town));
    let found = sim_core::explain::explain(world, town as u16);
    if found.is_empty() {
        s.push_str("Nothing stands out: food, goods, treasury and people look fine.\n");
    }
    for f in found.iter().take(6) {
        finding(&mut s, f, 0);
    }
    s.push_str("\nRule-based: what is short, what stops it, where the money goes.\n");
    footer(&mut s);
    s
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

/// One character per value, low to high.
const SPARKS: [char; 8] = ['_', '.', ':', '-', '=', '+', '*', '#'];

/// The last `n` values as a sparkline, scaled to their own range.
pub fn sparkline(values: &[f32], n: usize) -> String {
    let v = &values[values.len().saturating_sub(n)..];
    let (lo, hi) = v
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    v.iter()
        .map(|&x| {
            let k = if hi > lo {
                ((x - lo) / (hi - lo) * 7.0).round() as usize
            } else {
                3
            };
            SPARKS[k.min(7)]
        })
        .collect()
}

fn history(world: &World) -> String {
    let towns = world.resource::<Towns>();
    let samples = &world.resource::<History>().samples;
    let mut s = String::from("HISTORY   monthly, the last 36 samples per town\n");
    if samples.is_empty() {
        s.push_str("\nThe first samples come after a month.\n");
        footer(&mut s);
        return s;
    }
    type Series = fn(&sim_core::history::Sample) -> f32;
    let series: [(&str, Series); 7] = [
        ("people", |x| x.population as f32),
        ("treasury", |x| x.treasury),
        ("food / resident", |x| x.food_per_capita),
        ("mean wealth", |x| x.mean_wealth),
        ("bread price", |x| x.bread_price),
        ("soldiers", |x| x.soldiers as f32),
        ("buildings", |x| x.buildings as f32),
    ];
    // No blank line between series, so seven fit in the panel.
    s.push('\n');
    for (name, f) in series {
        s.push_str(&format!("{name}\n"));
        for (t, town) in towns.0.iter().enumerate() {
            let v: Vec<f32> = samples
                .iter()
                .filter(|x| x.town as usize == t)
                .map(f)
                .collect();
            let Some(last) = v.last() else { continue };
            s.push_str(&format!(
                "  {:<11} {:<36} {:.1}\n",
                town.name,
                sparkline(&v, 36),
                last
            ));
        }
    }
    footer(&mut s);
    s
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

    #[test]
    fn sparklines_span_their_own_range() {
        assert_eq!(sparkline(&[1.0, 2.0, 3.0], 10), "_=#");
        assert_eq!(sparkline(&[5.0, 5.0], 10), "--");
        assert_eq!(sparkline(&[0.0, 1.0, 2.0, 3.0], 2), "_#");
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
        let town = town_report(&w, 0);
        assert!(town.contains("treasury"), "{town}");
        assert!(town.contains("mean wealth"), "{town}");
        assert!(
            town.contains("Buildings: ") && town.contains(" × "),
            "{town}"
        );
        let hist = history(&w);
        for series in ["treasury", "mean wealth", "bread price"] {
            assert!(hist.contains(series), "{hist}");
        }
        assert!(hist.lines().count() <= MAX_LINES, "{hist}");
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
