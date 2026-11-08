//! The readable economy: a panel per town, money and trade flows on the map, and
//! the match goal when the scenario sets one.
//!
//! The panel (E to show or hide, Tab for the next town) shows where a town's money
//! comes from and goes to, as daily averages over about a month: coin struck at
//! its mint, what the treasury issues to people, what it takes back as taxes, and
//! what people spend at its market. Below that: who is working, the town's goods
//! with stock and price, and the goods in short supply. Click a section header to
//! fold it away. Arrows between towns show the value of goods each caravan route
//! carries.

use bevy::prelude::*;
use sim_core::goal::{MatchState, WinReason};
use sim_core::ledger::{Flow, Group, Ledger};
use sim_core::store::Activity;
use sim_core::world::{Params, ScenarioSpec, Town};
use sim_core::{AgentStore, Db, Towns};
use sim_data::Category;

/// Foldable sections of the panel.
pub const SECTIONS: [&str; 5] = [
    "Money per day, last month",
    "Levers",
    "Work today",
    "Goods",
    "Food and shortages",
];
const MONEY: usize = 0;
const LEVERS: usize = 1;
const WORK: usize = 2;
const GOODS: usize = 3;
const FOOD: usize = 4;

#[derive(Resource)]
pub struct EconomyView {
    pub visible: bool,
    pub town: usize,
    /// Folded sections. Goods start folded so the panel fits next to the map.
    pub folded: [bool; 5],
    /// Which section header sits on each panel line (filled with the text).
    pub headers: Vec<Option<usize>>,
}

impl EconomyView {
    pub fn new(town: usize) -> Self {
        EconomyView {
            visible: true,
            town,
            folded: [false, false, false, true, false],
            headers: Vec::new(),
        }
    }
}

/// One line of the economy panel, filled every frame.
#[derive(Component)]
pub struct EconLine(pub usize);

#[derive(Component)]
pub struct EconomyRoot;

/// Value label on a trade route from one town to another.
#[derive(Component)]
pub struct FlowLabel(pub usize, pub usize);

pub const LINES: usize = 44;

pub const MINTED: Color = Color::srgb(0.85, 0.6, 1.0);
pub const ISSUED: Color = Color::srgb(0.45, 0.85, 0.45);
pub const TAXED: Color = Color::srgb(0.95, 0.6, 0.3);
pub const SPENT: Color = Color::srgb(0.45, 0.7, 1.0);
pub const TRADE: Color = Color::srgb(0.95, 0.8, 0.35);
const TEXT: Color = Color::srgb(0.9, 0.9, 0.9);
const HEADER: Color = Color::srgb(1.0, 1.0, 1.0);
const DIM: Color = Color::srgb(0.65, 0.65, 0.65);
const ALERT: Color = Color::srgb(1.0, 0.45, 0.4);

