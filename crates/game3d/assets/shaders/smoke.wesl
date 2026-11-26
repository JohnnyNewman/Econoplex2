// Shader step 16: chimney smoke. Explained in docs/shaders/16-building-life.md.
//
// Every quad of the mesh is one puff. Its four vertices hold the chimney top
// (the position), which corner they are (the UV, -1..1) and the puff's
// traits (the colour: r how dark, g a random seed, b its slot in the plume
// 0..1, a 1 for a workplace, 0 for a hearth). The vertex shader gives each
// puff an age that runs from 0 to 1 over a few seconds and starts again:
// young puffs sit small and dense on the chimney, old ones have risen,
// grown, drifted with the wind and faded.

import bevy_pbr::render::{
    forward_io::{Vertex, decompress_vertex},
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
};
import super::fog::apply_fog;
import super::lights::Env;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<storage, read> env: Env; // the shared fog, sun, wind (src/fog.rs, FOG_BUFFER)

struct SmokeOut {
    @builtin(position) position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) rgb: vec3<f32>,
    @location(3) alpha: f32,
    @location(4) seed: f32,
};

@vertex
fn vertex(vertex_in: Vertex) -> SmokeOut {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    var corner = vec2<f32>(0.0);
@if(VERTEX_UVS_A)
    corner = v.uv;
    var c = vec4<f32>(0.0);
@if(VERTEX_COLORS)
    c = v.color;
    let dark = c.r;
    let seed = c.g;
    let workplace = c.a;
    let f = env.fog;
    let t = globals.time;

    // The puff's age, 0..1: a plume's puffs are spread evenly over the cycle
    // (their slot), each plume starts at its own point (its seed).
    let life = mix(7.0, 5.0, workplace) * (0.9 + 0.2 * seed);
    let age = fract(t / life + c.b + seed);

    // Rise, drift with the wind (more the higher it gets), a slow wobble.
    let wind = f.wind.xy * (0.25 + 1.2 * f.wind.z);
    let rise = age * mix(2.4, 3.4, dark);
    let drift = vec3<f32>(wind.x, 0.0, wind.y) * pow(age, 1.4) * 3.0;
    let wobble = vec3<f32>(sin(t * 0.9 + seed * 40.0), 0.0, cos(t * 0.7 + seed * 25.0)) * 0.15 * age;
    let centre = v.position + vec3<f32>(0.0, rise, 0.0) + drift + wobble;
    let size = (0.15 + 0.65 * age) * mix(1.0, 1.5, dark);

    // A square facing the camera.
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let world = centre + (right * corner.x + up * corner.y) * size;

    var out: SmokeOut;
    out.position = position_world_to_clip(world);
    out.corner = corner;
    out.world = world;
    out.seed = seed;

    // How much smoke: a workplace at work smokes fully; a hearth a thin wisp
    // in summer (cooking) and fully in autumn and winter (heating).
    let cold = clamp(f.ground.w + 0.7 * f.ground.z, 0.0, 1.0);
    let amount = mix(0.3 + 0.7 * cold, 1.0, workplace);
    // Fade in at the chimney, out as it thins.
    out.alpha = min(amount * smoothstep(0.0, 0.04, age) * pow(1.0 - age, 0.8) * 1.4, 0.9);

    // Pale to black by the fire, paler as it thins; lit by the daylight.
    let grey = mix(mix(0.62, 0.1, dark), 0.6, age * 0.4);
    let daylight = clamp((f.sun_dir.y + 0.05) / 0.3, 0.0, 1.0);
    out.rgb = vec3<f32>(grey) * (0.2 + 0.8 * daylight);
    return out;
}

@fragment
fn fragment(in: SmokeOut) -> @location(0) vec4<f32> {
    // A soft round puff with a lumpy edge.
    let lump = 0.15 * sin(in.corner.x * 5.0 + in.seed * 30.0) * sin(in.corner.y * 4.0 + in.seed * 17.0);
    let a = in.alpha * (1.0 - smoothstep(0.45, 1.0, length(in.corner) + lump));
    return vec4<f32>(apply_fog(in.rgb, in.world, env.fog), a);
}
