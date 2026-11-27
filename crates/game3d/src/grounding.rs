//! Grounding: soft dark contact shadows on the ground around houses,
//! buildings and trees (and darker wall feet and eaves, baked into vertex
//! colours: `bake::occlusion`).
//!
//! Sun shadows fall to one side; under an overcast sky, at dusk, or on the
//! sunny side, things looked pasted onto the ground. Here a greyscale map over
//! the whole terrain, the *contact map*, holds how much each spot of ground
//! is hemmed in by something standing on it: 1 under a footprint, fading to
//! 0 a little beyond it. The terrain shader darkens the ground by it.
//!
//! The map is drawn on the CPU (a soft disc per footprint) at start and again
//! whenever a town's houses or buildings change, and only its pixels are
//! uploaded: no material changes.
//!
//! Beside it, the *wall map*: how far each spot is from the nearest wall of a
//! workplace or house (their footprints as the town's layout has them,
//! `sim_core::layout`), signed, negative inside. The crowd's walk
//! (walk.wesl) reads it to go round buildings and never into one.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

/// Pixels across the contact map (about 0.2 3D units each on today's maps).
pub const SIZE: usize = 1024;

/// How far a footprint's shade reaches, in footprint radii.
pub const REACH: f32 = 2.4;

/// The distances the wall map holds, 3D units, either side of a wall: it
/// stores `0.5 + d / (2 WALL_RANGE)`, clamped (walk.wesl reads it back).
pub const WALL_RANGE: f32 = 1.0;

pub struct GroundingPlugin;

impl Plugin for GroundingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, draw_contact_map);
    }
}

/// What stands on the ground: centre (3D x, z) and radius of each footprint.
#[derive(Resource, Default)]
pub struct Footprints {
    pub trees: Vec<(Vec2, f32)>,
    /// Per town, its houses.
    pub houses: Vec<Vec<(Vec2, f32)>>,
    /// Buildings by their simulation id.
    pub buildings: HashMap<u64, (Vec2, f32)>,
    /// Per town, the footprints of its workplaces and houses (map units),
    /// for the wall map.
    pub walls: Vec<Vec<sim_core::layout::Footprint>>,
    /// Something changed since the map was drawn.
    pub dirty: bool,
}

/// The contact map and where it lies: x, z of its first corner, and its size
/// in 3D units along x and z.
#[derive(Resource, Clone)]
pub struct ContactMap {
    pub image: Handle<Image>,
    pub rect: Vec4,
    /// The wall map, over the same rectangle.
    pub walls: Handle<Image>,
}

/// An empty contact map (filled by `draw_contact_map`). The main world keeps
/// its pixels so they can be redrawn.
pub fn contact_image() -> Image {
    Image::new(
        Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; SIZE * SIZE],
        TextureFormat::R8Unorm,
        RenderAssetUsages::default(),
    )
}

/// An open wall map (filled by `draw_contact_map`): far from every wall.
pub fn wall_image() -> Image {
    let mut image = contact_image();
    image.data = Some(vec![255u8; SIZE * SIZE]);
    image
}

/// Signed distance from `p` to the footprint `f`, both in map units:
/// negative inside.
pub fn wall_distance(f: &sim_core::layout::Footprint, p: Vec2) -> f32 {
    let d = p - Vec2::new(f.centre.0, f.centre.1);
    let axis = Vec2::new(f.axis.0, f.axis.1);
    let local = Vec2::new(d.dot(axis), axis.perp_dot(d));
    let q = local.abs() - Vec2::new(f.half.0, f.half.1);
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0)
}

/// Draw the walls into a fresh wall map, `SIZE` × `SIZE` over `rect`, row 0
/// at its smallest z.
pub fn draw_walls(walls: &[Vec<sim_core::layout::Footprint>], rect: Vec4) -> Vec<u8> {
    let mut px = vec![255u8; SIZE * SIZE];
    let scale = Vec2::new(SIZE as f32 / rect.z, SIZE as f32 / rect.w);
    for f in walls.iter().flatten() {
        let c = Vec2::new(f.centre.0, -f.centre.1) / crate::SCALE;
        let reach = f.half.0.hypot(f.half.1) / crate::SCALE + WALL_RANGE;
        let lo = ((c - reach - rect.xy()) * scale).floor().max(Vec2::ZERO);
        let hi = ((c + reach - rect.xy()) * scale)
            .ceil()
            .min(Vec2::splat(SIZE as f32 - 1.0));
        if lo.x > hi.x || lo.y > hi.y {
            continue;
        }
        for v in lo.y as usize..=hi.y as usize {
            for u in lo.x as usize..=hi.x as usize {
                let p = rect.xy() + (Vec2::new(u as f32, v as f32) + 0.5) / scale;
                let d = wall_distance(f, Vec2::new(p.x, -p.y) * crate::SCALE) / crate::SCALE;
                let k = ((0.5 + d / (2.0 * WALL_RANGE)).clamp(0.0, 1.0) * 255.0).round() as u8;
                let cell = &mut px[v * SIZE + u];
                *cell = (*cell).min(k);
            }
        }
    }
    px
}

