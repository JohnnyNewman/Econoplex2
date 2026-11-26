//! Text overlays: world summary, agent inspector, domain legend, controls.

use crate::view::{activity_label, domain_color};
use crate::{Selection, SimControl, SPEEDS};
use bevy::prelude::*;
use sim_core::components::Task;
use sim_core::metrics::Metrics;
use sim_core::store::Role;
use sim_core::world::Params;
use sim_core::{AgentStore, Db, SimClock, Towns};
use sim_data::dims::{MIND_NAMES, PHYS_NAMES, STATE_NAMES};

#[derive(Component)]
pub struct HudText;

#[derive(Component)]
pub struct InspectorText;

/// Left-hand column: the world summary, then the economy panel.
#[derive(Component)]
pub struct LeftColumn;

/// Right-hand column: the inspector, then the policy panel, then the controls
/// help. Stacked, so they never overlap; the inspector gives up height first.
#[derive(Component)]
pub struct RightColumn;

/// The controls help, one line until `F1` opens it.
#[derive(Component)]
pub struct HelpText;

/// The full controls help (`F1`).
pub const HELP: &str = "Space pause | 1-5 speed | WASD pan | wheel zoom\nclick select | Esc deselect | F follow | P product space\nK colour by best skill / today's work\nE economy | Tab next town | F5 save | F9 load\nY why | T town | H history | L log | I person\nC culture: red tradition, green faith, blue liberty\nF1 hide this help";
/// The one line it shows while folded.
pub const HELP_FOLDED: &str = "F1 controls";

#[derive(Component)]
pub struct LegendRoot;

/// The legend's title, which says which colouring `K` has chosen.
#[derive(Component)]
pub struct LegendTitle;

/// The legend's activity rows, shown only while people are coloured by what
/// they do today.
#[derive(Component)]
pub struct LegendActivity;

const BY_WORK: &str = "Work today (K: best skill)";
const BY_SKILL: &str = "Best skill domain (K: work today)";

const PANEL: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);

fn panel(left: Option<f32>, right: Option<f32>, top: Option<f32>, bottom: Option<f32>) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: left.map_or(Val::Auto, Val::Px),
        right: right.map_or(Val::Auto, Val::Px),
        top: top.map_or(Val::Auto, Val::Px),
        bottom: bottom.map_or(Val::Auto, Val::Px),
        padding: UiRect::all(Val::Px(8.0)),
        flex_direction: FlexDirection::Column,
        ..default()
    }
}

pub fn spawn_hud(mut commands: Commands, db: Res<Db>) {
    let font = |size: f32| TextFont {
        font_size: FontSize::Px(size),
        ..default()
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(8.0),
                top: Val::Px(8.0),
                bottom: Val::Px(8.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                align_items: AlignItems::FlexStart,
                // What doesn't fit is cut off at the bottom (fold economy
                // sections, or hide the panel with E).
                overflow: Overflow::clip_y(),
                ..default()
            },
            LeftColumn,
        ))
        .with_children(|c| {
            c.spawn((
                Node {
                    padding: UiRect::all(Val::Px(8.0)),
                    flex_direction: FlexDirection::Column,
                    // Long lines (the era list) wrap instead of running
                    // under the panels in the middle.
                    max_width: Val::Px(640.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(PANEL),
            ))
            .with_children(|p| {
                p.spawn((Text::new(""), font(13.0), HudText));
            });
        });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                top: Val::Px(8.0),
                bottom: Val::Px(8.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                align_items: AlignItems::FlexEnd,
                ..default()
            },
            RightColumn,
        ))
        .with_children(|c| {
            // The inspector shrinks (and cuts off its last lines) before the
            // policy panel below it does.
            c.spawn((
                Node {
                    padding: UiRect::all(Val::Px(8.0)),
                    flex_direction: FlexDirection::Column,
                    flex_shrink: 1.0,
                    min_height: Val::Px(0.0),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                BackgroundColor(PANEL),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new("Click an agent to inspect it."),
                    font(13.0),
                    InspectorText,
                ));
            });
        });
    commands
        .spawn((
            panel(Some(8.0), None, None, Some(8.0)),
            BackgroundColor(PANEL),
            LegendRoot,
        ))
        .with_children(|p| {
            p.spawn((Text::new(BY_WORK), font(13.0), LegendTitle));
            let n = db.content.domains.len();
            for (d, name) in db.content.domains.iter().enumerate() {
                p.spawn((
                    Text::new(format!("## {name}")),
                    font(13.0),
                    TextColor(domain_color(d, n, 0.8)),
                ));
            }
            p.spawn((
                Text::new("## child"),
                font(13.0),
                TextColor(crate::view::CHILD),
            ));
            for (name, c) in [
                ("idle", crate::view::IDLE),
                ("resting", crate::view::RESTING),
                ("soldier", crate::view::SOLDIER),
                ("captive", crate::view::CAPTIVE),
            ] {
                p.spawn((
                    Text::new(format!("## {name}")),
                    font(13.0),
                    TextColor(c),
                    LegendActivity,
                ));
            }
        });
}

