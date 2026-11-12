//! The towns' buildings, by type, built procedurally from a few primitive
//! shapes.
//!
//! Each building type gets a recognisable silhouette, made of a handful of parts:
//! - a body (box), a roof (gable prism, cone, dome or flat);
//! - one feature that tells the trade: a chimney, a tower with sails, a barrel,
//!   stacked blocks.
//!
//! Positions and types come from the simulation's snapshots (`live.rs`): every
//! building is drawn where the sim put it, appears when it is built and goes
//! when it is gone.
//!
//! **Style (M8):** a town builds in the style of its newest era (eras are
//! display labels): timber and thatch, then plaster and tiles, then brick, then
//! stone and slate. Its houses follow its residents' wealth: small thatched
//! cottages, ordinary houses, or two-storey houses with chimneys.

use crate::outline::Outlines;
use crate::toon::{toon, ToonMaterial};
use bevy::prelude::*;
use std::collections::HashMap;

/// One building from the simulation, in 3D coordinates.
pub struct BuildingSpec {
    /// Product id of the building type, e.g. "mill".
    pub kind: String,
    /// Ground position (y = terrain height).
    pub at: Vec3,
    /// A stable number for small variations (rotation, colour).
    pub seed: u32,
}

/// Meshes and materials shared by all buildings (a resource, so buildings can
/// be added at any time).
#[derive(Resource)]
pub struct Kit {
    cube: Handle<Mesh>,
    gable: Handle<Mesh>,
    cone: Handle<Mesh>,
    cylinder: Handle<Mesh>,
    dome: Handle<Mesh>,
    colours: HashMap<&'static str, Handle<ToonMaterial>>,
}

