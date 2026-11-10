//! Shader step 08: a hand-written bloom (docs/shaders/08-bloom.md), in place of
//! Bevy's built-in one.
//!
//! In the render world, for every camera with [`BloomSettings`], before
//! tonemapping (the image is still HDR there):
//! 1. **prefilter**: the HDR image -> mip 0 (half size), keeping only what is
//!    brighter than the threshold;
//! 2. **downsample**: mip 0 -> mip 1 -> ... -> mip 5, each half the size;
//! 3. **upsample**: mip 5 -> mip 4 -> ... -> mip 0, each pass *added* onto the
//!    larger mip (additive blending);
//! 4. **composite**: image + intensity * mip 0 -> the camera's next image.
//!
//! Keys: `B` bloom on/off; `N` shows the glow alone (debug).

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

const SHADER: &str = "shaders/bloom.wesl";
/// Number of mips below the full-size image (mip 0 is half size).
const MIPS: usize = 6;
/// The HDR format of the camera's image and of the mips.
const FORMAT: TextureFormat = TextureFormat::Rgba16Float;

pub struct BloomPlugin;

impl Plugin for BloomPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<BloomSettings>::default())
            .add_systems(Update, bloom_keys);
        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_mips.in_set(RenderSystems::PrepareResources))
            .add_systems(
                Core3d,
                bloom_pass
                    .in_set(Core3dSystems::PostProcess)
                    .before(tonemapping),
            );
    }
}

/// Put this on a camera (with `Hdr`) to give it our bloom.
#[derive(Component, Clone, Copy, ExtractComponent, ShaderType, Debug)]
#[extract_app(RenderApp)]
pub struct BloomSettings {
    /// Brightness where the glow starts (linear HDR; 1.0 is "white").
    pub threshold: f32,
    /// Softness of that start.
    pub knee: f32,
    /// How much glow is added back.
    pub intensity: f32,
    /// 1: show only the glow.
    pub debug: f32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            threshold: 1.2,
            knee: 0.5,
            intensity: 0.25,
            debug: 0.0,
        }
    }
}

fn bloom_keys(keys: Res<ButtonInput<KeyCode>>, mut cams: Query<&mut BloomSettings>) {
    for mut b in &mut cams {
        if keys.just_pressed(KeyCode::KeyB) {
            b.intensity = if b.intensity > 0.0 { 0.0 } else { 0.25 };
        }
        if keys.just_pressed(KeyCode::KeyN) {
            b.debug = 1.0 - b.debug;
        }
    }
}

#[derive(Resource)]
struct BloomPipelines {
    layout: BindGroupLayoutDescriptor,
    composite_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    prefilter: CachedRenderPipelineId,
    downsample: CachedRenderPipelineId,
    upsample: CachedRenderPipelineId,
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
        "bloom",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<BloomSettings>(false),
            ),
        ),
    );
    let composite_layout = BindGroupLayoutDescriptor::new(
        "bloom_composite",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<BloomSettings>(false),
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
                label: Some(format!("bloom_{entry}").into()),
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
    // Upsampling adds onto what the larger mip already holds: one + one.
    let additive = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::OVER,
    };
    commands.insert_resource(BloomPipelines {
        prefilter: pipeline("prefilter", &layout, None),
        downsample: pipeline("downsample", &layout, None),
        upsample: pipeline("upsample", &layout, Some(additive)),
        composite: pipeline("composite", &composite_layout, None),
        layout,
        composite_layout,
        sampler,
    });
}

/// The mip chain of one camera.
#[derive(Component)]
struct BloomMips(Vec<CachedTexture>);

/// Cameras with our bloom.
type BloomViews = (With<ExtractedView>, With<BloomSettings>);

fn prepare_mips(
    mut commands: Commands,
    device: Res<RenderDevice>,
    mut textures: ResMut<TextureCache>,
    views: Query<(Entity, &ViewTarget), BloomViews>,
) {
    for (entity, target) in &views {
        let size = target.main_texture().size();
        let mips = (0..MIPS)
            .map(|k| {
                let div = 2u32 << k; // mip 0 is half size
                textures.get(
                    &device,
                    TextureDescriptor {
                        label: Some("bloom_mip"),
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
        commands.entity(entity).insert(BloomMips(mips));
    }
}

fn bloom_pass(
    view: bevy::render::renderer::ViewQuery<(&ViewTarget, &BloomSettings, &BloomMips)>,
    pipelines: Option<Res<BloomPipelines>>,
    cache: Res<PipelineCache>,
    queue: Res<RenderQueue>,
    mut ctx: RenderContext,
) {
    let Some(p) = pipelines else { return };
    let (target, settings, mips) = view.into_inner();
    if target.main_texture_format() != FORMAT {
        return; // needs an HDR camera
    }
    let (Some(prefilter), Some(downsample), Some(upsample), Some(composite)) = (
        cache.get_render_pipeline(p.prefilter),
        cache.get_render_pipeline(p.downsample),
        cache.get_render_pipeline(p.upsample),
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
            "bloom",
            &layout,
            &BindGroupEntries::sequential((input, &p.sampler, &uniform)),
        )
    };
    let post = target.post_process_write();
    let encoder = ctx.command_encoder();
    // One full-screen triangle into `out`, keeping (`load`) or clearing it.
    let mut draw = |pipeline: &RenderPipeline, bind: &BindGroup, out: &TextureView, load: bool| {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("bloom"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: if load {
                        LoadOp::Load
                    } else {
                        LoadOp::Clear(Default::default())
                    },
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    };
    // 1. Prefilter: the image -> mip 0.
    draw(
        prefilter,
        &group(post.source),
        &mips.0[0].default_view,
        false,
    );
    // 2. Downsample: mip k -> mip k+1.
    for k in 0..MIPS - 1 {
        draw(
            downsample,
            &group(&mips.0[k].default_view),
            &mips.0[k + 1].default_view,
            false,
        );
    }
    // 3. Upsample: mip k+1 added onto mip k.
    for k in (0..MIPS - 1).rev() {
        draw(
            upsample,
            &group(&mips.0[k + 1].default_view),
            &mips.0[k].default_view,
            true,
        );
    }
    // 4. Composite: image + glow -> the camera's next image.
    let composite_group = device.create_bind_group(
        "bloom_composite",
        &cache.get_bind_group_layout(&p.composite_layout),
        &BindGroupEntries::sequential((post.source, &p.sampler, &uniform, &mips.0[0].default_view)),
    );
    draw(composite, &composite_group, post.destination, false);
}
