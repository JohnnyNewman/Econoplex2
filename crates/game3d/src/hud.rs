//! The help text in the corner: time of day, which shader the number keys
//! control, and each shader's current debug view.

use crate::fog::FogSettings;
use crate::sky::{DebugTarget, SkyClock};
use crate::terrain::TerrainSettings;
use crate::toon::ToonSettings;
use bevy::prelude::*;

/// The help text, with its title and camera hint.
#[derive(Component)]
pub struct Help {
    pub title: &'static str,
    pub camera: &'static str,
}

pub fn spawn_help(commands: &mut Commands, title: &'static str, camera: &'static str) {
    commands.spawn((
        Help { title, camera },
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            top: Val::Px(10.0),
            ..default()
        },
    ));
}

pub fn update_help(
    clock: Res<SkyClock>,
    toon: Res<ToonSettings>,
    terrain: Res<TerrainSettings>,
    fog: Res<FogSettings>,
    target: Res<DebugTarget>,
    mut text: Query<(&mut Text, &Help)>,
) {
    if !clock.is_changed()
        && !toon.is_changed()
        && !terrain.is_changed()
        && !fog.is_changed()
        && !target.is_changed()
    {
        return;
    }
    let h = clock.hour;
    let mark = |t: DebugTarget| if *target == t { ">" } else { " " };
    for (mut t, help) in &mut text {
        t.0 = format!(
            "{}\n\
         time {:02}:{:02}  [ ] change  T run ({})\n\
         {} sky view {}: {}\n\
         {} toon view {}: {}   M toon/PBR ({})\n\
         {} terrain view {}: {}   L grid step {}\n\
         {} fog view {}: {}   F fog ({})\n\
         Tab: which shader keys 0-4 control\n\
         {}\n\
         edit assets/shaders/*.wesl and save to reload",
            help.title,
            h.floor() as u32,
            (h.fract() * 60.0) as u32,
            if clock.running { "on" } else { "off" },
            mark(DebugTarget::Sky),
            clock.debug,
            SkyClock::VIEWS[clock.debug as usize],
            mark(DebugTarget::Toon),
            toon.debug,
            ToonSettings::VIEWS[toon.debug as usize],
            if toon.on { "toon" } else { "PBR" },
            mark(DebugTarget::Terrain),
            terrain.debug,
            TerrainSettings::VIEWS[terrain.debug as usize],
            terrain.step,
            mark(DebugTarget::Fog),
            fog.debug,
            FogSettings::VIEWS[fog.debug as usize],
            if fog.on { "on" } else { "off" },
            help.camera,
        );
    }
}