impl Kit {
    pub fn new(meshes: &mut Assets<Mesh>, toons: &mut Assets<ToonMaterial>) -> Self {
        // A gable roof: a triangle extruded along z (1 wide, 1 high, 1 deep).
        let gable = Extrusion::new(
            Triangle2d::new(
                Vec2::new(-0.5, -0.5),
                Vec2::new(0.5, -0.5),
                Vec2::new(0.0, 0.5),
            ),
            1.0,
        );
        let palette: [(&'static str, Color); 12] = [
            ("plaster", Color::srgb(0.88, 0.82, 0.70)),
            ("timber", Color::srgb(0.52, 0.38, 0.26)),
            ("stone", Color::srgb(0.66, 0.64, 0.60)),
            ("dark_stone", Color::srgb(0.38, 0.37, 0.38)),
            ("brick", Color::srgb(0.66, 0.36, 0.26)),
            ("red_roof", Color::srgb(0.68, 0.27, 0.20)),
            ("brown_roof", Color::srgb(0.50, 0.32, 0.22)),
            ("slate_roof", Color::srgb(0.34, 0.36, 0.44)),
            ("thatch", Color::srgb(0.80, 0.68, 0.40)),
            ("sail", Color::srgb(0.93, 0.90, 0.82)),
            ("soot", Color::srgb(0.22, 0.20, 0.20)),
            ("gold", Color::srgb(0.92, 0.72, 0.25)),
        ];
        Self {
            cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            gable: meshes.add(gable),
            cone: meshes.add(Cone::new(0.5, 1.0)),
            cylinder: meshes.add(Cylinder::new(0.5, 1.0)),
            dome: meshes.add(Sphere::new(0.5).mesh().uv(24, 12)),
            colours: palette
                .into_iter()
                .map(|(name, c)| (name, toons.add(toon(c))))
                .collect(),
        }
    }

    pub fn mat(&self, name: &str) -> Handle<ToonMaterial> {
        self.colours[name].clone()
    }
}

/// The parts of one building: (mesh, colour, offset from the ground point,
/// scale), in the building's own frame (x across, z along, y up).
fn parts(kit: &Kit, kind: &str) -> Vec<(Handle<Mesh>, &'static str, Vec3, Vec3)> {
    let cube = || kit.cube.clone();
    let gable = || kit.gable.clone();
    let cone = || kit.cone.clone();
    let cyl = || kit.cylinder.clone();
    let dome = || kit.dome.clone();
    // Offsets put each part's centre: a box h high stands on the ground at
    // y = h / 2. All buildings are about 2 units across (a house is 1).
    match kind {
        // Long workshop with a timber frame and a gable roof.
        "workshop" => vec![
            (
                cube(),
                "plaster",
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(1.8, 1.0, 3.0),
            ),
            (
                gable(),
                "brown_roof",
                Vec3::new(0.0, 1.35, 0.0),
                Vec3::new(2.0, 0.7, 3.2),
            ),
        ],
        // Round mill tower, cone roof, four sails.
        "mill" => vec![
            (
                cyl(),
                "stone",
                Vec3::new(0.0, 1.1, 0.0),
                Vec3::new(1.4, 2.2, 1.4),
            ),
            (
                cone(),
                "thatch",
                Vec3::new(0.0, 2.6, 0.0),
                Vec3::new(1.6, 0.8, 1.6),
            ),
            (
                cube(),
                "sail",
                Vec3::new(0.0, 2.0, 0.78),
                Vec3::new(0.25, 3.0, 0.05),
            ),
            (
                cube(),
                "sail",
                Vec3::new(0.0, 2.0, 0.78),
                Vec3::new(3.0, 0.25, 0.05),
            ),
        ],
        // Bakery: cosy box, red roof, chimney.
        "bakery" => vec![
            (
                cube(),
                "plaster",
                Vec3::new(0.0, 0.55, 0.0),
                Vec3::new(1.6, 1.1, 1.8),
            ),
            (
                gable(),
                "red_roof",
                Vec3::new(0.0, 1.45, 0.0),
                Vec3::new(1.8, 0.7, 2.0),
            ),
            (
                cube(),
                "brick",
                Vec3::new(0.45, 1.7, -0.5),
                Vec3::new(0.3, 0.9, 0.3),
            ),
        ],
        // Furnace: squat stone, tall dark chimney.
        "furnace" | "blast_furnace" => vec![
            (
                cube(),
                "dark_stone",
                Vec3::new(0.0, 0.45, 0.0),
                Vec3::new(1.8, 0.9, 1.8),
            ),
            (
                cyl(),
                "soot",
                Vec3::new(0.4, 1.6, 0.4),
                Vec3::new(0.5, 3.2, 0.5),
            ),
        ],
        // Smithy: open front (low body), slate roof, chimney.
        "smithy" => vec![
            (
                cube(),
                "timber",
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(1.8, 1.0, 1.8),
            ),
            (
                gable(),
                "slate_roof",
                Vec3::new(0.0, 1.35, 0.0),
                Vec3::new(2.0, 0.7, 2.0),
            ),
            (
                cube(),
                "dark_stone",
                Vec3::new(-0.6, 1.6, 0.5),
                Vec3::new(0.35, 1.2, 0.35),
            ),
        ],
        // Weaving shed: long and low.
        "weaving_shed" | "textile_mill" => vec![
            (
                cube(),
                "plaster",
                Vec3::new(0.0, 0.4, 0.0),
                Vec3::new(1.6, 0.8, 3.4),
            ),
            (
                gable(),
                "thatch",
                Vec3::new(0.0, 1.05, 0.0),
                Vec3::new(1.8, 0.5, 3.6),
            ),
        ],
        // Brewery: box with a big barrel beside it.
        "brewery" => vec![
            (
                cube(),
                "timber",
                Vec3::new(0.0, 0.55, 0.0),
                Vec3::new(1.6, 1.1, 1.8),
            ),
            (
                gable(),
                "brown_roof",
                Vec3::new(0.0, 1.45, 0.0),
                Vec3::new(1.8, 0.7, 2.0),
            ),
            (
                cyl(),
                "timber",
                Vec3::new(1.15, 0.45, 0.3),
                Vec3::new(0.7, 0.9, 0.7),
            ),
        ],
        // Mason's yard: low walls around stacked blocks.
        "masons_yard" => vec![
            (
                cube(),
                "stone",
                Vec3::new(0.0, 0.2, 0.9),
                Vec3::new(2.2, 0.4, 0.2),
            ),
            (
                cube(),
                "stone",
                Vec3::new(0.0, 0.2, -0.9),
                Vec3::new(2.2, 0.4, 0.2),
            ),
            (
                cube(),
                "stone",
                Vec3::new(-0.3, 0.3, 0.0),
                Vec3::new(0.6, 0.6, 0.6),
            ),
            (
                cube(),
                "stone",
                Vec3::new(0.4, 0.2, 0.2),
                Vec3::new(0.5, 0.4, 0.5),
            ),
        ],
        // Kiln: a brick dome.
        "kiln" => vec![
            (
                dome(),
                "brick",
                Vec3::new(0.0, 0.3, 0.0),
                Vec3::new(1.8, 1.8, 1.8),
            ),
            (
                cyl(),
                "brick",
                Vec3::new(0.0, 1.3, 0.0),
                Vec3::new(0.3, 0.6, 0.3),
            ),
        ],
        // Mint: solid stone, flat roof, a gold band.
        "mint" => vec![
            (
                cube(),
                "stone",
                Vec3::new(0.0, 0.7, 0.0),
                Vec3::new(1.8, 1.4, 1.8),
            ),
            (
                cube(),
                "gold",
                Vec3::new(0.0, 1.45, 0.0),
                Vec3::new(1.9, 0.12, 1.9),
            ),
        ],
        // Machine shop, roller mill: brick hall with a saw-tooth hint.
        "machine_shop" | "roller_mill" => vec![
            (
                cube(),
                "brick",
                Vec3::new(0.0, 0.7, 0.0),
                Vec3::new(2.2, 1.4, 2.8),
            ),
            (
                gable(),
                "slate_roof",
                Vec3::new(0.0, 1.7, 0.0),
                Vec3::new(2.4, 0.6, 3.0),
            ),
            (
                cyl(),
                "brick",
                Vec3::new(0.8, 2.2, 1.0),
                Vec3::new(0.35, 2.0, 0.35),
            ),
        ],
        // Anything else: a plain hall.
        _ => vec![
            (
                cube(),
                "plaster",
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(1.6, 1.0, 1.6),
            ),
            (
                gable(),
                "slate_roof",
                Vec3::new(0.0, 1.3, 0.0),
                Vec3::new(1.8, 0.6, 1.8),
            ),
        ],
    }
}

/// How a town builds: its era and its wealth tier (0 poor, 1, 2 rich).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub era: u8,
    pub tier: u8,
}

