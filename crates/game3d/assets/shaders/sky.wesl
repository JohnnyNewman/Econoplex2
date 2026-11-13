// Shader step 01: gradient sky. Explained line by line in docs/shaders/01-sky.md.
// Step 11 (docs/shaders/11-day-night-weather.md) adds the moon, stars and clouds.
//
// This is WESL: WGSL plus `import` statements (and `@if` conditions). Everything
// below the imports is plain WGSL. Save this file while econoplex3d runs and the
// sky recompiles on the fly; a compile error is printed to the console and the
// last working version stays on screen.

// What Bevy's vertex shader hands to every fragment: the interpolated world
// position (and normal, UV, ...). We only use the world position.
import bevy_pbr::render::forward_io::VertexOutput;
// The view uniform: camera position and matrices, the same for every pixel.
// `globals.time`: seconds since start, moves the clouds.
import bevy_pbr::render::mesh_view_bindings::{view, globals};

// Our uniform block. Its layout matches `SkyUniform` in src/sky.rs field by field.
struct Sky {
    horizon: vec4<f32>,   // colour at the horizon (linear RGB)
    zenith: vec4<f32>,    // colour straight up
    ground: vec4<f32>,    // haze below the horizon
    sun_dir: vec4<f32>,   // xyz: unit vector toward the sun
    sun_color: vec4<f32>, // rgb: sun colour, a: disc radius (radians)
    debug: vec4<f32>,     // x: debug view
    moon_dir: vec4<f32>,  // xyz: toward the moon, w: phase (0 new, 0.5 full)
    weather: vec4<f32>,   // x: cloud cover, y: stars, z: haze, w: precipitation
    wind: vec4<f32>,      // xy: wind (m/s), drifts the clouds
    cloud_color: vec4<f32>, // rgb: lit cloud colour, a: shade of the dark side
};

// Bind group "material" (Bevy picks its number), binding 0 = `#[uniform(0)]`.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: Sky;

// ---- noise for stars and clouds

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}
fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(i), hash2(i + vec2(1.0, 0.0)), u.x),
               mix(hash2(i + vec2(0.0, 1.0)), hash2(i + vec2(1.0, 1.0)), u.x), u.y);
}
// Fractal noise: four octaves, each half as strong and twice as fine.
fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.5;
    var s = 0.0;
    for (var i = 0; i < 4; i++) {
        s += a * noise2(p);
        p = p * 2.03 + vec2<f32>(1.7, 9.2);
        a *= 0.5;
    }
    return s;
}

// Stars: one candidate per cell of a grid on the sky; a few cells hold a star,
// which twinkles a little.
fn stars(d: vec3<f32>) -> f32 {
    // Direction to a 2D grid: longitude and height on the dome.
    let uv = vec2<f32>(atan2(d.z, d.x) * 120.0, d.y * 190.0);
    let cell = floor(uv);
    let r = hash2(cell);
    if r < 0.93 { return 0.0; }
    let centre = cell + vec2<f32>(hash2(cell + 3.1), hash2(cell + 7.7));
    let dist = length(uv - centre);
    let twinkle = 0.75 + 0.25 * sin(globals.time * (1.5 + r * 3.0) + r * 40.0);
    return smoothstep(0.35, 0.0, dist) * (r - 0.93) / 0.07 * twinkle;
}

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

    // Hide sun, moon and stars below the horizon.
    let above = smoothstep(-0.02, 0.01, h);

    // Stars (step 11): only at night and through gaps in the clouds (below).
    let star = stars(d) * sky.weather.y * smoothstep(0.02, 0.2, h);

    // Sun: cosine of the angle between where we look and where the sun is.
    let cos_t = dot(d, sky.sun_dir.xyz);
    // Disc: 1 inside the radius r, 0 outside, with a soft rim. cos() is
    // decreasing, so cos(1.2 r) < cos(r): the edge goes from 1.2 r in to r.
    let r = sky.sun_color.a;
    let disc = smoothstep(cos(r * 1.2), cos(r), cos_t);
    // Glow: a cheap scattering lobe. A larger exponent gives a tighter glow;
    // two lobes give a bright core and a wide soft halo.
    let glow = pow(max(cos_t, 0.0), 300.0) * 0.5 + pow(max(cos_t, 0.0), 12.0) * 0.12;
    var lights = sky.sun_color.rgb * (disc + glow);

    // Moon (step 11): a pale disc, its phase a second disc shifted across it
    // that cuts away the dark part, and a faint glow.
    let m = sky.moon_dir.xyz;
    let cos_m = dot(d, m);
    let mr = 0.028;
    let moon_disc = smoothstep(cos(mr * 1.15), cos(mr), cos_m);
    // Where on the disc: offsets along two axes at right angles to the moon.
    let side = normalize(cross(m, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(side, m);
    let off = vec2<f32>(dot(d - m, side), dot(d - m, up)) / mr;
    let phase = sky.moon_dir.w;               // 0 new, 0.5 full, 1 new
    let shift = (1.0 - abs(phase - 0.5) * 2.0); // 0 new .. 1 full
    let shadow_x = sign(phase - 0.5) * shift * 2.0;
    let lit = smoothstep(0.95, 1.05, length(off - vec2<f32>(shadow_x, 0.0)));
    let moon_light = max(lit, step(0.98, shift));
    let night = sky.weather.y + 0.15;
    let moon_col = vec3<f32>(0.9, 0.92, 1.0) * 1.6 * moon_disc * mix(0.06, 1.0, moon_light)
        + vec3<f32>(0.25, 0.3, 0.45) * pow(max(cos_m, 0.0), 60.0) * 0.4 * shift;
    lights += moon_col * clamp(night, 0.0, 1.0);

    // Clouds (step 11): a layer of fractal noise, seen from below. A point of
    // the dome maps to a point of a flat cloud deck (d.xz / d.y), so clouds
    // shrink toward the horizon like real ones; the wind moves the deck.
    var cloud = 0.0;
    var cloud_rgb = vec3<f32>(0.0);
    let cover = sky.weather.x;
    if h > 0.0 {
        let deck = d.xz / (h + 0.08) * 1.6 + sky.wind.xy * globals.time * 0.004;
        let n = fbm(deck);
        // More cover: a lower threshold, so more of the noise counts as cloud.
        let threshold = mix(0.72, 0.2, cover);
        cloud = smoothstep(threshold, threshold + 0.22, n) * smoothstep(0.0, 0.12, h);
        cloud = max(cloud, smoothstep(0.75, 1.0, cover) * 0.9 * smoothstep(0.0, 0.05, h));
        // Thicker parts are darker underneath; toward the sun the edges glow.
        let thick = smoothstep(threshold, threshold + 0.5, fbm(deck * 1.7 + 4.0));
        let shade = mix(1.0, sky.cloud_color.a, thick);
        let silver = pow(max(cos_t, 0.0), 6.0) * (1.0 - thick) * 0.8;
        cloud_rgb = sky.cloud_color.rgb * shade + sky.sun_color.rgb * 0.08 * silver;
    }
    // Clouds hide sun, moon and stars, then cover the gradient.
    lights = (lights + vec3<f32>(star)) * above * (1.0 - cloud);
    col = mix(col, cloud_rgb, cloud) + lights;

    // Haze (mist, rain): everything melts toward the horizon colour.
    col = mix(col, sky.horizon.rgb, sky.weather.z * 0.5 * (1.0 - t * 0.5));

    // Debug views (keys 0-3; 4 is yours, exercise 3): show an intermediate value as a colour.
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
