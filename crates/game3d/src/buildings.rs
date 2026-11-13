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

use crate::bake::Baked;
use crate::lights::{LampKind, Lamps};
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
    /// The shapes themselves, to bake buildings from.
    shapes: HashMap<AssetId<Mesh>, Mesh>,
    /// The palette as linear colours (vertex colours of baked buildings).
    linear: HashMap<&'static str, [f32; 4]>,
    /// One white toon material for all baked buildings.
    white: Handle<ToonMaterial>,
    /// Baked variants, shared by every building that looks the same.
    baked: HashMap<String, Handle<Mesh>>,
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
        let palette: [(&'static str, Color); 15] = [
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
            // Step 12: parts that light up at night (lights.wesl). The colour
            // is what they show by day; the alpha below marks the kind.
            ("window", Color::srgb(0.16, 0.15, 0.16)),
            ("lantern", Color::srgb(0.30, 0.26, 0.18)),
            ("fire", Color::srgb(0.30, 0.12, 0.06)),
        ];
        let shapes: [Mesh; 5] = [
            Cuboid::new(1.0, 1.0, 1.0).into(),
            gable.into(),
            Cone::new(0.5, 1.0).into(),
            Cylinder::new(0.5, 1.0).into(),
            Sphere::new(0.5).mesh().uv(24, 12),
        ];
        let handles: Vec<Handle<Mesh>> = shapes.iter().map(|m| meshes.add(m.clone())).collect();
        Self {
            cube: handles[0].clone(),
            gable: handles[1].clone(),
            cone: handles[2].clone(),
            cylinder: handles[3].clone(),
            dome: handles[4].clone(),
            shapes: handles.iter().map(|h| h.id()).zip(shapes).collect(),
            linear: palette
                .iter()
                .map(|&(name, c)| {
                    let mut rgba = c.to_linear().to_f32_array();
                    rgba[3] = glow_marker(name);
                    (name, rgba)
                })
                .collect(),
            white: toons.add(toon(Color::WHITE)),
            baked: HashMap::new(),
        }
    }

    /// One mesh for these parts (already in their final colours), baked once
    /// per distinct look and shared after.
    fn bake(&mut self, meshes: &mut Assets<Mesh>, parts: &[Part]) -> Handle<Mesh> {
        let key = parts
            .iter()
            .map(|(m, c, o, s)| format!("{:?}{c}{o}{s};", m.id()))
            .collect::<String>();
        if let Some(h) = self.baked.get(&key) {
            return h.clone();
        }
        let mut baked = Baked::default();
        for (mesh, colour, offset, scale) in parts {
            baked.add(
                &self.shapes[&mesh.id()],
                Transform::from_translation(*offset).with_scale(*scale),
                self.linear[colour],
                None,
            );
        }
        let h = meshes.add(baked.mesh());
        self.baked.insert(key, h.clone());
        h
    }

    /// Spawn a baked building: the body and its outline hull (step 05).
    fn spawn_baked(
        &mut self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        outlines: &Outlines,
        at: Transform,
        parts: &[Part],
    ) -> Entity {
        let mesh = self.bake(meshes, parts);
        // Step 12: lanterns and fires light their surroundings at night.
        let lamps: Vec<(Vec3, LampKind)> = parts
            .iter()
            .filter_map(|(_, c, o, _)| match *c {
                "lantern" => Some((*o, LampKind::Lantern)),
                // In front of the fire's mouth, not inside the wall.
                "fire" => Some((*o + Vec3::Z * 0.3, LampKind::Fire)),
                _ => None,
            })
            .collect();
        let mut e = commands.spawn((at, Mesh3d(mesh.clone()), MeshMaterial3d(self.white.clone())));
        e.with_child(outlines.hull(mesh));
        if !lamps.is_empty() {
            e.insert(Lamps(lamps));
        }
        e.id()
    }
}

