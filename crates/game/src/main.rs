//! Econoplex: a simple 2D view of the economic simulation.
//!
//! The simulation lives in the same ECS world as the view. Each `FixedUpdate`
//! runs the economy steps that are due at the game speed (five minutes of
//! real time a simulated day is about a step a second); the view attaches
//! sprites to simulation entities and animates agents toward where they work.
//!
//! Controls: Space pause · 1-9 speed · WASD/arrows pan · mouse wheel zoom ·
//! left click select agent · Esc deselect · F follow the selected person ·
//! K colour by best skill (else by today's work) · P product space overlay ·
//! E economy panel · Tab next town (the camera goes there) · F5 quicksave ·
//! F9 quickload · policy panel (bottom right) for the player's town
//! · Y why · T town · H history charts · L event log · I person (reports.rs)
//! · F2 hide all panels · the trade arrows carry caravans (economy_ui.rs)
//! · the player's own battles are fought live, with orders (live_battle.rs)

mod autorun;
mod battles;
mod economy_ui;
mod hud;
mod live_battle;
mod policy_ui;
mod reports;
mod saving;
mod seasons;
mod terrain_view;
mod ui_compat;
mod view;

use bevy::prelude::*;
use sim_core::AssetPaths;

/// Game speeds in simulated days per real second, as in the 3D client: 5
/// minutes a day (about an economy step a second at the default
/// `time.steps_per_day`), 1 minute, 12 seconds, then 1 to 120 days a second
/// for letting the economy run. The fastest speeds run as many steps as a
/// frame has time for.
pub const SPEEDS: [f64; 9] = [
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

/// `FixedUpdate` runs this often; each run takes the steps that are due.
const FIXED_HZ: f64 = 60.0;
/// Longest a fixed update may spend on steps before the frame goes on.
const STEP_BUDGET: std::time::Duration = std::time::Duration::from_millis(30);

#[derive(Resource)]
pub struct SimControl {
    pub paused: bool,
    pub speed: usize,
    /// Steps due and not yet run (fractions carry over).
    pub due: f64,
}

#[derive(Resource, Default)]
pub struct Selection(pub Option<u32>);

fn main() {
    let mut args = autorun::parse();
    let paths = AssetPaths::discover();
    let (db, params, mut scenario) = match sim_core::load_with(&paths, &args.sets) {
        Ok(x) => x,
        Err(e) => {
            eprintln!(
                "failed to load Econoplex assets from {}: {e}",
                paths.content.display()
            );
            std::process::exit(1);
        }
    };

    if let Some(name) = &args.player {
        if !scenario.towns.iter().any(|t| &t.name == name) {
            eprintln!("error: no town `{name}` to govern");
            std::process::exit(2);
        }
        scenario.player = Some(name.clone());
    }

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Econoplex".into(),
            ..default()
        }),
        ..default()
    }))
    // The button state the panels read (Bevy 0.20 UI widgets are part of
    // DefaultPlugins with the bevy_ui_widgets feature).
    .add_systems(PreUpdate, ui_compat::update_interaction);

    sim_core::world::setup(app.world_mut(), db, params, &scenario);
    // The player's squad marches only when ordered.
    if let Some(town) = app.world().resource::<sim_core::policy::PlayerTown>().0 {
        app.world_mut()
            .resource_mut::<sim_core::policy::OrderQueue>()
            .pending
            .push(sim_core::policy::Order::War {
                town,
                war: sim_core::policy::War::Defend,
            });
    }
    for text in &args.orders {
        let order = sim_core::policy::Order::parse(
            text,
            app.world().resource::<sim_core::Db>(),
            app.world().resource::<sim_core::Towns>(),
        )
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(2);
        });
        app.world_mut()
            .resource_mut::<sim_core::policy::OrderQueue>()
            .pending
            .push(order);
    }
    if let Some(lineup) = &args.battle {
        args.battle_at = Some(battles::stage_battle(app.world_mut(), lineup));
    }
    app.add_schedule(sim_core::build_schedule());
    app.add_schedule(sim_core::build_economy_schedule());

    let speed = args.speed.unwrap_or(0);
    let econ_town = args
        .town
        .as_ref()
        .and_then(|name| {
            app.world()
                .resource::<sim_core::Towns>()
                .0
                .iter()
                .position(|t| &t.name == name)
        })
        .or(app
            .world()
            .resource::<sim_core::policy::PlayerTown>()
            .0
            .map(|t| t as usize))
        .unwrap_or(0);
    app.insert_resource(Time::<Fixed>::from_hz(FIXED_HZ))
        .insert_resource(SimControl {
            paused: false,
            speed,
            due: 0.0,
        })
        .insert_resource(args)
        .insert_resource(autorun::ShotState::default())
        .insert_resource(Selection::default())
        .insert_resource(view::Overlay::default())
        .insert_resource(view::Follow::default())
        .insert_resource(seasons::FieldYear::default())
        .insert_resource(seasons::WhyRings::default())
        .insert_resource(live_battle::LiveBattle::default())
        // The player's own battles wait for the player (live_battle.rs).
        .insert_resource(sim_core::systems::military::LiveBattles(true))
        .insert_resource(policy_ui::PolicyCursor::default())
        .insert_resource(policy_ui::Placing::default())
        .insert_resource(policy_ui::SentOrders::default())
        .insert_resource(saving::Toast::default())
        .insert_resource(reports::Reports::default())
        .insert_resource(hud::UiHidden::default())
        .init_gizmo_group::<reports::ChartGizmos>()
        .insert_resource(battles::BattleScenes::default())
        .insert_resource(economy_ui::EconomyView::new(econ_town))
        .insert_resource(ClearColor(Color::srgb(0.08, 0.09, 0.08)))
        .add_systems(
            Startup,
            (
                view::spawn_camera,
                view::spawn_static,
                terrain_view::spawn_terrain,
                view::spawn_skill_labels,
                view::spawn_overlay_shade,
                hud::spawn_hud,
                policy_ui::spawn_policy_panel.after(hud::spawn_hud),
                hud::spawn_help.after(policy_ui::spawn_policy_panel),
                economy_ui::spawn_economy_panel.after(hud::spawn_hud),
                reports::spawn_reports,
                reports::spawn_chart_camera,
                seasons::spawn_night_veil,
                live_battle::spawn_battle_panel,
            ),
        )
        .add_systems(
            PostStartup,
            (
                autorun::apply_view_args,
                autorun::load_at_start,
                reports::apply_args,
            ),
        )
        .add_systems(FixedUpdate, run_sim_tick)
        .add_systems(
            Update,
            (
                economy_ui::economy_keys,
                economy_ui::fold_sections,
                economy_ui::update_economy_panel,
                economy_ui::draw_flows,
                economy_ui::draw_money_rings,
                view::update_overlay_shade,
            ),
        )
        .add_systems(
            Update,
            (
                reports::update_reports
                    .after(view::select_agent)
                    .after(seasons::update_field_year),
                reports::draw_charts.after(reports::update_reports),
                hud::toggle_panels,
            ),
        )
        .add_systems(
            PostUpdate,
            hud::hide_panels
                .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
        )
        .add_systems(
            Update,
            (
                (seasons::update_field_year, seasons::color_fields)
                    .chain()
                    .after(view::attach_nature_sprites),
                seasons::tint_terrain,
                seasons::update_night_veil,
                seasons::draw_precipitation,
                seasons::draw_harvest_rings,
                seasons::draw_why_rings,
            ),
        )
        .add_systems(
            Update,
            (
                view::follow_selected.after(view::animate_agents),
                policy_ui::update_sent_orders
                    .after(policy_ui::policy_buttons)
                    .after(policy_ui::place_building),
            ),
        )
        .add_systems(
            Update,
            (battles::start_battles, battles::draw_battles).chain(),
        )
        .add_systems(
            Update,
            (
                live_battle::draw_live_battle,
                live_battle::battle_buttons,
                live_battle::update_battle_panel.after(live_battle::battle_buttons),
            ),
        )
        .add_systems(
            Update,
            (
                controls,
                saving::quicksave_keys,
                autorun::autoshot,
                view::camera_controls,
                view::attach_agent_sprites,
                view::attach_building_sprites,
                view::attach_nature_sprites,
                view::animate_agents,
                view::draw_world,
                view::draw_batches,
                view::update_skill_labels,
                view::select_agent,
                hud::update_hud,
                hud::update_inspector,
                hud::update_legend,
                hud::toggle_help,
                policy_ui::policy_buttons,
                policy_ui::update_policy_panel,
                policy_ui::place_building.after(view::select_agent),
            ),
        );
    app.run();
}

