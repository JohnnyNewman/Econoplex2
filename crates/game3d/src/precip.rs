//! Shader step 11: rain and snow (docs/shaders/11-day-night-weather.md).
//!
//! One mesh of many small quads, one per drop or flake. The mesh never moves:
//! each quad's vertices carry a random point in a unit cube, and the vertex
//! shader (`assets/shaders/precip.wesl`) turns it into a position that falls
//! with time, drifts with the wind and wraps around inside a box in front of
//! the camera. The CPU only writes one small uniform: where the box is, how
//! fast things fall, and how much falls.
//!
//! Rain is drawn as thin streaks along the direction of fall, snow as soft
//! dots that flutter. How many of the quads show follows the precipitation.

use crate::env::Environment;
use crate::sky::SkyClock;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

const SHADER: &str = "shaders/precip.wesl";

/// Drops in the mesh (the most that can show at once).
const COUNT: usize = 14_000;

pub struct PrecipPlugin;

impl Plugin for PrecipPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PrecipMaterial>::default())
            .add_systems(Startup, spawn_precip)
            .add_systems(Update, update_precip);
    }
}

/// The uniform block the shader reads as `precip`.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct PrecipUniform {
    /// xyz: centre of the box the drops wrap around in; w: its edge length.
    pub centre: Vec4,
    /// xyz: velocity of the drops (world units per second); w: streak length
    /// in seconds of fall (0 for snow: round flakes).
    pub fall: Vec4,
    /// x: fraction of the drops that show (0..1), y: snow 0..1, z: drop width,
    /// w: brightness (daylight).
    pub look: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct PrecipMaterial {
    #[uniform(0)]
    pub precip: PrecipUniform,
}

impl Material for PrecipMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

#[derive(Component)]
struct Precip;

/// The quads: four corners each, all at the same random point (in the
/// position); the UV says which corner (x across -1..1, y along 0..1).
fn precip_mesh(n: usize) -> Mesh {
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut pos = Vec::with_capacity(n * 4);
    let mut uv = Vec::with_capacity(n * 4);
    let mut idx = Vec::with_capacity(n * 6);
    for i in 0..n {
        let p = [rand(), rand(), rand()];
        for c in [[-1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [-1.0, 1.0]] {
            pos.push(p);
            uv.push(c);
        }
        let b = (i * 4) as u32;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n * 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

fn spawn_precip(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PrecipMaterial>>,
) {
    commands.spawn((
        Precip,
        Mesh3d(meshes.add(precip_mesh(COUNT))),
        MeshMaterial3d(materials.add(PrecipMaterial::default())),
        // The shader places every drop itself: never cull the mesh, never
        // let it cast or catch shadows.
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
        Transform::default(),
        Visibility::Hidden,
    ));
}

/// The uniform for this frame's camera and weather.
pub fn precip_uniform(
    env: &Environment,
    daylight: f32,
    eye: Vec3,
    forward: Vec3,
    distance: f32,
) -> PrecipUniform {
    let w = &env.weather;
    let snow = w.snow_fraction();
    // The box sits in front of the camera, sized to what is in view: drops far
    // beyond it would be too small to see anyway.
    let size = (distance * 0.7).clamp(25.0, 90.0);
    let centre = eye + forward * size * 0.55;
    // Rain falls fast and slants with the wind; snow drifts.
    let wind = Vec3::new(w.wind.x, 0.0, w.wind.y);
    let speed = 22.0 - 19.0 * snow;
    let fall = Vec3::new(0.0, -speed, 0.0) + wind * (0.35 + 0.65 * snow);
    PrecipUniform {
        centre: centre.extend(size),
        // A raindrop is drawn as the path it covers in 1/30 s.
        fall: fall.extend(0.033 * (1.0 - snow)),
        look: Vec4::new(
            w.precip.clamp(0.0, 1.0),
            snow,
            (0.025 + 0.06 * snow) * size / 40.0,
            0.25 + 0.75 * daylight,
        ),
    }
}

fn update_precip(
    env: Res<Environment>,
    clock: Res<SkyClock>,
    camera: Query<(&GlobalTransform, Option<&crate::camera::WorldCamera>), With<Camera3d>>,
    mut precip: Query<(&MeshMaterial3d<PrecipMaterial>, &mut Visibility), With<Precip>>,
    mut materials: ResMut<Assets<PrecipMaterial>>,
) {
    let Some((cam, world_cam)) = camera.iter().next() else {
        return;
    };
    let distance = world_cam.map_or(60.0, |c| c.distance);
    let daylight = ((clock.sun().y + 0.05) / 0.3).clamp(0.0, 1.0) * (1.0 - 0.4 * env.weather.cloud);
    let u = precip_uniform(&env, daylight, cam.translation(), *cam.forward(), distance);
    for (handle, mut vis) in &mut precip {
        vis.set_if_neq(if u.look.x > 0.01 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        // Only touch the material when the uniform changes (the camera moves):
        // a changed material is sent to the GPU again.
        if materials.get(handle).is_some_and(|m| m.precip != u) {
            if let Some(mut m) = materials.get_mut(handle) {
                m.precip = u;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Weather;

    #[test]
    fn snow_falls_slower_than_rain() {
        let at = |temperature| {
            let env = Environment {
                weather: Weather {
                    temperature,
                    precip: 0.8,
                    ..default()
                },
                ..default()
            };
            precip_uniform(&env, 1.0, Vec3::ZERO, Vec3::NEG_Z, 60.0)
        };
        let (rain, snow) = (at(10.0), at(-8.0));
        assert!(rain.fall.y < snow.fall.y * 3.0);
        assert!(rain.look.y < 0.01 && snow.look.y > 0.99);
    }

    #[test]
    fn every_drop_owns_four_corners() {
        let mesh = precip_mesh(10);
        assert_eq!(mesh.count_vertices(), 40);
        let Some(Indices::U32(idx)) = mesh.indices() else {
            panic!("u32 indices")
        };
        for (q, tri) in idx.chunks(6).enumerate() {
            assert!(tri.iter().all(|&i| i as usize / 4 == q));
        }
    }
}
