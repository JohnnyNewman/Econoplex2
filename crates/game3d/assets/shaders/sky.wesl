// Shader step 01: gradient sky. Explained line by line in docs/shaders/01-sky.md.
//
// This is WESL: WGSL plus `import` statements (and `@if` conditions). Everything
// below the imports is plain WGSL. Save this file while econoplex3d runs and the
// sky recompiles on the fly; a compile error is printed to the console and the
// last working version stays on screen.

// What Bevy's vertex shader hands to every fragment: the interpolated world
// position (and normal, UV, ...). We only use the world position.
import bevy_pbr::render::forward_io::VertexOutput;
// The view uniform: camera position and matrices, the same for every pixel.
import bevy_pbr::render::mesh_view_bindings::view;

// Our uniform block. Its layout matches `SkyUniform` in src/sky.rs field by field.
struct Sky {
    horizon: vec4<f32>,   // colour at the horizon (linear RGB)
    zenith: vec4<f32>,    // colour straight up
    ground: vec4<f32>,    // haze below the horizon
    sun_dir: vec4<f32>,   // xyz: unit vector toward the sun
    sun_color: vec4<f32>, // rgb: sun colour, a: disc radius (radians)
    debug: vec4<f32>,     // x: debug view
};

// Bind group "material" (Bevy picks its number), binding 0 = `#[uniform(0)]`.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: Sky;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // View direction: from the camera to this point of the sky sphere.
    // normalize() makes it unit length, so dot products below are cosines.
    let d = normalize(in.world_position.xyz - view.world_position);

    // How far above the horizon we look: -1 straight down, 1 straight up.
    let h = d.y;

    // Gradient parameter t: 0 at the horizon, 1 at the zenith. pow(h, 0.6)
    // lifts small values, so the pale band near the horizon gets wider.
    let t = smoothstep(0.0, 1.0, pow(max(h, 0.0), 0.6));
    var col = mix(sky.horizon.rgb, sky.zenith.rgb, t);

    // Below the horizon: blend toward the ground haze over a narrow band.
    // (smoothstep with edge0 > edge1 runs "backwards": 0 at h=0, 1 at h=-0.15.)
    col = mix(col, sky.ground.rgb, smoothstep(0.0, -0.15, h));

    // Sun: cosine of the angle between where we look and where the sun is.
    let cos_t = dot(d, sky.sun_dir.xyz);
    // Disc: 1 inside the radius r, 0 outside, with a soft rim. cos() is
    // decreasing, so cos(1.2 r) < cos(r): the edge goes from 1.2 r in to r.
    let r = sky.sun_color.a;
    let disc = smoothstep(cos(r * 1.2), cos(r), cos_t);
    // Glow: a cheap scattering lobe. A larger exponent gives a tighter glow;
    // two lobes give a bright core and a wide soft halo.
    let glow = pow(max(cos_t, 0.0), 300.0) * 0.5 + pow(max(cos_t, 0.0), 12.0) * 0.12;
    // Hide the sun below the horizon (no sun shining through the ground).
    let above = smoothstep(-0.02, 0.01, h);
    col += sky.sun_color.rgb * (disc + glow) * above;

    // Debug views (keys 0-3): show an intermediate value as a colour.
    // A uniform is the same for every pixel, so this branch costs almost nothing:
    // all neighbouring pixels take the same side.
    let view_id = u32(sky.debug.x);
    if view_id == 1u {
        // Direction as colour: red = +x, green = up, blue = +z.
        return vec4<f32>(d * 0.5 + 0.5, 1.0);
    } else if view_id == 2u {
        return vec4<f32>(vec3<f32>(t), 1.0);
    } else if view_id == 3u {
        return vec4<f32>(vec3<f32>(max(cos_t, 0.0)), 1.0);
    }
    return vec4<f32>(col, 1.0);
}
