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

#[derive(Component)]
pub struct LegendRoot;

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
            panel(Some(8.0), None, Some(8.0), None),
            BackgroundColor(PANEL),
        ))
        .with_children(|p| {
            p.spawn((Text::new(""), font(13.0), HudText));
        });
    commands
        .spawn((
            panel(None, Some(8.0), Some(8.0), None),
            BackgroundColor(PANEL),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("Click an agent to inspect it."),
                font(13.0),
                InspectorText,
            ));
        });
    commands
        .spawn((
            panel(Some(8.0), None, None, Some(8.0)),
            BackgroundColor(PANEL),
            LegendRoot,
        ))
        .with_children(|p| {
            p.spawn((Text::new("Best skill domain"), font(13.0)));
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
                TextColor(Color::srgb(0.75, 0.75, 0.75)),
            ));
        });
    commands.spawn((panel(None, Some(8.0), None, Some(8.0)), BackgroundColor(PANEL))).with_children(|p| {
        p.spawn((
            Text::new("Space pause | 1-5 speed | WASD pan | wheel zoom\nclick select | Esc deselect | P product space"),
            font(12.0),
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
        ));
    });
}

pub fn update_hud(
    mut text: Single<&mut Text, With<HudText>>,
    clock: Res<SimClock>,
    control: Res<SimControl>,
    store: Res<AgentStore>,
    towns: Res<Towns>,
    db: Res<Db>,
    metrics: Res<Metrics>,
) {
    let mut s = format!(
        "ECONOPLEX   year {}  day {:>3}   {}{} days/s\npopulation {}\n",
        clock.year(),
        clock.day_of_year(),
        if control.paused { "PAUSED  " } else { "" },
        SPEEDS[control.speed],
        store.alive_count()
    );
    let latest = metrics.latest();
    s.push_str("\ntown          pop   food  ECI  div  swords 1/2/3  army\n");
    for (t, town) in towns.0.iter().enumerate() {
        let food = town.food_stock(&db) / town.residents.len().max(1) as f32;
        let (eci, div, swords) = latest
            .and_then(|y| y.towns.get(t))
            .map(|ts| {
                (
                    format!("{:>5.2}", ts.eci),
                    ts.diversity.to_string(),
                    ts.swords,
                )
            })
            .unwrap_or(("  -  ".into(), "-".into(), [0; 3]));
        let army = match &town.raid {
            Some(r) => format!(
                "{} -> {}",
                town.squad.len(),
                towns.0[r.target as usize].name
            ),
            None => town.squad.len().to_string(),
        };
        s.push_str(&format!(
            "{:<11} {:>5} {:>6.1} {} {:>4}  {}/{}/{}  {}\n",
            town.name,
            town.residents.len(),
            food,
            eci,
            div,
            swords[0],
            swords[1],
            swords[2],
            army
        ));
    }
    if latest.is_none() {
        s.push_str("(complexity and swords appear after the first year)\n");
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
    let age = store.age_years(i, clock.days_per_year);
    let mut s = format!(
        "Agent #{idx}  {}  age {:.0}\n{}  wealth {:.0}\n{}\n",
        if store.female[i] { "female" } else { "male" },
        age,
        towns.0[store.town[i] as usize].name,
        store.wealth[i],
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
    mut legend: Single<&mut Visibility, With<LegendRoot>>,
    overlay: Res<crate::view::Overlay>,
) {
    **legend = if overlay.product_space {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
}
