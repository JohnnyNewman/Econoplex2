//! Colour grading by season, time of day and weather (step 18,
//! docs/shaders/18-grading.md).
//!
//! The light already changes with the sun and the weather (sky.rs), and the
//! ground with the season (fog.wesl, `seasonal`). A grade on top gives each
//! moment its own mood, the way a painter would: the low sun's golden hour
//! warm and a little richer, night cool and drained of colour, autumn warm
//! and saturated, winter cold and grey, spring a touch greener, an overcast
//! or rainy day flat and dull.
//!
//! It is Bevy's own `ColorGrading` on the camera, applied in the tonemapping
//! pass that runs anyway: no extra pass, no material changes, one small
//! component written when the grade moves.

use crate::env::Environment;
use crate::sky::SkyClock;
use bevy::prelude::*;
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection};

pub struct GradingPlugin;

impl Plugin for GradingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, grade_camera);
    }
}

/// The parts of a moment that set its grade, each 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mood {
    /// Low sun above the horizon: golden hour.
    pub golden: f32,
    /// Sun well below the horizon.
    pub night: f32,
    pub spring: f32,
    pub autumn: f32,
    pub winter: f32,
    /// Cloud cover and rain.
    pub cloud: f32,
    pub precip: f32,
}

impl Mood {
    pub fn of(env: &Environment, sun_height: f32) -> Self {
        let up = sun_height;
        Mood {
            golden: smooth(-0.06, 0.04, up) * (1.0 - smooth(0.12, 0.35, up)),
            night: smooth(0.0, -0.18, up),
            spring: env.spring(),
            autumn: env.autumn(),
            winter: env.winter(),
            cloud: env.weather.cloud.clamp(0.0, 1.0),
            precip: env.weather.precip.clamp(0.0, 1.0),
        }
    }
}

/// The grade for a mood. Values are small: the scene's own light does most
/// of the work, the grade only leans it.
pub fn grade(m: Mood) -> ColorGrading {
    // Clouds take the golden hour away.
    let golden = m.golden * (1.0 - 0.8 * m.cloud);
    // Bevy's temperature moves the colours a long way: a few hundredths are
    // plenty. Cooling alone turns the image mint (it only moves CIE x), so a
    // little magenta goes with it.
    let temperature = 0.035 * golden - 0.03 * m.night + 0.015 * m.autumn - 0.02 * m.winter;
    let tint = -0.005 * m.spring + 0.5 * (-temperature).max(0.0);
    let saturation = (1.0 + 0.08 * golden + 0.14 * m.autumn + 0.06 * m.spring
        - 0.2 * m.winter
        - 0.3 * m.night
        - 0.15 * m.cloud
        - 0.1 * m.precip)
        .max(0.4);
    let contrast = 1.0 + 0.06 * golden - 0.08 * m.cloud;
    let mid = ColorGradingSection {
        saturation,
        contrast,
        ..default()
    };
    ColorGrading {
        global: ColorGradingGlobal {
            temperature,
            tint,
            ..default()
        },
        shadows: ColorGradingSection {
            saturation,
            ..default()
        },
        midtones: mid,
        highlights: ColorGradingSection {
            saturation,
            ..default()
        },
    }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Grade the world camera for this moment; written only when it moves.
fn grade_camera(
    mut commands: Commands,
    env: Res<Environment>,
    clock: Res<SkyClock>,
    mut cams: Query<(Entity, Option<&mut ColorGrading>), With<crate::camera::WorldCamera>>,
) {
    let want = grade(Mood::of(&env, clock.sun().y));
    for (e, grading) in &mut cams {
        match grading {
            Some(mut g) => {
                g.set_if_neq(want.clone());
            }
            None => {
                commands.entity(e).insert(want.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_summer_noon_is_left_alone() {
        let g = grade(Mood::default());
        assert_eq!(g.global.temperature, 0.0);
        assert_eq!(g.midtones.saturation, 1.0);
    }

    #[test]
    fn evenings_are_warm_nights_cool_and_winters_grey() {
        let evening = grade(Mood {
            golden: 1.0,
            ..default()
        });
        let night = grade(Mood {
            night: 1.0,
            ..default()
        });
        let winter = grade(Mood {
            winter: 1.0,
            ..default()
        });
        let autumn = grade(Mood {
            autumn: 1.0,
            ..default()
        });
        assert!(evening.global.temperature > 0.0);
        assert!(night.global.temperature < 0.0);
        assert!(night.midtones.saturation < 1.0);
        assert!(winter.midtones.saturation < 1.0 && winter.global.temperature < 0.0);
        assert!(autumn.midtones.saturation > 1.0);
        // An overcast evening loses most of its gold.
        let grey_evening = grade(Mood {
            golden: 1.0,
            cloud: 1.0,
            ..default()
        });
        assert!(grey_evening.global.temperature < evening.global.temperature * 0.5);
    }
}
