// Water: rivers and lakes. Explained in docs/shaders/13-water.md.
//
// A flat plane at the water level; the terrain's river beds lie below it.
// Depth from the terrain's heightmap, ripples from travelling waves, the sky
// at grazing angles, a sun glint, foam at the shore, ice in deep winter.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, globals},
};
import super::toon_lighting::{ToonParams, toon_terms};
import super::fog::{Fog, apply_fog};
import super::lights::{Env, lamp_term};

// Matches `WaterUniform` in src/water.rs.
struct Water {
    grid: vec4<f32>,    // x, z of texel (0,0); distance between texels; height scale
    dims: vec4<f32>,    // texels across, deep; z: water level (y)
    shallow: vec4<f32>, // rgb shallow colour; a: depth that looks deep
    deep: vec4<f32>,    // rgb deep colour; a: how strongly the sky shows
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> water: Water;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var heights: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var<storage, read> env: Env; // the shared fog and lamps (src/fog.rs, FOG_BUFFER)

// The ground under the water: the terrain's heights, bilinear by hand (the
// float texture isn't filterable everywhere).
fn ground_at(xz: vec2<f32>) -> f32 {
    let t = vec2<f32>(xz.x - water.grid.x, water.grid.y - xz.y) / water.grid.z;
    let last = vec2<i32>(water.dims.xy) - 1;
    // Beyond the map the plane is open sea: deep.
    if any(t < vec2<f32>(0.0)) || any(t > vec2<f32>(last)) {
        return water.dims.z - 10.0;
    }
    let t0 = vec2<i32>(floor(t));
    let f = fract(t);
    let a = textureLoad(heights, clamp(t0, vec2<i32>(0), last), 0).r;
    let b = textureLoad(heights, clamp(t0 + vec2(1, 0), vec2<i32>(0), last), 0).r;
    let c = textureLoad(heights, clamp(t0 + vec2(0, 1), vec2<i32>(0), last), 0).r;
    let d = textureLoad(heights, clamp(t0 + vec2(1, 1), vec2<i32>(0), last), 0).r;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y) * water.grid.w;
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise.
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(i), hash2(i + vec2(1.0, 0.0)), u.x),
               mix(hash2(i + vec2(0.0, 1.0)), hash2(i + vec2(1.0, 1.0)), u.x), u.y);
}

// The slope of the water surface (d height / dx, dz) from three travelling
// waves along the wind, plus small cross ripples.
fn ripples(xz: vec2<f32>, t: f32, dir: vec2<f32>, strength: f32) -> vec2<f32> {
    let side = vec2<f32>(-dir.y, dir.x);
    var g = vec2<f32>(0.0);
    let k = array<f32, 3>(1.7, 3.1, 5.3);   // wave numbers (per unit)
    let w = array<f32, 3>(1.3, 2.1, 3.4);   // speeds
    let a = array<f32, 3>(0.05, 0.03, 0.015);
    for (var i = 0; i < 3; i++) {
        let d = normalize(dir + side * (f32(i) - 1.0) * 0.45);
        let phase = dot(xz, d) * k[i] - t * w[i];
        g += d * cos(phase) * k[i] * a[i];
    }
    // Small, slow noise so the pattern doesn't repeat.
    let n = vec2<f32>(noise(xz * 2.3 + t * 0.4) - 0.5, noise(xz * 2.3 + 17.0 - t * 0.35) - 0.5);
    return (g + n * 0.12) * strength;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    let fog: Fog = env.fog;
    let p = pbr.world_position.xyz;
    let t = globals.time;

    // How deep: water level minus the ground below.
    let depth = max(water.dims.z - ground_at(p.xz), 0.0);
    let deepness = smoothstep(0.0, water.shallow.a, depth);

    // Season and weather from the shared block: ice in deep winter, rain
    // rings while it is wet, waves with the wind.
    let ice = smoothstep(0.65, 0.9, fog.ground.w);
    let wind_dir = normalize(fog.wind.xy + vec2<f32>(1e-4, 0.0));
    // Ripples fade out with distance: far away they would only sparkle.
    let dist = length(view.world_position - p);
    let near = 1.0 - smoothstep(15.0, 90.0, dist);
    let strength = (0.35 + 1.3 * fog.wind.z) * (1.0 - ice) * near;
    var slope = ripples(p.xz, t, wind_dir, strength);
    // Rain: little rings popping up on a grid of cells.
    let wet = fog.ground.y * (1.0 - ice) * near;
    if wet > 0.05 {
        let cell = floor(p.xz * 3.0);
        let local = fract(p.xz * 3.0) - 0.5;
        let life = fract(t * 0.9 + hash2(cell));
        let r = length(local);
        let ring = sin((r - life * 0.5) * 60.0) * (1.0 - life) * smoothstep(0.5, 0.0, r);
        slope += local / max(r, 1e-3) * ring * 0.15 * wet;
    }
    let n = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    pbr.N = n;

    // Colour: shallows to deep, ice over it in winter.
    var albedo = mix(water.shallow.rgb, water.deep.rgb, deepness);
    albedo = mix(albedo, vec3<f32>(0.80, 0.87, 0.92), ice);

    // Lit like everything else (toon ramp, sun shadows), plus nearby lamps.
    let params = ToonParams(vec3<f32>(0.42, 0.48, 0.85), 0.4, 0.28, 0.62, 0.05, 0.0, 3.0);
    let tt = toon_terms(pbr, params);
    var lamps = vec3<f32>(0.0);
    for (var i = 0u; i < u32(env.lamps_info.x); i++) {
        lamps += lamp_term(env.lamps[i], p, n);
    }
    var col = albedo * (tt.light + lamps);

    // The sky at grazing angles (fresnel), the horizon colour of the fog.
    let v = normalize(view.world_position - p);
    let fresnel = pow(1.0 - max(dot(n, v), 0.0), 4.0) * (1.0 - ice * 0.7);
    col = mix(col, fog.color.rgb * 1.05, clamp(fresnel * water.deep.a, 0.0, 0.55));

    // Sun glint: the sun's reflection, sharp and brighter than white so the
    // bloom catches it. Gone at night and on ice.
    let sun = normalize(fog.sun_dir.xyz);
    let day = smoothstep(-0.02, 0.12, sun.y);
    let glint = pow(max(dot(reflect(-sun, n), v), 0.0), 180.0) * day * (1.0 - ice);
    col += fog.sun_color.rgb * 6.0 * glint;

    // Foam where the water is shallow, broken up and drifting.
    let foam_band = 1.0 - smoothstep(0.02, 0.14, depth);
    let foam = foam_band * smoothstep(0.55, 0.8, noise(p.xz * 5.0 + vec2<f32>(t * 0.3, -t * 0.2))) * (1.0 - ice);
    col = mix(col, vec3<f32>(0.92, 0.95, 0.95) * (tt.light * 0.9 + 0.05 + lamps), foam * 0.6);

    let fogged = apply_fog(col, p, fog);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(fogged, 1.0));
    return out;
}
