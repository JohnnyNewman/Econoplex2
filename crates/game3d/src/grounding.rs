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

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

/// Pixels across the contact map (about 0.2 3D units each on today's maps).
pub const SIZE: usize = 1024;

/// How far a footprint's shade reaches, in footprint radii.
pub const REACH: f32 = 2.4;

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
    /// Something changed since the map was drawn.
    pub dirty: bool,
}

/// The contact map and where it lies: x, z of its first corner, and its size
/// in 3D units along x and z.
#[derive(Resource, Clone)]
pub struct ContactMap {
    pub image: Handle<Image>,
    pub rect: Vec4,
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
}