/// Wealth tier boundaries (residents' mean coin).
const TIERS: [f32; 2] = [6.0, 30.0];

impl Style {
    /// The style for a town, given its era and mean wealth. The tier only moves
    /// when wealth is clearly past a boundary (15 % margin), so a town hovering
    /// at a boundary doesn't rebuild every day.
    pub fn of(era: u8, mean_wealth: f32, current: Option<Style>) -> Style {
        let mut tier = TIERS.iter().filter(|&&b| mean_wealth >= b).count() as u8;
        if let Some(c) = current {
            if tier > c.tier && mean_wealth < TIERS[c.tier as usize] * 1.15 {
                tier = c.tier;
            }
            if tier < c.tier && mean_wealth > TIERS[c.tier as usize - 1] * 0.85 {
                tier = c.tier;
            }
        }
        Style { era, tier }
    }

    /// The wall colour of the era.
    pub fn wall(&self) -> &'static str {
        match self.era {
            0 => "timber",
            1 => "plaster",
            2 => "brick",
            _ => "stone",
        }
    }

    /// A roof colour of the era (`k` picks among the era's few).
    pub fn roof(&self, k: u32) -> &'static str {
        let pick = |opts: &[&'static str]| opts[k as usize % opts.len()];
        match self.era {
            0 => "thatch",
            1 => pick(&["red_roof", "brown_roof", "thatch"]),
            2 => pick(&["red_roof", "red_roof", "brown_roof"]),
            _ => pick(&["slate_roof", "slate_roof", "red_roof"]),
        }
    }

    /// A building part's colour in this style: walls and roofs follow the era;
    /// trade colours (soot, gold, sails, brick kilns) stay.
    pub fn restyle(&self, colour: &'static str, k: u32) -> &'static str {
        match colour {
            "plaster" => self.wall(),
            "red_roof" | "brown_roof" | "thatch" | "slate_roof" => self.roof(k),
            other => other,
        }
    }
}