pub fn spawn_economy_panel(
    mut commands: Commands,
    towns: Res<Towns>,
    column: Single<Entity, With<crate::hud::LeftColumn>>,
) {
    let font = |size: f32| TextFont {
        font_size: FontSize::Px(size),
        ..default()
    };
    let panel = commands
        .spawn((
            Node {
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            EconomyRoot,
        ))
        .with_children(|p| {
            for k in 0..LINES {
                p.spawn((
                    Text::new(""),
                    font(13.0),
                    TextColor(TEXT),
                    EconLine(k),
                    Button,
                ));
            }
        })
        .id();
    commands.entity(*column).add_child(panel);
    let n = towns.0.len();
    for a in 0..n {
        for b in 0..n {
            if a != b {
                commands.spawn((
                    FlowLabel(a, b),
                    Text2d::new(""),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(TRADE),
                    Transform::from_xyz(0.0, 0.0, 6.0),
                    Visibility::Hidden,
                ));
            }
        }
    }
}

pub fn economy_keys(
    keys: Res<ButtonInput<KeyCode>>,
    towns: Res<Towns>,
    mut view: ResMut<EconomyView>,
) {
    if keys.just_pressed(KeyCode::KeyE) {
        view.visible = !view.visible;
    }
    if keys.just_pressed(KeyCode::Tab) && !towns.0.is_empty() {
        view.town = (view.town + 1) % towns.0.len();
    }
}

/// Clicking a section header folds or unfolds it.
pub fn fold_sections(
    mut view: ResMut<EconomyView>,
    lines: Query<(&Interaction, &EconLine), Changed<Interaction>>,
) {
    for (interaction, line) in &lines {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(Some(s)) = view.headers.get(line.0).copied() {
            view.folded[s] = !view.folded[s];
        }
    }
}

fn bar(v: f32, max: f32, width: usize) -> String {
    let n = ((v / max.max(1e-6)).clamp(0.0, 1.0) * width as f32).round() as usize;
    format!("{}{}", "#".repeat(n), ".".repeat(width - n))
}

/// One panel line: text, color, and the section it heads (for headers).
pub type PanelLine = (String, Color, Option<usize>);

/// The panel for town `t`, with folded sections left out.
pub fn economy_lines(
    town: &Town,
    t: usize,
    player: Option<u16>,
    folded: &[bool; 5],
    store: &AgentStore,
    db: &Db,
    params: &Params,
) -> Vec<PanelLine> {
    let mut out: Vec<PanelLine> = Vec::new();
    let mut line = |s: String, c: Color| out.push((s, c, None));
    let you = if player == Some(t as u16) {
        "  (your town)"
    } else {
        ""
    };
    line(format!("ECONOMY  {}{you}", town.name), TEXT);
    line("E hide | Tab next town | click a heading".into(), DIM);
    let header = |out: &mut Vec<PanelLine>, s: usize| {
        let mark = if folded[s] { "[+]" } else { "[-]" };
        out.push((format!("{mark} {}", SECTIONS[s]), HEADER, Some(s)));
        !folded[s]
    };

    // Money: coin struck, what the treasury issues, takes back as tax, and takes
    // in at market.
    if header(&mut out, MONEY) {
        let mut line = |s: String, c: Color| out.push((s, c, None));
        let l: &Ledger = &town.ledger;
        let minted = l.monthly_group(Group::Minted);
        let issued = l.monthly_group(Group::Issued);
        let taxed = l.monthly_group(Group::Taxed);
        let spent = l.monthly_group(Group::Spent);
        let ext_in = l.monthly_group(Group::ExternalIn);
        let ext_out = l.monthly_group(Group::ExternalOut);
        let max = minted.max(issued).max(taxed).max(spent);
        let detail = |g: Group| {
            let mut parts: Vec<(f32, &str)> = Flow::ALL
                .iter()
                .filter(|f| f.group() == g)
                .map(|f| (l.monthly[*f as usize], f.name()))
                .filter(|x| x.0 >= 0.05)
                .collect();
            parts.sort_by(|a, b| b.0.total_cmp(&a.0));
            let s = parts
                .iter()
                .take(2)
                .map(|(v, n)| format!("{n} {v:.1}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("      {s}")
        };
        line(
            format!("  minted {} {minted:>6.1}", bar(minted, max, 14)),
            MINTED,
        );
        let mint = db
            .content
            .product("mint")
            .map_or(0, |m| town.building_count(m));
        let minting = match town.policy.minting {
            sim_core::policy::Minting::Auto => "auto",
            sim_core::policy::Minting::On => "on",
            sim_core::policy::Minting::Off => "off",
        };
        line(
            match mint {
                0 => format!("      no mint (minting {minting})"),
                n => format!("      {n} mint, minting {minting}"),
            },
            MINTED.with_alpha(0.75),
        );
        // Trend: this month's daily rate against last year's.
        let dpy = params.life.days_per_year.max(1) as f32;
        let trend = |g: Group, now: f32| {
            let before = l.last_year_group(g) / dpy;
            if before < 0.05 {
                String::new()
            } else {
                format!("  {:+.0}% vs last yr", 100.0 * (now / before - 1.0))
            }
        };
        for (name, v, g, c) in [
            ("issued", issued, Group::Issued, ISSUED),
            ("taxed ", taxed, Group::Taxed, TAXED),
            ("spent ", spent, Group::Spent, SPENT),
        ] {
            line(
                format!("  {name} {} {v:>6.1}{}", bar(v, max, 14), trend(g, v)),
                c,
            );
            line(detail(g), c.with_alpha(0.75));
        }
        line(format!("  trade  in {ext_in:.1}, out {ext_out:.1}"), TRADE);
        let held: f32 = town
            .residents
            .iter()
            .map(|&i| store.wealth[i as usize].max(0.0))
            .sum();
        line(
            format!("  treasury {:.0}, people hold {:.0}", town.treasury, held),
            DIM,
        );
    }

    // The town's economic levers, each against the default from models.ron.
    if header(&mut out, LEVERS) {
        let mut line = |s: String, c: Color| out.push((s, c, None));
        let p = &town.policy;
        let mp = &params.market;
        let mark = |set: bool| if set { "" } else { " (default)" };
        line(
            format!(
                "  wealth tax {:.2}/yr{}, income tax {:.0}%",
                p.wealth_tax.unwrap_or(mp.wealth_tax),
                mark(p.wealth_tax.is_some()),
                100.0 * p.income_tax
            ),
            TAXED,
        );
        line(
            format!(
                "  wages {:.0}% of value added{}",
                100.0 * p.wage_share.unwrap_or(params.work.wage_share),
                mark(p.wage_share.is_some())
            ),
            ISSUED,
        );
        line(
            format!(
                "  spending {:.1}/yr of the surplus{}",
                p.public_spending.unwrap_or(mp.public_spending),
                mark(p.public_spending.is_some())
            ),
            ISSUED,
        );
        let subsidies: Vec<String> = p
            .subsidy
            .iter()
            .enumerate()
            .filter(|(_, &s)| s > 0.0)
            .map(|(i, s)| format!("{} {:.0}%", db.content.products[i].name, 100.0 * s))
            .collect();
        if !subsidies.is_empty() {
            line(format!("  subsidies: {}", subsidies.join(", ")), ISSUED);
        }
        for prod in town.orders_eating_food(db, params) {
            let days = town.food_stock(db)
                / (town.residents.len().max(1) as f32 * params.needs.hunger_per_day).max(1e-6);
            line(
                format!(
                    "  ! the {} order takes food people need: {:.0} days left",
                    db.content.products[prod as usize].name.to_lowercase(),
                    days
                ),
                ALERT,
            );
        }
        if p.procurement.is_empty() {
            line("  no open government orders".into(), DIM);
        }
        let delivered: Vec<String> = town
            .procured
            .iter()
            .enumerate()
            .filter(|(_, &q)| q > 0.0)
            .map(|(i, q)| format!("{} {:.0}", db.content.products[i].name, q))
            .collect();
        if !delivered.is_empty() {
            line(
                format!("  delivered so far: {}", delivered.join(", ")),
                SPENT,
            );
        }
        for &(prod, left) in &p.procurement {
            let i = prod as usize;
            line(
                format!(
                    "  order: {:.0} more {} ({:.0} delivered)",
                    left,
                    db.content.products[i].name,
                    town.procured.get(i).copied().unwrap_or(0.0)
                ),
                SPENT,
            );
        }
    }

    // Employment today.
    if header(&mut out, WORK) {
        let mut line = |s: String, c: Color| out.push((s, c, None));
        let (mut working, mut idle, mut resting, mut soldiers, mut adults) = (0, 0, 0, 0, 0usize);
        for &i in &town.residents {
            let i = i as usize;
            if !store.alive[i] {
                continue;
            }
            match store.activity[i] {
                Activity::Child | Activity::Captive => continue,
                Activity::Soldier => soldiers += 1,
                _ if store.worked_today[i].is_some() => working += 1,
                Activity::Resting => resting += 1,
                _ => idle += 1,
            }
            adults += 1;
        }
        let pct = |n: usize| 100.0 * n as f32 / adults.max(1) as f32;
        line(
            format!(
                "  {adults} adults: worked {:.0}%, rested {:.0}%",
                pct(working),
                pct(resting)
            ),
            TEXT,
        );
        line(
            format!("  idle {:.0}%, soldiers {:.0}%", pct(idle), pct(soldiers)),
            if pct(idle) > 25.0 { ALERT } else { TEXT },
        );
    }

    // Goods: foods first, then what the town made most of last year. Coin is money,
    // not a good.
    let goods: Vec<usize> = (0..db.content.products.len())
        .filter(|&p| {
            db.category(p as u32) != Category::Building
                && db.opens[p].is_none()
                && db.coin[p] == 0.0
        })
        .collect();
    if header(&mut out, GOODS) {
        let mut line = |s: String, c: Color| out.push((s, c, None));
        let mut shown: Vec<usize> = db
            .foods
            .iter()
            .map(|&(p, _)| p as usize)
            .filter(|&p| town.stock[p] > 0.5 || town.produced_last_year[p] > 0.0)
            .collect();
        let mut by_value: Vec<usize> = goods
            .iter()
            .copied()
            .filter(|p| !shown.contains(p) && town.produced_last_year[*p] > 0.0)
            .collect();
        by_value.sort_by(|&a, &b| {
            (town.produced_last_year[b] * db.base_price[b])
                .total_cmp(&(town.produced_last_year[a] * db.base_price[a]))
        });
        shown.extend(by_value.into_iter().take(4));
        line(
            format!("  {:<14} {:>6} {:>6} {:>5}", "", "stock", "price", "/base"),
            DIM,
        );
        for &p in &shown {
            let ratio = town.price[p] / db.base_price[p].max(1e-6);
            let c = if ratio >= 1.5 {
                ALERT
            } else if ratio <= 0.6 {
                DIM
            } else {
                TEXT
            };
            let name: String = db.content.products[p].name.chars().take(14).collect();
            line(
                format!(
                    "  {:<14} {:>6.0} {:>6.1} {:>4.1}x",
                    name, town.stock[p], town.price[p], ratio
                ),
                c,
            );
        }
    }

    // Food, and goods selling far above their base price.
    if header(&mut out, FOOD) {
        let mut line = |s: String, c: Color| out.push((s, c, None));
        let mut short: Vec<(usize, f32)> = goods
            .iter()
            .map(|&p| (p, town.price[p] / db.base_price[p].max(1e-6)))
            .filter(|&(_, r)| r >= 1.5)
            .collect();
        short.sort_by(|a, b| b.1.total_cmp(&a.1));
        let food_days = town.food_stock(db)
            / (town.residents.len().max(1) as f32 * params.needs.hunger_per_day).max(1e-6);
        line(
            format!(
                "  food for {:.0} days, {} hungry days this year",
                food_days, town.starving
            ),
            if food_days < 30.0 || town.starving > 0 {
                ALERT
            } else {
                TEXT
            },
        );
        if short.is_empty() {
            line("  no shortages".into(), DIM);
        } else {
            let names: Vec<String> = short
                .iter()
                .take(6)
                .map(|&(p, r)| format!("{} {:.1}x", db.content.products[p].name, r))
                .collect();
            for (k, pair) in names.chunks(2).enumerate() {
                let head = if k == 0 { "  short: " } else { "         " };
                line(format!("{head}{}", pair.join(", ")), ALERT);
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn update_economy_panel(
    mut view: ResMut<EconomyView>,
    towns: Res<Towns>,
    store: Res<AgentStore>,
    db: Res<Db>,
    params: Res<Params>,
    player: Res<sim_core::policy::PlayerTown>,
    mut root: Single<&mut Visibility, With<EconomyRoot>>,
    mut lines: Query<(&EconLine, &mut Text, &mut TextColor, &mut Node)>,
) {
    **root = if view.visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if !view.visible {
        return;
    }
    let t = view.town.min(towns.0.len().saturating_sub(1));
    let Some(town) = towns.0.get(t) else {
        return;
    };
    let content = economy_lines(town, t, player.0, &view.folded, &store, &db, &params);
    view.headers = content.iter().map(|l| l.2).collect();
    for (l, mut text, mut color, mut node) in &mut lines {
        match content.get(l.0) {
            Some((s, c, _)) => {
                text.0.clone_from(s);
                color.0 = *c;
                node.display = Display::Flex;
            }
            None => {
                text.0.clear();
                node.display = Display::None;
            }
        }
    }
}

/// The match goal, when the scenario sets one.
pub fn goal_text(
    scenario: &ScenarioSpec,
    state: &MatchState,
    towns: &Towns,
    db: &Db,
) -> Option<String> {
    let goal = scenario.0.goal.as_ref()?;
    let p = state.product?;
    let name = &db.content.products[p as usize].name;
    if let Some(r) = state.result {
        let how = match r.reason {
            WinReason::Goal => "reached the goal",
            WinReason::TimeUp => "was closest when time ran out",
        };
        return Some(format!(
            "{} WINS in year {}: it {how} ({:.0} of {:.0} {name})",
            towns.0[r.winner as usize].name,
            r.year,
            state.made.get(r.winner as usize).copied().unwrap_or(0.0),
            goal.amount
        ));
    }
    Some(format!(
        "GOAL: first free town to make {:.0} {name}\n      by year {} (see the goal column)",
        goal.amount, goal.years
    ))
}

/// Caravan routes as arrows: brighter and wider the more value they carry.
pub fn draw_flows(
    mut gizmos: Gizmos,
    towns: Res<Towns>,
    overlay: Res<crate::view::Overlay>,
    mut labels: Query<(&FlowLabel, &mut Text2d, &mut Transform, &mut Visibility)>,
) {
    let max = towns
        .0
        .iter()
        .flat_map(|t| t.ledger.exports.iter().copied())
        .fold(1.0f32, f32::max);
    for (label, mut text, mut tf, mut vis) in &mut labels {
        let (a, b) = (label.0, label.1);
        let v = towns
            .0
            .get(a)
            .and_then(|t| t.ledger.exports.get(b).copied())
            .unwrap_or(0.0);
        if v < 0.5 || overlay.product_space {
            *vis = Visibility::Hidden;
            continue;
        }
        let pa = Vec2::new(towns.0[a].pos.0, towns.0[a].pos.1);
        let pb = Vec2::new(towns.0[b].pos.0, towns.0[b].pos.1);
        let dir = (pb - pa).normalize_or_zero();
        // Two-way routes sit side by side.
        let side = Vec2::new(-dir.y, dir.x) * 14.0;
        let (from, to) = (pa + dir * 115.0 + side, pb - dir * 115.0 + side);
        let w = (v / max).sqrt();
        let color = TRADE.with_alpha(0.25 + 0.7 * w);
        let lanes = 1 + (w * 4.0).round() as i32;
        for k in 0..lanes {
            let off = Vec2::new(-dir.y, dir.x) * (k as f32 - (lanes - 1) as f32 / 2.0) * 2.0;
            gizmos.line_2d(from + off, to + off, color);
        }
        gizmos.arrow_2d(to - dir * 30.0, to, color);
        let mid = (from + to) / 2.0 + side * 1.4;
        tf.translation = mid.extend(6.0);
        text.0 = format!("{v:.0}/day");
        *vis = Visibility::Inherited;
    }
}

/// A ring per town showing the month's money: minted (violet), issued (green),
/// taxed (orange), spent (blue), as arcs proportional to each flow.
pub fn draw_money_rings(
    mut gizmos: Gizmos,
    view: Res<EconomyView>,
    towns: Res<Towns>,
    overlay: Res<crate::view::Overlay>,
) {
    if overlay.product_space {
        return;
    }
    let groups = [
        (Group::Minted, MINTED),
        (Group::Issued, ISSUED),
        (Group::Taxed, TAXED),
        (Group::Spent, SPENT),
    ];
    let total = |l: &Ledger| groups.iter().map(|g| l.monthly_group(g.0)).sum::<f32>();
    let max = towns
        .0
        .iter()
        .map(|t| total(&t.ledger))
        .fold(1.0f32, f32::max);
    for (t, town) in towns.0.iter().enumerate() {
        let l = &town.ledger;
        let sum = total(l);
        if sum <= 0.0 {
            continue;
        }
        let center = Vec2::new(town.pos.0, town.pos.1);
        let radius = 100.0 + 6.0;
        // The full ring is the busiest town; a smaller arc means less money moving.
        let span = std::f32::consts::TAU * (sum / max).sqrt();
        let mut start = 0.0f32;
        for (g, c) in groups {
            let arc = span * l.monthly_group(g) / sum;
            for r in 0..3 {
                gizmos.arc_2d(
                    Isometry2d::new(center, Rot2::radians(-start - arc / 2.0)),
                    arc,
                    radius + r as f32 * 1.5,
                    c,
                );
            }
            start += arc;
        }
        if view.visible && view.town == t {
            gizmos.circle_2d(center, 118.0, Color::srgba(1.0, 1.0, 1.0, 0.5));
        }
    }
}