/// The controls help at the foot of the right-hand column (after the policy
/// panel, so it is spawned last).
pub fn spawn_help(mut commands: Commands, column: Single<Entity, With<RightColumn>>) {
    let help = commands
        .spawn((
            Node {
                padding: UiRect::all(Val::Px(6.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(HELP_FOLDED),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
                HelpText,
            ));
        })
        .id();
    commands.entity(*column).add_child(help);
}

/// `F1` opens and folds the controls help.
pub fn toggle_help(keys: Res<ButtonInput<KeyCode>>, mut help: Single<&mut Text, With<HelpText>>) {
    if keys.just_pressed(KeyCode::F1) {
        help.0 = if help.0 == HELP { HELP_FOLDED } else { HELP }.to_string();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_hud(
    mut text: Single<&mut Text, With<HudText>>,
    clock: Res<SimClock>,
    control: Res<SimControl>,
    store: Res<AgentStore>,
    towns: Res<Towns>,
    db: Res<Db>,
    metrics: Res<Metrics>,
    player: Res<sim_core::policy::PlayerTown>,
    toast: Res<crate::saving::Toast>,
    time: Res<Time>,
    history: Res<sim_core::history::History>,
    goal: (
        Res<sim_core::world::ScenarioSpec>,
        Res<sim_core::goal::MatchState>,
    ),
) {
    let (scenario, state) = goal;
    let goal_amount = scenario.0.goal.as_ref().map(|g| g.amount);
    let mut s = format!(
        "ECONOPLEX   year {}  {} {:02}:00   {}{}\npopulation {}\n",
        clock.year(),
        sim_core::calendar::month_name(clock.day_of_year(), clock.months_per_year),
        clock.hour().floor() as u32,
        if control.paused { "PAUSED  " } else { "" },
        sim_core::calendar::speed_label(SPEEDS[control.speed] as f32),
        store.alive_count()
    );
    let latest = metrics.latest();
    // The goal column only shows when the scenario sets a goal.
    s.push_str(if goal_amount.is_some() {
        "\ntown          pop   food  ECI  div  era  swords 1/2/3  goal  army\n"
    } else {
        "\ntown          pop   food  ECI  div  era  swords 1/2/3  army\n"
    });
    for (t, town) in towns.0.iter().enumerate() {
        let food = town.food_stock(&db) / town.residents.len().max(1) as f32;
        let (eci, div, swords, era) = latest
            .and_then(|y| y.towns.get(t))
            .map(|ts| {
                (
                    format!("{:>5.2}", ts.eci),
                    ts.diversity.to_string(),
                    ts.swords,
                    ts.era.to_string(),
                )
            })
            .unwrap_or(("  -  ".into(), "-".into(), [0; 3], "-".into()));
        let army = match &town.raid {
            Some(r) => format!(
                "{} -> {}",
                town.squad.len(),
                towns.0[r.target as usize].name
            ),
            None => town.squad.len().to_string(),
        };
        let army = match town.ruler {
            Some(r) => format!("{army} (ruled by {})", towns.0[r as usize].name),
            None => army,
        };
        let name = if player.0 == Some(t as u16) {
            format!("{}*", town.name)
        } else {
            town.name.clone()
        };
        let progress = match goal_amount {
            Some(a) => format!(
                "{:>5}  ",
                format!("{:.0}/{a:.0}", state.progress(&towns, t))
            ),
            None => String::new(),
        };
        s.push_str(&format!(
            "{:<11} {:>5} {:>6.1} {} {:>4} {:>4}  {:>3}/{}/{}  {}{}\n",
            name,
            town.residents.len(),
            food,
            eci,
            div,
            era,
            swords[0],
            swords[1],
            swords[2],
            progress,
            army
        ));
    }
    if player.0.is_some() {
        s.push_str("* your town\n");
    }
    // Each town's culture vector (follows its people slowly).
    s.push_str("\nculture      tradition faith liberty\n");
    for town in &towns.0 {
        if let [t, f, l, ..] = town.culture[..] {
            s.push_str(&format!(
                "{:<11}  {:>8.2} {:>5.2} {:>7.2}\n",
                town.name, t, f, l
            ));
        }
    }
    if !db.content.eras.is_empty() {
        let names: Vec<String> = db
            .content
            .eras
            .iter()
            .enumerate()
            .map(|(k, n)| format!("{k} {n}"))
            .collect();
        s.push_str(&format!("eras: {}\n", names.join(" | ")));
    }
    if let Some(g) = crate::economy_ui::goal_text(&scenario, &state, &towns, &db) {
        s.push_str(&format!("\n{g}\n"));
    }
    if latest.is_none() {
        s.push_str("(complexity and swords appear after the first year)\n");
    }
    // The latest events (see `sim_core::history`), newest first.
    let recent: Vec<String> = history
        .recent(5)
        .map(|e| {
            format!(
                "  {:<7}  {}",
                sim_core::calendar::date(e.tick, clock.months_per_year),
                e.text
            )
        })
        .collect();
    if !recent.is_empty() {
        s.push_str("\nlatest events\n");
        s.push_str(&recent.join("\n"));
        s.push('\n');
    }
    if let Some(msg) = toast.current(time.elapsed_secs_f64()) {
        s.push_str(&format!(
            "
{msg}
"
        ));
    }
    text.0 = s;
}

fn bar(v: f32) -> String {
    let n = (v.clamp(0.0, 1.0) * 10.0).round() as usize;
    format!("[{}{}] {:.2}", "#".repeat(n), ".".repeat(10 - n), v)
}

#[allow(clippy::too_many_arguments)]
pub fn update_inspector(
    mut text: Single<&mut Text, With<InspectorText>>,
    selection: Res<Selection>,
    store: Res<AgentStore>,
    db: Res<Db>,
    towns: Res<Towns>,
    params: Res<Params>,
    clock: Res<SimClock>,
    tasks: Query<&Task>,
) {
    let Some(idx) = selection.0 else {
        text.0 = "Click an agent to inspect it.".into();
        return;
    };
    let i = idx as usize;
    if !store.alive[i] {
        text.0 = format!("Agent #{idx} has died.");
        return;
    }
    let age = store.age_years(i, clock.months_per_year);
    let mut s = format!(
        "Agent #{idx}  {}  age {:.0}\n{}  wealth {:.0}\n{}\n",
        if store.female[i] { "female" } else { "male" },
        age,
        towns.0[store.town[i] as usize].name,
        store.wealth[i].coins(),
        activity_label(&store, &db, &tasks, i),
    );
    if let Some(g) = store.guild[i] {
        let role = match g.role {
            Role::Apprentice => "apprentice",
            Role::Member => "member",
            Role::Master => "master",
        };
        s.push_str(&format!(
            "{} guild, {role}\n",
            db.content.domains[g.domain as usize]
        ));
    }
    if let Some(tool) = store.equipped[i] {
        s.push_str(&format!("tool equipped ({tool})\n"));
    }
    if age < params.life.adult_age {
        s.push_str("child: learns by apprenticing with a parent\n");
    }

    s.push_str("\nTop skills\n");
    let prof = db.skill_profile(&store.cap[i]);
    let mut order: Vec<usize> = (0..prof.len()).collect();
    order.sort_by(|&a, &b| prof[b].total_cmp(&prof[a]));
    for &k in order.iter().take(4) {
        s.push_str(&format!(
            "  {:<16} {}\n",
            db.content.skills[k].name,
            bar(prof[k] / 1.5)
        ));
    }
    s.push_str("\nMind\n");
    for (k, name) in MIND_NAMES.iter().enumerate() {
        s.push_str(&format!("  {:<12} {}\n", name, bar(store.mind[i][k])));
    }
    // How well the agent's outlook fits each town's culture (-1..1).
    s.push_str("\nCulture fit\n");
    let outlook = &store.mind[i][sim_data::dims::mind::IDEOLOGY_START..];
    for (t, town) in towns.0.iter().enumerate() {
        if town.culture.is_empty() {
            continue;
        }
        let fit = sim_core::decision::culture::fit(outlook, &town.culture);
        let here = if t == store.town[i] as usize {
            "  (home)"
        } else {
            ""
        };
        s.push_str(&format!("  {:<12} {:+.2}{here}\n", town.name, fit));
    }
    s.push_str("\nPhysique (actual / potential)\n");
    for (k, name) in PHYS_NAMES.iter().enumerate() {
        s.push_str(&format!(
            "  {:<12} {} / {:.2}\n",
            name,
            bar(store.phys[i][k]),
            store.potential[i][k]
        ));
    }
    s.push_str("\nState\n");
    for (k, name) in STATE_NAMES.iter().enumerate() {
        s.push_str(&format!("  {:<12} {}\n", name, bar(store.state[i][k])));
    }
    s.push_str(&format!("\naptitude {:.2}", store.aptitude[i]));
    if let Some(p) = store.partner[i] {
        s.push_str(&format!("   partner #{p}"));
    }
    if let Some((a, b)) = store.parents[i] {
        s.push_str(&format!("\nparents #{a} and #{b}"));
    }
    text.0 = s;
}

/// The legend is static; this keeps it hidden while the product space overlay explains colors itself.
pub fn update_legend(
    mut legend: Single<&mut Visibility, (With<LegendRoot>, Without<LegendActivity>)>,
    mut title: Single<&mut Text, With<LegendTitle>>,
    mut rows: Query<&mut Visibility, With<LegendActivity>>,
    overlay: Res<crate::view::Overlay>,
    economy: Res<crate::economy_ui::EconomyView>,
) {
    legend.set_if_neq(
        if overlay.product_space || overlay.culture || economy.visible {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        },
    );
    let want = if overlay.best_skill {
        BY_SKILL
    } else {
        BY_WORK
    };
    if title.0 != want {
        title.0 = want.to_string();
    }
    for mut v in &mut rows {
        v.set_if_neq(if overlay.best_skill {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
}
