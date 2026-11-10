//! Shader step 07: the crowd's walking runs in a compute shader
//! (`assets/shaders/walk.wesl`, docs/shaders/07-crowds-compute.md).
//!
//! The pieces, in Bevy's two worlds:
//! - **Main world:** [`CrowdCompute`] holds the two buffers (people, targets),
//!   the terrain heightmap and the per-frame parameters. `crowd.rs` writes the
//!   targets buffer when a tick arrives and updates `dt` every frame.
//! - **Render world:** the resource is copied over every frame
//!   (`ExtractResource`). There we build the compute pipeline once, a bind
//!   group per frame (the GPU buffers behind the assets, the heightmap texture,
//!   a uniform buffer with the parameters), and dispatch one invocation per
//!   person before the camera draws, so the people shader sees this frame's
//!   positions.

use bevy::core_pipeline::schedule::camera_driver;
use bevy::prelude::*;
use bevy::render::{
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    render_asset::RenderAssets,
    render_resource::{
        binding_types::{
            storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, uniform_buffer,
        },
        *,
    },
    renderer::{RenderContext, RenderDevice, RenderGraph, RenderQueue},
    storage::{GpuShaderBuffer, ShaderBuffer},
    texture::GpuImage,
    Render, RenderApp, RenderStartup, RenderSystems,
};
use std::borrow::Cow;

const SHADER: &str = "shaders/walk.wesl";
const WORKGROUP: u32 = 64;

pub struct CrowdComputePlugin;

impl Plugin for CrowdComputePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractResourcePlugin::<CrowdCompute>::default());
        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(
                Render,
                prepare_bind_group.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(RenderGraph, dispatch.before(camera_driver));
    }
}

/// The walking parameters (a uniform in walk.wesl).
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct WalkParams {
    pub dt: f32,
    pub walk: f32,
    pub count: u32,
    pub pad: u32,
    /// Terrain: x, z of texel (0, 0); texel spacing; height scale.
    pub grid: Vec4,
    /// Terrain: texels across and deep.
    pub dims: Vec4,
}

/// Everything the compute pass needs, mirrored into the render world.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct CrowdCompute {
    pub people: Handle<ShaderBuffer>,
    pub targets: Handle<ShaderBuffer>,
    pub heights: Handle<Image>,
    pub params: WalkParams,
}

#[derive(Resource)]
struct WalkPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

#[derive(Resource)]
struct WalkBindGroup(BindGroup, u32);

fn init_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "crowd_walk",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_sized(false, None),
                storage_buffer_read_only_sized(false, None),
                texture_2d(TextureSampleType::Float { filterable: false }),
                uniform_buffer::<WalkParams>(false),
            ),
        ),
    );
    let pipeline = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("crowd_walk".into()),
        layout: vec![layout.clone()],
        shader: asset_server.load(SHADER),
        entry_point: Some(Cow::from("walk")),
        ..default()
    });
    commands.insert_resource(WalkPipeline { layout, pipeline });
}

#[allow(clippy::too_many_arguments)]
fn prepare_bind_group(
    mut commands: Commands,
    crowd: Option<Res<CrowdCompute>>,
    pipeline: Option<Res<WalkPipeline>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
) {
    commands.remove_resource::<WalkBindGroup>();
    let (Some(crowd), Some(pipeline)) = (crowd, pipeline) else {
        return;
    };
    let (Some(people), Some(targets), Some(heights)) = (
        buffers.get(&crowd.people),
        buffers.get(&crowd.targets),
        images.get(&crowd.heights),
    ) else {
        return;
    };
    let mut params = UniformBuffer::from(crowd.params);
    params.write_buffer(&device, &queue);
    let group = device.create_bind_group(
        "crowd_walk",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            people.buffer.as_entire_binding(),
            targets.buffer.as_entire_binding(),
            &heights.texture_view,
            &params,
        )),
    );
    commands.insert_resource(WalkBindGroup(group, crowd.params.count));
}

fn dispatch(
    mut ctx: RenderContext,
    group: Option<Res<WalkBindGroup>>,
    pipeline: Option<Res<WalkPipeline>>,
    cache: Res<PipelineCache>,
) {
    let (Some(group), Some(pipeline)) = (group, pipeline) else {
        return;
    };
    let Some(compute) = cache.get_compute_pipeline(pipeline.pipeline) else {
        return; // still compiling (or a shader error, printed by Bevy)
    };
    let mut pass = ctx
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("crowd_walk"),
            ..default()
        });
    pass.set_pipeline(compute);
    pass.set_bind_group(0, &group.0, &[]);
    pass.dispatch_workgroups(group.1.div_ceil(WORKGROUP), 1, 1);
}