/// How dark the ground is at distance `d` from the centre of a footprint of
/// radius `r`: full under it, fading out smoothly to 0 at `REACH` r.
pub fn contact(d: f32, r: f32) -> f32 {
    let outer = r * REACH;
    if d <= r {
        1.0
    } else if d >= outer {
        0.0
    } else {
        let t = 1.0 - (d - r) / (outer - r);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Draw the footprints into a fresh map, `SIZE` × `SIZE`, row 0 at the
/// rectangle's smallest z.
pub fn draw(footprints: &Footprints, rect: Vec4) -> Vec<u8> {
    let mut px = vec![0u8; SIZE * SIZE];
    let scale = Vec2::new(SIZE as f32 / rect.z, SIZE as f32 / rect.w);
    let all = footprints
        .trees
        .iter()
        .chain(footprints.houses.iter().flatten())
        .chain(footprints.buildings.values());
    for &(c, r) in all {
        let outer = r * REACH;
        let lo = ((c - outer - rect.xy()) * scale).floor().max(Vec2::ZERO);
        let hi = ((c + outer - rect.xy()) * scale)
            .ceil()
            .min(Vec2::splat(SIZE as f32 - 1.0));
        if lo.x > hi.x || lo.y > hi.y {
            continue;
        }
        for v in lo.y as usize..=hi.y as usize {
            for u in lo.x as usize..=hi.x as usize {
                let p = rect.xy() + (Vec2::new(u as f32, v as f32) + 0.5) / scale;
                let k = (contact(p.distance(c), r) * 255.0).round() as u8;
                let cell = &mut px[v * SIZE + u];
                *cell = (*cell).max(k);
            }
        }
    }
    px
}

/// Redraw the map when what stands on the ground changed.
fn draw_contact_map(
    footprints: Option<ResMut<Footprints>>,
    map: Option<Res<ContactMap>>,
    mut images: ResMut<Assets<Image>>,
    terrain: Query<&MeshMaterial3d<crate::terrain::TerrainMaterial>>,
    mut terrains: ResMut<Assets<crate::terrain::TerrainMaterial>>,
) {
    let (Some(mut footprints), Some(map)) = (footprints, map) else {
        return;
    };
    if !footprints.dirty {
        return;
    }
    footprints.dirty = false;
    let px = draw(&footprints, map.rect);
    if let Some(mut image) = images.get_mut(&map.image) {
        image.data = Some(px);
    }
    let px = draw_walls(&footprints.walls, map.rect);
    if let Some(mut image) = images.get_mut(&map.walls) {
        image.data = Some(px);
    }
    // A new texture on the GPU needs a new bind group: touch the terrain's
    // material so Bevy prepares it again (one material, only when houses or
    // buildings changed, about once a game day).
    for m in &terrain {
        let _ = terrains.get_mut(&m.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_is_full_underneath_and_fades_out() {
        assert_eq!(contact(0.0, 1.0), 1.0);
        assert_eq!(contact(1.0, 1.0), 1.0);
        assert!(contact(1.4, 1.0) > 0.0 && contact(1.4, 1.0) < 1.0);
        assert_eq!(contact(REACH, 1.0), 0.0);
    }

    #[test]
    fn a_footprint_darkens_its_own_pixels_only() {
        let mut f = Footprints::default();
        f.trees.push((Vec2::new(50.0, 50.0), 2.0));
        // A 100 x 100 rectangle from (0, 0): about 10 pixels per unit.
        let px = draw(&f, Vec4::new(0.0, 0.0, 100.0, 100.0));
        let at = |x: f32, z: f32| px[(z * 10.24) as usize * SIZE + (x * 10.24) as usize];
        assert_eq!(at(50.0, 50.0), 255);
        assert_eq!(at(10.0, 10.0), 0);
        assert!(at(53.0, 50.0) > 0 && at(53.0, 50.0) < 255);
    }

    /// The wall map holds the distance to a footprint's wall: below half
    /// inside it, half on the wall, full far from it.
    #[test]
    fn the_wall_map_measures_from_the_walls() {
        // A house at 3D (50, -50), turned a quarter: map (500, 500).
        let f = sim_core::layout::Footprint {
            id: 1,
            centre: (500.0, 500.0),
            half: (10.0, 5.0),
            axis: (0.0, 1.0),
        };
        assert!((wall_distance(&f, Vec2::new(500.0, 500.0)) + 5.0).abs() < 1e-4);
        assert!((wall_distance(&f, Vec2::new(508.0, 500.0)) - 3.0).abs() < 1e-4);
        assert!((wall_distance(&f, Vec2::new(500.0, 512.0)) - 2.0).abs() < 1e-4);
        let px = draw_walls(&[vec![f]], Vec4::new(0.0, -100.0, 100.0, 100.0));
        let at = |x: f32, z: f32| {
            px[((z + 100.0) * 10.24) as usize * SIZE + (x * 10.24) as usize] as f32 / 255.0
        };
        assert!(at(50.0, -50.0) < 0.3);
        assert!((at(50.0, -51.0) - 0.5).abs() < 0.06);
        assert_eq!(at(20.0, -20.0), 1.0);
    }
}
