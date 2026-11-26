//! The ground under the towns: hills, rivers and the roads between towns,
//! drawn from the simulation's terrain grid, one pixel per cell.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sim_core::terrain::Terrain;
use sim_core::Towns;
use std::collections::HashSet;

/// The cells a road crosses: the cheapest path from each town to every other
/// town it has a road to (`Terrain::path`, as the 3D client's roads).
pub fn road_cells(terrain: &Terrain, towns: &[(f32, f32)]) -> HashSet<usize> {
    let mut cells = HashSet::new();
    for (a, &from) in towns.iter().enumerate() {
        for b in 0..towns.len() {
            if a == b || !terrain.road(a, b).is_finite() {
                continue;
            }
            for p in terrain.path(from, b) {
                if let Some(c) = terrain.cell_at(p) {
                    cells.insert(c);
                }
            }
        }
    }
    cells
}

/// What a terrain pixel shows, for the season's tint (seasons.rs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
    Ground,
    Road,
    Water,
    Ford,
}

/// The terrain image and what each pixel was drawn from, so the seasons can
/// recolour it: the base colour, what the cell is, and its nearest town (whose
/// weather it takes). In image order: rows top to bottom.
#[derive(Resource)]
pub struct TerrainLook {
    pub image: Handle<Image>,
    pub base: Vec<[u8; 3]>,
    pub cell: Vec<Cell>,
    pub town: Vec<u16>,
}

pub fn spawn_terrain(
    mut commands: Commands,
    terrain: Res<Terrain>,
    towns: Res<Towns>,
    mut images: ResMut<Assets<Image>>,
) {
    let (w, h) = (terrain.width, terrain.height);
    let pos: Vec<(f32, f32)> = towns.0.iter().map(|t| t.pos).collect();
    let roads = road_cells(&terrain, &pos);
    let mut data = Vec::with_capacity(w * h * 4);
    let (mut base, mut cells, mut near) = (Vec::new(), Vec::new(), Vec::new());
    // Image rows run top to bottom, the grid's bottom to top.
    for j in (0..h).rev() {
        for i in 0..w {
            let c = j * w + i;
            let (rgb, kind) = if terrain.water[c] && terrain.cost[c].is_finite() {
                ([70, 92, 96], Cell::Ford)
            } else if terrain.water[c] {
                ([32, 62, 104], Cell::Water)
            } else if roads.contains(&c) {
                ([96, 78, 52], Cell::Road)
            } else {
                // Hills are lighter and browner the harder they are to cross.
                let hill = ((terrain.cost[c] - 1.0) / 1.5).clamp(0.0, 1.0);
                let lerp = |a: f32, b: f32| (a + (b - a) * hill) as u8;
                (
                    [lerp(22.0, 70.0), lerp(28.0, 60.0), lerp(21.0, 38.0)],
                    Cell::Ground,
                )
            };
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            base.push(rgb);
            cells.push(kind);
            let at = terrain.center(c);
            let town = pos
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    let d = |p: &(f32, f32)| (p.0 - at.0).powi(2) + (p.1 - at.1).powi(2);
                    d(a.1).total_cmp(&d(b.1))
                })
                .map_or(0, |(k, _)| k as u16);
            near.push(town);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    let size = Vec2::new(w as f32, h as f32) * terrain.cell;
    let corner = Vec2::new(terrain.origin.0, terrain.origin.1);
    let center = corner + size / 2.0;
    let handle = images.add(image);
    commands.insert_resource(TerrainLook {
        image: handle.clone(),
        base,
        cell: cells,
        town: near,
    });
    commands.spawn((
        Sprite {
            image: handle,
            custom_size: Some(size),
            ..default()
        },
        Transform::from_xyz(center.x, center.y, -1.0),
        crate::view::WorldView,
    ));
}

/// Where a walker at `pos` heading for `target` steps next. Near a town the walker
/// follows that town's flow field, so crowds go around the river and through the
/// fords; one shared field serves everyone heading to the same town.
pub fn walk(terrain: &Terrain, towns: &[Vec2], pos: Vec2, target: Vec2, step: f32) -> Vec2 {
    let delta = target - pos;
    if delta.length() <= step {
        return target;
    }
    let straight = pos + delta.normalize() * step;
    // Short hops inside a town, or open ground ahead: walk straight.
    if delta.length() < 120.0 {
        return straight;
    }
    let goal = towns
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.distance(target).total_cmp(&b.1.distance(target)))
        .filter(|(_, t)| t.distance(target) < 260.0)
        .map(|(k, _)| k);
    let Some(town) = goal else {
        // A target out in the field (a squad on the march): straight, unless that
        // would wade into the river.
        return if terrain.passable((straight.x, straight.y)) {
            straight
        } else {
            pos
        };
    };
    match terrain.step_toward(town, (pos.x, pos.y)) {
        Some(d) => pos + Vec2::new(d.0, d.1) * step,
        None => straight,
    }
}
