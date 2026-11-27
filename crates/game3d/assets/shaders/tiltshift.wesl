// Shader step 20: tilt-shift, the miniature look. Explained in
// docs/shaders/20-tilt-shift.md.
//
// Two kinds of full-screen pass:
//   downsample  image -> half -> quarter -> eighth size, a 13-tap average
//   composite   the image, fading into the quarter and then the eighth mip
//               above and below a sharp band across the screen

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Tilt {
    centre: f32,   // where the sharp band sits, 0 top .. 1 bottom
    band: f32,     // half its height
    fade: f32,     // how far beyond it the blur takes to be full
    strength: f32, // 0 off .. 1
};

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;
@group(0) @binding(2) var<uniform> tilt: Tilt;
// Only the composite pass binds these: the quarter and eighth size mips.
@group(0) @binding(3) var quarter: texture_2d<f32>;
@group(0) @binding(4) var eighth: texture_2d<f32>;

// The bloom's 13-tap downsample (docs/shaders/08-bloom.md).
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
    return e * 0.125
        + (a + c + g + i) * 0.03125
        + (b + d + f + h) * 0.0625
        + (j + k + l + m) * 0.125;
}

@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var c = downsample13(in.uv);
    if any(c != c) { c = vec3<f32>(0.0); } // a NaN would spread through the blur
    return vec4<f32>(min(c, vec3<f32>(64.0)), 1.0);
}

// A 3x3 tent filter on a small mip: smooths its texels away.
fn tent(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(t));
    var c = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let w = f32((2 - abs(x)) * (2 - abs(y)));
            c += textureSample(t, input_sampler, uv + vec2<f32>(f32(x), f32(y)) * px).rgb * w;
        }
    }
    return c / 16.0;
}

@fragment
fn composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let image = textureSample(input, input_sampler, in.uv);
    // 0 in the sharp band, 1 where the blur is full.
    let off = abs(in.uv.y - tilt.centre) - tilt.band;
    let a = smoothstep(0.0, tilt.fade, off) * tilt.strength;
    if a <= 0.0 {
        return image;
    }
    // A light blur first (quarter size), a heavy one further out (eighth).
    let blur = mix(tent(quarter, in.uv), tent(eighth, in.uv), smoothstep(0.4, 1.0, a));
    return vec4<f32>(mix(image.rgb, blur, smoothstep(0.0, 0.5, a)), image.a);
}