/// Run the economy steps due at the game speed since the last fixed update.
fn run_sim_tick(world: &mut World) {
    // A battle waiting for the player holds the economy's clock; it runs in
    // real time instead (live_battle.rs).
    let paused = world.resource::<SimControl>().paused;
    if live_battle::run(world, (1.0 / FIXED_HZ) as f32, paused) {
        world.resource_mut::<SimControl>().due = 0.0;
        return;
    }
    let spd = world.resource::<sim_core::SimClock>().steps_per_day.max(1) as f64;
    let mut control = world.resource_mut::<SimControl>();
    if control.paused {
        control.due = 0.0;
        return;
    }
    control.due += SPEEDS[control.speed] * spd / FIXED_HZ;
    let start = std::time::Instant::now();
    while world.resource::<SimControl>().due >= 1.0 {
        world.run_schedule(sim_core::SimTick);
        world.resource_mut::<SimControl>().due -= 1.0;
        if start.elapsed() > STEP_BUDGET {
            // Too slow for this speed: drop what is left rather than fall behind.
            world.resource_mut::<SimControl>().due = 0.0;
            break;
        }
    }
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut control: ResMut<SimControl>,
    mut selection: ResMut<Selection>,
    mut overlay: ResMut<view::Overlay>,
) {
    if keys.just_pressed(KeyCode::Space) {
        control.paused = !control.paused;
    }
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (i, k) in digits.iter().enumerate() {
        if keys.just_pressed(*k) {
            control.speed = i;
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        selection.0 = None;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        overlay.product_space = !overlay.product_space;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        overlay.culture = !overlay.culture;
    }
    if keys.just_pressed(KeyCode::KeyK) {
        overlay.best_skill = !overlay.best_skill;
    }
}
