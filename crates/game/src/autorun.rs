//! Command-line options for the game, mainly for scripted runs:
//!
//! ```text
//! econoplex [--load FILE] [--speed 1-9] [--shot FILE [--shot-day N | --shot-battle S] [--exit]]
//!           [--town NAME] [--camera X Y] [--zoom F] [--no-economy] [--product-space]
//!           [--report why|town|history|log|person] [--select ID] [--culture]
//!           [--set PATH=VALUE]... [--battle LINEUP]
//! ```
//!
//! `--set` changes models.ron for this run, as in the CLI (e.g.
//! `--set battle.model=Soldiers`). `--battle` fights a line-up (as
//! `econoplex-sim --battle`) on `battle.model`, plays it beside the player's
//! town as the 3D client does, and starts the camera on its field.
//!
//! `--shot` runs until day N (default: right away), pauses, lets the view settle,
//! writes a PNG screenshot and, with `--exit`, quits. With `--shot-battle S` it
//! waits instead for a live battle to have been fought S battle seconds, and
//! looks at its field; with `--battle`, for the staged battle's playback to
//! reach S battle seconds. Either way it then stops the clock, so the battle
//! holds still for the shot. A live battle waiting for its deployment is
//! started for the shot, except with `--shot-battle 0`, which shows the
//! deployment. Used to make the README and PR screenshots.

use crate::view::WorldCamera;
use crate::SimControl;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use std::path::PathBuf;

#[derive(Resource, Default, Clone)]
pub struct GameArgs {
    pub load: Option<PathBuf>,
    pub speed: Option<usize>,
    pub shot: Option<PathBuf>,
    pub shot_day: u64,
    /// Take the shot once a live battle has been fought this many battle seconds.
    pub shot_battle: Option<f32>,
    pub exit: bool,
    /// Town shown in the economy panel at start.
    pub town: Option<String>,
    /// Camera center and zoom (world units per pixel; the default is 1.6).
    pub camera: Option<(f32, f32)>,
    pub zoom: Option<f32>,
    pub no_economy: bool,
    pub product_space: bool,
    /// Start with the culture overlay on.
    pub culture: bool,
    /// The town the player governs (overrides scenario.ron).
    pub player: Option<String>,
    /// Orders given on day 0, as typed in the CLI (e.g. "tax Greenvale income 0.1").
    pub orders: Vec<String>,
    /// Changes to models.ron (`--set PATH=VALUE`).
    pub sets: Vec<(String, String)>,
    /// A line-up to fight and play back at start (`--battle`), and where on
    /// the map its field lies once staged.
    pub battle: Option<String>,
    pub battle_at: Option<(f32, f32)>,
    /// The report open at start (reports.rs), and a person selected at start.
    pub report: Option<crate::reports::Report>,
    pub select: Option<u32>,
    /// Show a battle control without a mouse, for screenshots (`--rts-demo`).
    pub rts_demo: Option<crate::battle_input::Demo>,
}

pub fn parse() -> GameArgs {
    let mut a = GameArgs::default();
    let mut it = std::env::args().skip(1);
    let die = |m: &str| -> ! {
        eprintln!("error: {m}");
        std::process::exit(2)
    };
    while let Some(arg) = it.next() {
        let mut val = || {
            it.next()
                .unwrap_or_else(|| die(&format!("{arg} needs a value")))
        };
        match arg.as_str() {
            "--load" => a.load = Some(PathBuf::from(val())),
            "--speed" => {
                let s: usize = val().parse().unwrap_or_else(|_| die("--speed expects 1-9"));
                a.speed = Some(s.clamp(1, crate::SPEEDS.len()) - 1);
            }
            "--shot" => a.shot = Some(PathBuf::from(val())),
            "--shot-day" => {
                a.shot_day = val()
                    .parse()
                    .unwrap_or_else(|_| die("--shot-day expects a day"))
            }
            "--shot-battle" => {
                a.shot_battle = Some(
                    val()
                        .parse()
                        .unwrap_or_else(|_| die("--shot-battle expects battle seconds")),
                )
            }
            "--exit" => a.exit = true,
            "--town" => a.town = Some(val()),
            "--camera" => {
                let x = val()
                    .parse()
                    .unwrap_or_else(|_| die("--camera expects X Y"));
                let y = val()
                    .parse()
                    .unwrap_or_else(|_| die("--camera expects X Y"));
                a.camera = Some((x, y));
            }
            "--zoom" => {
                a.zoom = Some(
                    val()
                        .parse()
                        .unwrap_or_else(|_| die("--zoom expects a number")),
                )
            }
            "--no-economy" => a.no_economy = true,
            "--report" => {
                a.report =
                    Some(crate::reports::Report::named(&val()).unwrap_or_else(|| {
                        die("--report expects why, town, history, log or person")
                    }))
            }
            "--select" => {
                a.select = Some(
                    val()
                        .parse()
                        .unwrap_or_else(|_| die("--select expects an agent id")),
                )
            }
            "--product-space" => a.product_space = true,
            "--culture" => a.culture = true,
            "--player" => a.player = Some(val()),
            "--order" => a.orders.push(val()),
            "--set" => {
                let v = val();
                let (k, x) = v
                    .split_once('=')
                    .unwrap_or_else(|| die("--set expects path=value"));
                a.sets.push((k.to_string(), x.to_string()));
            }
            "--battle" => a.battle = Some(val()),
            "--rts-demo" => a.rts_demo = Some(val().parse().unwrap_or_else(|e: String| die(&e))),
            "-h" | "--help" => {
                println!(
                    "econoplex: the Econoplex game\n\n\
                     --load FILE      start from a saved game\n\
                     --speed N        start at speed N (1-9)\n\
                     --shot FILE      save a PNG screenshot once the game reaches --shot-day\n\
                     --shot-day N     simulated day (month) of the screenshot (default: at once)\n\
                     --shot-battle S  instead, once a live (or --battle) battle has been fought S seconds\n\
                     \x20                (0: while it waits for the deployment)\n\
                     --exit           quit after the screenshot\n\
                     --town NAME      town shown in the economy panel at start\n\
                     --camera X Y     center the camera on a world position\n\
                     --zoom F         camera zoom (default 1.6; smaller is closer)\n\
                     --no-economy     start with the economy panel closed\n\
                     --product-space  start with the product space overlay (P) on\n\
                     --player NAME    govern this town (default: scenario.ron's player)\n\
                     --order TEXT     give an order on day 0 (repeatable), as in the CLI\n\
                     --set PATH=VALUE change models.ron for this run (repeatable)\n\
                     --battle LINEUP  fight a line-up on battle.model and play it back\n\
                     --rts-demo WHAT  in a live battle, show `ghost` (your squads selected, a line\n\
                     \x20                laid out ahead of them), `overlay` (morale and fatigue), `deploy`\n\
                     \x20                (the zone and a line dragged in it; with --shot-battle 0) or\n\
                     \x20                `queue` (two waypoints queued for each squad, and their paths)"
                );
                std::process::exit(0);
            }
            other => die(&format!("unknown argument `{other}` (try --help)")),
        }
    }
    a
}