/// A building part: (shape, colour, offset, scale).
type Part = (Handle<Mesh>, &'static str, Vec3, Vec3);

/// The vertex alpha that tells the toon shader what lights up at night
/// (lights.wesl): 1 for ordinary surfaces.
fn glow_marker(colour: &str) -> f32 {
    match colour {
        "window" => 0.25,
        "lantern" => 0.5,
        "fire" => 0.75,
        _ => 1.0,
    }
}

/// Windows on the two long sides of a box of size `body` standing on the
/// ground: `rows` storeys, two per side.
fn windows(kit: &Kit, body: Vec3, rows: u32) -> Vec<Part> {
    let mut out = Vec::new();
    let pane = Vec3::new(0.04, 0.2, 0.17);
    for row in 0..rows {
        let y = body.y * (row as f32 + 0.55) / rows as f32;
        for side in [-1.0, 1.0] {
            for along in [-0.22, 0.22] {
                out.push((
                    kit.cube.clone(),
                    "window",
                    Vec3::new(side * (body.x / 2.0 + 0.01), y, along * body.z),
                    pane,
                ));
            }
        }
    }
    out
}

/// A lantern on a post beside a house of size `body`.
fn lantern(kit: &Kit, body: Vec3) -> Vec<Part> {
    let at = Vec3::new(body.x / 2.0 + 0.22, 0.0, body.z / 2.0 + 0.1);
    vec![
        (
            kit.cube.clone(),
            "timber",
            at + Vec3::Y * 0.36,
            Vec3::new(0.05, 0.72, 0.05),
        ),
        (
            kit.cube.clone(),
            "lantern",
            at + Vec3::Y * 0.78,
            Vec3::new(0.12, 0.14, 0.12),
        ),
    ]
}

/// The parts of one building: (mesh, colour, offset from the ground point,
/// scale), in the building's own frame (x across, z along, y up).
fn parts(kit: &Kit, kind: &str) -> Vec<Part> {
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
            // The oven's mouth on the front.
            (
                cube(),
                "fire",
                Vec3::new(0.3, 0.25, 0.91),
                Vec3::new(0.35, 0.22, 0.04),
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
            // The furnace mouth.
            (
                cube(),
                "fire",
                Vec3::new(-0.3, 0.3, 0.91),
                Vec3::new(0.45, 0.35, 0.04),
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
            // The forge, seen through the open front.
            (
                cube(),
                "fire",
                Vec3::new(-0.4, 0.3, 0.91),
                Vec3::new(0.5, 0.3, 0.04),
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
#[allow(clippy::too_many_arguments)]
pub fn spawn_house(
    commands: &mut Commands,
    kit: &mut Kit,
    meshes: &mut Assets<Mesh>,
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
    // Step 12: windows (two storeys for the big houses), and a lantern by
    // every third door.
    parts.extend(windows(kit, body, if style.tier >= 2 { 2 } else { 1 }));
    if k.is_multiple_of(3) {
        parts.extend(lantern(kit, body));
    }
    if chimney {
        parts.push((
            kit.cube.clone(),
            "brick",
            Vec3::new(0.3, body.y + 0.35, 0.3),
            Vec3::new(0.2, 0.7, 0.2),
        ));
    }
    kit.spawn_baked(
        commands,
        meshes,
        outlines,
        Transform::from_translation(at).with_rotation(rot),
        &parts,
    )
}

/// Spawn one building at its ground point, its parts baked into one mesh (the
/// outline hull is its child, so one despawn removes both).
/// Land "buildings" (cleared fields, woodlots) are not drawn: `None`.
pub fn spawn_building(
    commands: &mut Commands,
    kit: &mut Kit,
    meshes: &mut Assets<Mesh>,
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
    let mut parts: Vec<Part> = parts(kit, &b.kind)
        .into_iter()
        .map(|(m, c, o, s)| (m, style.restyle(c, b.seed), o, s))
        .collect();
    // Step 12: windows on a box-shaped main body standing on the ground.
    if let Some((m, _, o, s)) = parts.first() {
        if m.id() == kit.cube.id() && o.x == 0.0 && o.z == 0.0 {
            let body = *s;
            parts.extend(windows(kit, body, 1));
        }
    }
    Some(kit.spawn_baked(commands, meshes, outlines, root, &parts))
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
