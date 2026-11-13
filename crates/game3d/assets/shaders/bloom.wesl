// Shader step 08: bloom, by hand. Explained in docs/shaders/08-bloom.md.
//
// Four kinds of full-screen pass, each drawing one triangle that covers its
// target texture; the fragment shader runs once per target pixel:
//
//   prefilter   HDR image -> mip 0 (half size): keep only what is brighter
//               than a threshold (a soft knee, so it doesn't snap on)
//   downsample  mip k -> mip k+1 (half again): average 13 taps, a wide blur
//   upsample    mip k+1 -> mip k, ADDED to what is there: a 3x3 tent filter
//   composite   image + intensity * mip 0 -> the final HDR image
//
// The chain of ever smaller textures is what makes the glow wide but cheap:
// blurring a 1/32-size texture by a few pixels blurs the screen by a lot.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Bloom {
    threshold: f32, // brightness where the glow starts
    knee: f32,      // softness of that start
    intensity: f32, // how much glow is added back
    debug: f32,     // 1: show only the glow
};

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;
@group(0) @binding(2) var<uniform> bloom: Bloom;
// Only the composite pass binds this: the finished glow (mip 0).
@group(0) @binding(3) var glow: texture_2d<f32>;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// 13 bilinear taps (the "Call of Duty" downsample): a centre box and four
// overlapping corner boxes, weighted so the result doesn't flicker as bright
// pixels move.
fn downsample13(uv: vec2<f32>) -> vec3<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(input));
    let a = textureSample(input, input_sampler, uv + vec2(-2.0, -2.0) * px).rgb;
    let b = textureSample(input, input_sampler, uv + vec2( 0.0, -2.0) * px).rgb;
    let c = textureSample(input, input_sampler, uv + vec2( 2.0, -2.0) * px).rgb;
    let d = textureSample(input, input_sampler, uv + vec2(-2.0,  0.0) * px).rgb;
    let e = textureSample(input, input_sampler, uv).rgb;
    let f = textureSample(input, input_sampler, uv + vec2( 2.0,  0.0) * px).rgb;
    let g = textureSample(input, input_sampler, uv + vec2(-2.0,  2.0) * px).rgb;
    let h = textureSample(input, input_sampler, uv + vec2( 0.0,  2.0) * px).rgb;
    let i = textureSample(input, input_sampler, uv + vec2( 2.0,  2.0) * px).rgb;
    let j = textureSample(input, input_sampler, uv + vec2(-1.0, -1.0) * px).rgb;
    let k = textureSample(input, input_sampler, uv + vec2( 1.0, -1.0) * px).rgb;
    let l = textureSample(input, input_sampler, uv + vec2(-1.0,  1.0) * px).rgb;
    let m = textureSample(input, input_sampler, uv + vec2( 1.0,  1.0) * px).rgb;
    // Weights: the inner box j k l m counts 0.5, the four corner boxes 0.125 each.
    return e * 0.125
        + (a + c + g + i) * 0.03125
        + (b + d + f + h) * 0.0625
        + (j + k + l + m) * 0.125;
}

@fragment
fn prefilter(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var c = downsample13(in.uv);
    // One NaN or infinite pixel in the scene would spread through the blur into
    // a black square: drop it, and cap very bright pixels.
    if any(c != c) { c = vec3<f32>(0.0); }
    c = min(c, vec3<f32>(64.0));
    // Soft threshold: a quadratic ramp over [threshold - knee, threshold + knee],
    // then linear. Divide by the brightness to scale the colour, not clip it.
    let br = max(max(c.r, c.g), c.b);
    let t = bloom.threshold;
    let k = max(bloom.knee, 1e-4);
    var soft = clamp(br - t + k, 0.0, 2.0 * k);
    soft = soft * soft / (4.0 * k);
    let contribution = max(soft, br - t) / max(br, 1e-4);
    return vec4<f32>(c * contribution, 1.0);
}

@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(downsample13(in.uv), 1.0);
}

// A 3x3 tent filter (1 2 1 / 2 4 2 / 1 2 1, divided by 16) on the smaller mip.
// The pipeline adds the result to the larger mip (additive blending), so each
// level keeps its own detail plus the wider glow from below.
@fragment
fn upsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(input));
    var c = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let w = f32((2 - abs(x)) * (2 - abs(y)));
            c += textureSample(input, input_sampler, in.uv + vec2<f32>(f32(x), f32(y)) * px).rgb * w;
        }
    }
    return vec4<f32>(c / 16.0, 1.0);
}

@fragment
fn composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let image = textureSample(input, input_sampler, in.uv);
    let g = textureSample(glow, input_sampler, in.uv).rgb;
    if bloom.debug > 0.5 {
        return vec4<f32>(g, 1.0);          // debug: the glow alone
    }
    return vec4<f32>(image.rgb + g * bloom.intensity, image.a);
}