/// Apply the camera and panel options once the view has been set up.
pub fn apply_view_args(
    args: Res<GameArgs>,
    mut camera: Single<&mut Transform, With<WorldCamera>>,
    mut economy: ResMut<crate::economy_ui::EconomyView>,
    mut overlay: ResMut<crate::view::Overlay>,
) {
    // A staged battle is what there is to watch: start on its field, close
    // enough in to tell its soldiers apart.
    if let Some((x, y)) = args.camera.or(args.battle_at) {
        camera.translation.x = x;
        camera.translation.y = y;
    }
    let staged = args.battle_at.map(|_| crate::live_battle::LOOK_ZOOM);
    if let Some(z) = args.zoom.or(staged) {
        camera.scale = Vec3::new(z, z, 1.0);
    }
    if args.no_economy {
        economy.visible = false;
    }
    overlay.product_space |= args.product_space;
    overlay.culture |= args.culture;
}

/// Load `--load` once the view has been set up.
pub fn load_at_start(world: &mut World) {
    let Some(path) = world.resource::<GameArgs>().load.clone() else {
        return;
    };
    match crate::saving::load_game(world, &path) {
        Ok(day) => info!("loaded {} at day {day}", path.display()),
        Err(e) => {
            eprintln!("error: cannot load {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}

#[derive(Resource, Default)]
pub struct ShotState {
    /// Frames since the target day was reached.
    frames: u32,
    taken: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn autoshot(
    mut commands: Commands,
    args: Res<GameArgs>,
    clock: Res<sim_core::SimClock>,
    mut live: ResMut<crate::live_battle::LiveBattle>,
    scenes: Res<crate::battles::BattleScenes>,
    mut time: ResMut<Time<Virtual>>,
    mut camera: Single<&mut Transform, With<WorldCamera>>,
    mut follow: ResMut<crate::view::Follow>,
    mut control: ResMut<SimControl>,
    mut state: ResMut<ShotState>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &args.shot else {
        return;
    };
    if state.frames == 0 {
        match args.shot_battle {
            // The staged battle, where the camera already looks.
            Some(s) if args.battle_at.is_some() => {
                if scenes.newest_at(time.elapsed_secs()).is_none_or(|t| t < s) {
                    return;
                }
                time.pause();
            }
            Some(s) => {
                // A scripted shot starts the battle itself, unless it is to
                // show the deployment (`--shot-battle 0`).
                if live.view.as_ref().is_some_and(|v| v.deploying) && s > 0.0 {
                    live.start = true;
                }
                let Some(v) = live
                    .view
                    .as_ref()
                    .filter(|v| v.frame.t >= s || v.finished || (v.deploying && s <= 0.0))
                else {
                    return;
                };
                follow.0 = false;
                crate::view::look_at(&mut camera, crate::live_battle::look_point(v));
                let z = args.zoom.unwrap_or(crate::live_battle::LOOK_ZOOM);
                camera.scale = Vec3::new(z, z, 1.0);
                time.pause();
            }
            None if clock.tick < args.shot_day => return,
            None => {}
        }
    }
    control.paused = true;
    state.frames += 1;
    // Give agents time to walk to their places and the HUD to refresh.
    if state.frames == 120 && !state.taken {
        state.taken = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if args.exit && state.frames == 180 {
        exit.write(AppExit::Success);
    }
}
