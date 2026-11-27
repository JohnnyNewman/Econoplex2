//! Shader step 20: tilt-shift, the miniature look (docs/shaders/20-tilt-shift.md).
//!
//! A real tilt-shift lens tilts its plane of focus, so only a band across
//! the picture is sharp; our eyes read that as a small model seen up close.
//! Here, for every camera with [`TiltShift`], after the bloom and before
//! tonemapping (the image is still HDR):
//! 1. **downsample**: the image -> mip 0 (half size) -> mip 1 (quarter) ->
//!    mip 2 (eighth), each a 13-tap average (the bloom's filter);
//! 2. **composite**: towards the top and bottom of the screen the image
//!    fades into the blurred mips (quarter, then eighth), smoothly; the band
//!    around `centre` stays sharp.
//!
//! Key: `X` tilt-shift on/off.

use bevy::core_pipeline::{tonemapping::tonemapping, Core3d, Core3dSystems, FullscreenShader};
use bevy::prelude::*;
use bevy::render::{
    extract_component::{ExtractComponent, ExtractComponentPlugin},
    render_resource::{
        binding_types::{sampler, texture_2d, uniform_buffer},
        *,
    },
    renderer::{RenderContext, RenderDevice, RenderQueue},
    texture::{CachedTexture, TextureCache},
    view::{ExtractedView, ViewTarget},
    Render, RenderApp, RenderStartup, RenderSystems,
};
use std::borrow::Cow;

const SHADER: &str = "shaders/tiltshift.wesl";
/// Number of mips below the full-size image (mip 0 is half size).
const MIPS: usize = 3;
/// The HDR format of the camera's image and of the mips.
const FORMAT: TextureFormat = TextureFormat::Rgba16Float;

pub struct TiltShiftPlugin;

impl Plugin for TiltShiftPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<TiltShift>::default())
            .add_systems(Update, tilt_keys);
        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_mips.in_set(RenderSystems::PrepareResources))
            .add_systems(
                Core3d,
                tilt_pass
                    .in_set(Core3dSystems::PostProcess)
                    .after(crate::bloom::bloom_pass)
                    .before(tonemapping),
            );
    }
}

/// Put this on a camera (with `Hdr`) for the miniature look.
#[derive(Component, Clone, Copy, ExtractComponent, ShaderType, Debug)]
#[extract_app(RenderApp)]
pub struct TiltShift {
    /// Where the sharp band sits, 0 top .. 1 bottom of the screen.
    pub centre: f32,
    /// Half its height (screen fraction): sharp within it.
    pub band: f32,
    /// How far beyond the band the blur takes to be full.
    pub fade: f32,
    /// How strong the blur gets (0 off .. 1).
    pub strength: f32,
}

impl Default for TiltShift {
    fn default() -> Self {
        Self {
            centre: 0.5,
            band: 0.32,
            fade: 0.18,
            strength: 0.5,
        }
    }
}

fn tilt_keys(keys: Res<ButtonInput<KeyCode>>, mut cams: Query<&mut TiltShift>) {
    if keys.just_pressed(KeyCode::KeyX) {
        for mut t in &mut cams {
            t.strength = if t.strength > 0.0 { 0.0 } else { 0.5 };
        }
    }
}

#[derive(Resource)]
struct TiltPipelines {
    layout: BindGroupLayoutDescriptor,
    composite_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    downsample: CachedRenderPipelineId,
    composite: CachedRenderPipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "tilt",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<TiltShift>(false),
            ),
        ),
    );
    let composite_layout = BindGroupLayoutDescriptor::new(
        "tilt_composite",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<TiltShift>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    // Linear filtering and clamped edges: bilinear taps between texels are what
    // make the downsample and upsample filters cheap.
    let sampler = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..default()
    });
    let shader = asset_server.load(SHADER);
    let pipeline =
        |entry: &'static str, layout: &BindGroupLayoutDescriptor, blend: Option<BlendState>| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some(format!("tilt_{entry}").into()),
                layout: vec![layout.clone()],
                vertex: fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: shader.clone(),
                    entry_point: Some(Cow::from(entry)),
                    targets: vec![Some(ColorTargetState {
                        format: FORMAT,
                        blend,
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                ..default()
            })
        };
    commands.insert_resource(TiltPipelines {
        downsample: pipeline("downsample", &layout, None),
        composite: pipeline("composite", &composite_layout, None),
        layout,
        composite_layout,
        sampler,
    });
}

/// The mip chain of one camera.
#[derive(Component)]
struct TiltMips(Vec<CachedTexture>);

/// Cameras with tilt-shift.
type TiltViews = (With<ExtractedView>, With<TiltShift>);

fn prepare_mips(
    mut commands: Commands,
    device: Res<RenderDevice>,
    mut textures: ResMut<TextureCache>,
    views: Query<(Entity, &ViewTarget), TiltViews>,
) {
    for (entity, target) in &views {
        let size = target.main_texture().size();
        let mips = (0..MIPS)
            .map(|k| {
                let div = 2u32 << k; // mip 0 is half size
                textures.get(
                    &device,
                    TextureDescriptor {
                        label: Some("tilt_mip"),
                        size: Extent3d {
                            width: (size.width / div).max(1),
                            height: (size.height / div).max(1),
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: TextureDimension::D2,
                        format: FORMAT,
                        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                )
            })
            .collect();
        commands.entity(entity).insert(TiltMips(mips));
    }
}

fn tilt_pass(
    view: bevy::render::renderer::ViewQuery<(&ViewTarget, &TiltShift, &TiltMips)>,
    pipelines: Option<Res<TiltPipelines>>,
    cache: Res<PipelineCache>,
    queue: Res<RenderQueue>,
    mut ctx: RenderContext,
) {
    let Some(p) = pipelines else { return };
    let (target, settings, mips) = view.into_inner();
    if target.main_texture_format() != FORMAT || settings.strength <= 0.0 {
        return; // needs an HDR camera; or off
    }
    let (Some(downsample), Some(composite)) = (
        cache.get_render_pipeline(p.downsample),
        cache.get_render_pipeline(p.composite),
    ) else {
        return; // still compiling
    };
    let device = ctx.render_device().clone();
    let mut uniform = UniformBuffer::from(*settings);
    uniform.write_buffer(&device, &queue);
    let layout = cache.get_bind_group_layout(&p.layout);
    let group = |input: &TextureView| {
        device.create_bind_group(
            "tilt",
            &layout,
            &BindGroupEntries::sequential((input, &p.sampler, &uniform)),
        )
    };
    let post = target.post_process_write();
    let encoder = ctx.command_encoder();
    let mut draw = |pipeline: &RenderPipeline, bind: &BindGroup, out: &TextureView| {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("tilt_shift"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Default::default()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    };
    // 1. Downsample: the image -> mip 0 -> mip 1 -> mip 2.
    draw(downsample, &group(post.source), &mips.0[0].default_view);
    for k in 0..MIPS - 1 {
        draw(
            downsample,
            &group(&mips.0[k].default_view),
            &mips.0[k + 1].default_view,
        );
    }
    // 2. Composite: the sharp band in the middle, the blur above and below.
    let composite_group = device.create_bind_group(
        "tilt_composite",
        &cache.get_bind_group_layout(&p.composite_layout),
        &BindGroupEntries::sequential((
            post.source,
            &p.sampler,
            &uniform,
            &mips.0[1].default_view,
            &mips.0[2].default_view,
        )),
    );
    draw(composite, &composite_group, post.destination);
}
