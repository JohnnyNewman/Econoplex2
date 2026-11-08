//! Command-line options for the game, mainly for scripted runs:
//!
//! ```text
//! econoplex [--load FILE] [--speed 1-5] [--shot FILE [--shot-day N] [--exit]]
//!           [--town NAME] [--camera X Y] [--zoom F] [--no-economy] [--product-space]
//! ```
//!
//! `--shot` runs until day N (default: right away), pauses, lets the view settle,
//! writes a PNG screenshot and, with `--exit`, quits. Used to make the README and PR
//! screenshots.

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
    pub exit: bool,
    /// Town shown in the economy panel at start.
    pub town: Option<String>,
    /// Camera center and zoom (world units per pixel; the default is 1.6).
    pub camera: Option<(f32, f32)>,
    pub zoom: Option<f32>,
    pub no_economy: bool,
    pub product_space: bool,
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
                let s: usize = val().parse().unwrap_or_else(|_| die("--speed expects 1-5"));
                a.speed = Some(s.clamp(1, crate::SPEEDS.len()) - 1);
            }
            "--shot" => a.shot = Some(PathBuf::from(val())),
            "--shot-day" => {
                a.shot_day = val()
                    .parse()
                    .unwrap_or_else(|_| die("--shot-day expects a day"))
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
            "--product-space" => a.product_space = true,
            "-h" | "--help" => {
                println!(
                    "econoplex: the Econoplex game\n\n\
                     --load FILE      start from a saved game\n\
                     --speed N        start at speed N (1-5)\n\
                     --shot FILE      save a PNG screenshot once the game reaches --shot-day\n\
                     --shot-day N     day of the screenshot (default: at once)\n\
                     --exit           quit after the screenshot
                     --town NAME      town shown in the economy panel at start
                     --camera X Y     center the camera on a world position
                     --zoom F         camera zoom (default 1.6; smaller is closer)
                     --no-economy     start with the economy panel closed
                     --product-space  start with the product space overlay (P) on"
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
    mut camera: Single<&mut Transform, With<Camera2d>>,
    mut economy: ResMut<crate::economy_ui::EconomyView>,
    mut overlay: ResMut<crate::view::Overlay>,
) {
    if let Some((x, y)) = args.camera {
        camera.translation.x = x;
        camera.translation.y = y;
    }
    if let Some(z) = args.zoom {
        camera.scale = Vec3::new(z, z, 1.0);
    }
    if args.no_economy {
        economy.visible = false;
    }
    overlay.product_space |= args.product_space;
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

pub fn autoshot(
    mut commands: Commands,
    args: Res<GameArgs>,
    clock: Res<sim_core::SimClock>,
    mut control: ResMut<SimControl>,
    mut state: ResMut<ShotState>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &args.shot else {
        return;
    };
    if clock.tick < args.shot_day {
        return;
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