/// Spawn one house at `at`, turned by `rot`, in the town's style.
pub fn spawn_house(
    commands: &mut Commands,
    kit: &Kit,
    outlines: &Outlines,
    at: Vec3,
    rot: Quat,
    k: u32,
    style: Style,
) -> Entity {
    // (body size, roof radius and height, chimney?)
    let (body, roof_r, roof_h, chimney) = match style.tier {
        0 => (Vec3::new(0.8, 0.6, 0.9), 0.8, 0.6, false),
        1 => (Vec3::new(1.0, 0.8, 1.2), 0.95, 0.7, false),
        _ => (Vec3::new(1.1, 1.5, 1.3), 1.0, 0.8, true),
    };
    let wall = if style.tier == 0 {
        "timber"
    } else {
        style.wall()
    };
    let roof = if style.tier == 0 {
        "thatch"
    } else {
        style.roof(k)
    };
    let mut parts = vec![
        (kit.cube.clone(), wall, Vec3::Y * body.y / 2.0, body),
        (
            kit.cone.clone(),
            roof,
            Vec3::Y * (body.y + roof_h / 2.0),
            Vec3::new(roof_r * 2.0, roof_h, roof_r * 2.0),
        ),
    ];
    if chimney {
        parts.push((
            kit.cube.clone(),
            "brick",
            Vec3::new(0.3, body.y + 0.35, 0.3),
            Vec3::new(0.2, 0.7, 0.2),
        ));
    }
    let parent = commands
        .spawn((
            Transform::from_translation(at).with_rotation(rot),
            Visibility::default(),
        ))
        .id();
    for (mesh, colour, offset, scale) in parts {
        commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(kit.mat(colour)),
                Transform::from_translation(offset).with_scale(scale),
                ChildOf(parent),
            ))
            .with_child(outlines.hull(mesh));
    }
    parent
}

/// Spawn one building: a parent entity at the ground point with its parts as
/// children, so the whole building can be removed again with one despawn.
/// Land "buildings" (cleared fields, woodlots) are not drawn: `None`.
pub fn spawn_building(
    commands: &mut Commands,
    kit: &Kit,
    outlines: &Outlines,
    b: &BuildingSpec,
    style: Style,
) -> Option<Entity> {
    if b.kind == "field_plot" || b.kind == "woodlot" {
        return None;
    }
    // Oriented a little differently each; workplaces are drawn 1.5 times the
    // size of a house, so they stand out.
    let rot = Quat::from_rotation_y((b.seed % 8) as f32 * std::f32::consts::FRAC_PI_4);
    let root = Transform::from_translation(b.at)
        .with_rotation(rot)
        .with_scale(Vec3::splat(1.5));
    let parent = commands.spawn((root, Visibility::default())).id();
    for (mesh, colour, offset, scale) in parts(kit, &b.kind) {
        // Each part carries its own outline hull (step 05) as a child.
        commands
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(kit.mat(style.restyle(colour, b.seed))),
                Transform::from_translation(offset).with_scale(scale),
                ChildOf(parent),
            ))
            .with_child(outlines.hull(mesh));
    }
    Some(parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wealth_tiers_move_only_clearly_past_a_boundary() {
        assert_eq!(Style::of(1, 2.0, None).tier, 0);
        assert_eq!(Style::of(1, 10.0, None).tier, 1);
        assert_eq!(Style::of(1, 50.0, None).tier, 2);
        let mid = Style::of(1, 10.0, None);
        // Just above the next boundary: stays (within the margin)...
        assert_eq!(Style::of(1, 31.0, Some(mid)).tier, 1);
        // ...clearly above: moves up.
        assert_eq!(Style::of(1, 40.0, Some(mid)).tier, 2);
        // Just below its own boundary: stays; clearly below: moves down.
        assert_eq!(Style::of(1, 5.5, Some(mid)).tier, 1);
        assert_eq!(Style::of(1, 4.0, Some(mid)).tier, 0);
    }

    #[test]
    fn eras_change_walls_and_roofs_but_not_trade_colours() {
        let old = Style { era: 0, tier: 1 };
        let new = Style { era: 3, tier: 1 };
        assert_eq!(old.restyle("plaster", 0), "timber");
        assert_eq!(new.restyle("plaster", 0), "stone");
        assert_eq!(old.restyle("red_roof", 0), "thatch");
        assert_eq!(new.restyle("thatch", 0), "slate_roof");
        assert_eq!(new.restyle("soot", 0), "soot");
        assert_eq!(new.restyle("gold", 0), "gold");
    }
}
