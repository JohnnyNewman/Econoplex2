//! Roads and caravans.
//!
//! - **Roads:** the simulation's roads between towns (each town's road field,
//!   `Terrain::path`), drawn as a dirt ribbon on the ground. Grass and trees
//!   leave the road cells free (main.rs).
//! - **Caravans:** small carts drive along the roads from each town to the towns
//!   it sells to, more of them for more trade (the daily export value in the
//!   snapshot, as the trade overlay uses).

use crate::bake::Baked;
use crate::live::LiveSim;
use crate::toon::{toon, ToonMaterial};
use crate::{ground, SCALE};
use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use sim_core::terrain::Terrain;
use std::collections::{HashMap, HashSet};

pub struct RoadsPlugin;

impl Plugin for RoadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Carts>()
            .add_systems(Update, drive_caravans);
    }
}

/// Road width in 3D units, and how high above the ground it is drawn.
const WIDTH: f32 = 1.5;
const LIFT: f32 = 0.2;
/// Caravan speed in 3D units per second.
const SPEED: f32 = 5.0;

/// The road network: which terrain cells roads cross, and the road from each
/// town to each other town as a line of 3D points.
#[derive(Resource, Default)]
pub struct Roads {
    pub cells: HashSet<usize>,
    pub paths: HashMap<(usize, usize), Path>,
}

pub struct Path {
    pub points: Vec<Vec3>,
    /// Running length at each point.
    pub along: Vec<f32>,
}

impl Path {
    fn new(points: Vec<Vec3>) -> Self {
        let mut along = vec![0.0];
        for w in points.windows(2) {
            along.push(along.last().unwrap() + w[0].distance(w[1]));
        }
        Path { points, along }
    }

    pub fn length(&self) -> f32 {
        *self.along.last().unwrap_or(&0.0)
    }

    /// The point a distance `d` along the path, and the direction there.
    pub fn at(&self, d: f32) -> (Vec3, Vec3) {
        let d = d.clamp(0.0, self.length());
        let k = self
            .along
            .partition_point(|&a| a < d)
            .clamp(1, self.points.len().max(2) - 1);
        let (a, b) = (self.points[k - 1], self.points[k]);
        let span = (self.along[k] - self.along[k - 1]).max(1e-6);
        let f = ((d - self.along[k - 1]) / span).clamp(0.0, 1.0);
        (a.lerp(b, f), (b - a).normalize_or(Vec3::Z))
    }
}

/// One round of corner cutting (Chaikin), so roads bend instead of zig-zagging
/// from cell to cell. Keeps the end points.
fn smooth(points: &[Vec3]) -> Vec<Vec3> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for w in points.windows(2) {
        out.push(w[0].lerp(w[1], 0.25));
        out.push(w[0].lerp(w[1], 0.75));
    }
    out.push(*points.last().unwrap());
    out
}

/// The roads between every pair of towns that a road connects.
pub fn network(t: &Terrain, towns: &[(f32, f32)]) -> Roads {
    let mut roads = Roads::default();
    let to3 = |p: (f32, f32)| Vec3::new(p.0 / SCALE, ground(t, p) + LIFT, -p.1 / SCALE);
    for (a, &from) in towns.iter().enumerate() {
        for (b, &to) in towns.iter().enumerate() {
            if a == b || !t.road(a, b).is_finite() {
                continue;
            }
            let cells = t.path(from, b);
            for p in &cells {
                if let Some(c) = t.cell_at(*p) {
                    roads.cells.insert(c);
                }
            }
            let mut points: Vec<Vec3> = cells.iter().map(|&p| to3(p)).collect();
            points.push(to3(to));
            let mut points = smooth(&smooth(&points));
            // Back on the ground after smoothing.
            for p in &mut points {
                let q = (p.x * SCALE, -p.z * SCALE);
                p.y = ground(t, q) + LIFT;
            }
            roads.paths.insert((a, b), Path::new(points));
        }
    }
    roads
}

/// The road ribbons as one mesh.
pub fn road_mesh(t: &Terrain, roads: &Roads) -> Mesh {
    let (mut pos, mut nrm, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    // Both directions: the road from a to b and the one back can take
    // different routes (each follows its own town's road field).
    for path in roads.paths.values() {
        let pts = &path.points;
        for (k, &p) in pts.iter().enumerate() {
            let next = pts[(k + 1).min(pts.len() - 1)];
            let prev = pts[k.saturating_sub(1)];
            let dir = (next - prev).with_y(0.0).normalize_or(Vec3::Z);
            let side = dir.cross(Vec3::Y) * WIDTH * 0.5;
            for s in [p - side, p + side] {
                let y = ground(t, (s.x * SCALE, -s.z * SCALE)) + LIFT;
                pos.push([s.x, y, s.z]);
                nrm.push([0.0, 1.0, 0.0]);
            }
            if k + 1 < pts.len() {
                let i = (pos.len() - 2) as u32;
                idx.extend([i, i + 1, i + 2, i + 1, i + 3, i + 2]);
            }
        }
    }
    let n = pos.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n]);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// Spawn the road ribbons; returns the network (caravans drive on it, grass and
/// trees keep off its cells).
pub fn spawn_roads(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    toons: &mut Assets<ToonMaterial>,
    t: &Terrain,
    towns: &[(f32, f32)],
) -> HashSet<usize> {
    let roads = network(t, towns);
    commands.spawn((
        Mesh3d(meshes.add(road_mesh(t, &roads))),
        MeshMaterial3d(toons.add(toon(Color::srgb(0.40, 0.31, 0.22)))),
        NotShadowCaster,
    ));
    let cells = roads.cells.clone();
    commands.insert_resource(roads);
    cells
}

