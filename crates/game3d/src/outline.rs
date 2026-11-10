//! Shader step 05: outlines by the "inverted hull" (docs/shaders/05-rim-outline.md).
//!
//! An outlined mesh gets a child entity with the same mesh and the
//! [`OutlineMaterial`]: a second, slightly fatter copy drawn inside out (front
//! faces culled), so only a thin dark band shows around the silhouette.
//!
//! Key: `O` turns outlines on and off (off = width 0, so the hull hides inside
//! the mesh).

use crate::fog::{FogState, FogUniform};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

const SHADER: &str = "shaders/outline.wesl";

/// Outline width (world units at the reference distance) and that distance.
const WIDTH: f32 = 0.06;
const REFERENCE_DISTANCE: f32 = 40.0;

pub struct OutlinePlugin;

impl Plugin for OutlinePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<OutlineMaterial>::default())
            .add_systems(Startup, init_outline)
            .add_systems(Update, update_outline);
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, Reflect)]
pub struct OutlineUniform {
    /// The line colour (linear RGB).
    pub color: Vec4,
    /// x: width, y: reference distance; zw unused.
    pub params: Vec4,
    /// The same fog as everything else (step 04).
    pub fog: FogUniform,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct OutlineMaterial {
    #[uniform(0)]
    pub outline: OutlineUniform,
}

impl Material for OutlineMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    /// The hull is only a look: no depth prepass, no shadows.
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    /// Draw the hull inside out: cull the faces that point at the camera.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}

/// The one outline material all outlined meshes share, and whether it is on.
#[derive(Resource)]
pub struct Outlines {
    pub material: Handle<OutlineMaterial>,
    pub on: bool,
}

impl Outlines {
    /// The child entity that outlines `mesh` (attach it under the mesh's entity).
    pub fn hull(&self, mesh: Handle<Mesh>) -> impl Bundle {
        (
            Mesh3d(mesh),
            MeshMaterial3d(self.material.clone()),
            Transform::default(),
            NotShadowCaster,
            NotShadowReceiver,
        )
    }
}

/// Start with outlines off (`--no-outlines`).
#[derive(Resource)]
pub struct OutlinesOff;

pub fn init_outline(
    mut commands: Commands,
    mut materials: ResMut<Assets<OutlineMaterial>>,
    off: Option<Res<OutlinesOff>>,
) {
    let material = materials.add(OutlineMaterial {
        outline: OutlineUniform {
            color: Vec4::new(0.08, 0.07, 0.09, 1.0),
            params: Vec4::new(WIDTH, REFERENCE_DISTANCE, 0.0, 0.0),
            fog: FogUniform::default(),
        },
    });
    commands.insert_resource(Outlines {
        material,
        on: off.is_none(),
    });
}

fn update_outline(
    keys: Res<ButtonInput<KeyCode>>,
    fog: Res<FogState>,
    outlines: Option<ResMut<Outlines>>,
    mut materials: ResMut<Assets<OutlineMaterial>>,
) {
    let Some(mut outlines) = outlines else { return };
    if keys.just_pressed(KeyCode::KeyO) {
        outlines.on = !outlines.on;
    }
    if !fog.is_changed() && !outlines.is_changed() && !outlines.is_added() {
        return;
    }
    if let Some(mut m) = materials.get_mut(&outlines.material) {
        m.outline.fog = fog.0;
        m.outline.params.x = if outlines.on { WIDTH } else { 0.0 };
    }
}