/// A cart: a box on two wheels with its load, pulled by an ox (baked, vertex
/// colours), facing +z.
fn cart_mesh() -> Mesh {
    let lin = |c: Color| c.to_linear().to_f32_array();
    let (wood, load, wheel, ox) = (
        lin(Color::srgb(0.45, 0.30, 0.18)),
        lin(Color::srgb(0.85, 0.76, 0.55)),
        lin(Color::srgb(0.22, 0.18, 0.15)),
        lin(Color::srgb(0.38, 0.26, 0.20)),
    );
    let mut b = Baked::default();
    let cube = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
    let wheel_mesh = Mesh::from(Cylinder::new(0.22, 0.1));
    b.add(
        &cube,
        Transform::from_xyz(0.0, 0.35, 0.0).with_scale(Vec3::new(0.7, 0.25, 1.0)),
        wood,
        None,
    );
    b.add(
        &cube,
        Transform::from_xyz(0.0, 0.62, -0.05).with_scale(Vec3::new(0.6, 0.3, 0.7)),
        load,
        None,
    );
    for x in [-0.4, 0.4] {
        b.add(
            &wheel_mesh,
            Transform::from_xyz(x, 0.22, -0.1)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
            wheel,
            None,
        );
    }
    b.add(
        &cube,
        Transform::from_xyz(0.0, 0.35, 0.95).with_scale(Vec3::new(0.32, 0.4, 0.7)),
        ox,
        None,
    );
    b.mesh()
}

/// The carts on the road, by (seller, buyer, number).
#[derive(Resource, Default)]
struct Carts {
    shown: HashMap<(usize, usize, usize), Entity>,
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<ToonMaterial>>,
}

/// Carts drive from each town to the towns it sells to: 1 to 4 per route, by
/// how much it ships, evenly spaced and looping.
#[allow(clippy::too_many_arguments)]
fn drive_caravans(
    mut commands: Commands,
    live: Res<LiveSim>,
    roads: Option<Res<Roads>>,
    time: Res<Time>,
    mut carts: ResMut<Carts>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut toons: ResMut<Assets<ToonMaterial>>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(roads) = roads else { return };
    let mesh = carts
        .mesh
        .get_or_insert_with(|| meshes.add(cart_mesh()))
        .clone();
    let material = carts
        .material
        .get_or_insert_with(|| toons.add(toon(Color::WHITE)))
        .clone();
    let towns = &live.snapshot.towns;
    let max = towns
        .iter()
        .flat_map(|t| t.exports.iter().copied())
        .fold(1.0f32, f32::max);
    let mut wanted = HashSet::new();
    for (a, from) in towns.iter().enumerate() {
        for (b, &value) in from.exports.iter().enumerate() {
            if a == b || value < 0.5 {
                continue;
            }
            let Some(path) = roads.paths.get(&(a, b)) else {
                continue;
            };
            let len = path.length();
            if len < 1.0 {
                continue;
            }
            let n = 1 + ((value / max).sqrt() * 3.0).round() as usize;
            for k in 0..n {
                let key = (a, b, k);
                wanted.insert(key);
                let d = ((time.elapsed_secs() * SPEED / len + k as f32 / n as f32).fract()) * len;
                let (p, dir) = path.at(d);
                // Keep to the right of the road, so two-way traffic passes.
                let side = dir.with_y(0.0).normalize_or(Vec3::Z).cross(Vec3::Y) * -0.3;
                let tf = Transform::from_translation(p + side - Vec3::Y * LIFT * 0.5)
                    .looking_to(-dir.with_y(0.0).normalize_or(Vec3::Z), Vec3::Y)
                    .with_scale(Vec3::splat(0.8));
                match carts.shown.get(&key) {
                    Some(&e) => {
                        if let Ok(mut t) = transforms.get_mut(e) {
                            *t = tf;
                        }
                    }
                    None => {
                        let e = commands
                            .spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), tf))
                            .id();
                        carts.shown.insert(key, e);
                    }
                }
            }
        }
    }
    carts.shown.retain(|key, e| {
        let keep = wanted.contains(key);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_walks_its_points_by_distance() {
        let p = Path::new(vec![
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 3.0),
            Vec3::new(4.0, 0.0, 3.0),
        ]);
        assert_eq!(p.length(), 7.0);
        let (mid, dir) = p.at(1.5);
        assert!((mid - Vec3::new(0.0, 0.0, 1.5)).length() < 1e-5 && dir == Vec3::Z);
        let (q, dir) = p.at(5.0);
        assert!((q - Vec3::new(2.0, 0.0, 3.0)).length() < 1e-5 && dir == Vec3::X);
        // Past the end: the end.
        assert!((p.at(99.0).0 - Vec3::new(4.0, 0.0, 3.0)).length() < 1e-5);
    }

    #[test]
    fn smoothing_keeps_the_ends_and_cuts_the_corners() {
        let pts = vec![
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 1.0),
        ];
        let s = smooth(&pts);
        assert_eq!(s.first(), pts.first());
        assert_eq!(s.last(), pts.last());
        assert!(!s.contains(&Vec3::new(1.0, 0.0, 0.0)), "the corner is cut");
    }
}
