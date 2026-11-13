// Shader step 06, trees: a vertex shader that sways tree crowns in the wind.
// The fragment shader is the toon shader of step 02 (toon.wesl), unchanged.
//
// The trees of a chunk are baked into one mesh (nature.rs `spawn_forest`):
// every vertex is already in world space. Two extra attributes say how it
// sways: UV 0 x is how far up its crown it is (0 at the base and for trunks,
// 1 at the tip), UV 1 is its tree's root (x, z), which gives each tree its own
// phase, so the forest doesn't sway in lockstep. The colour is a vertex colour
// (the material is white; `pbr_input_from_standard_material` multiplies it in).

import bevy_pbr::render::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_view_bindings::globals,
    view_transformations::position_world_to_clip,
};
import super::toon_lighting::Toon;
import super::fog::Fog;

// The toon block of the tree material (the fragment shader reads it too): its
// shared fog block carries the wind of the weather (step 11).
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> toon: Toon;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> fog_state: Fog; // the shared fog (src/fog.rs, FOG_BUFFER)

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    var world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));

    var up = 0.0;
    var root = vec2<f32>(0.0);
@if(VERTEX_UVS_A)
    up = v.uv.x;
@if(VERTEX_UVS_B)
    root = v.uv_b;
    let phase = root.x * 0.37 + root.y * 0.23;
    let t = globals.time;
    // Two sines of different speed make a less regular sway.
    // Stronger wind: faster and wider sway, and the crowns lean downwind.
    let strength = fog_state.wind.z;
    let speed = 1.0 + strength * 1.5;
    let sway = (sin(t * 1.3 * speed + phase) * 0.6 + sin(t * 2.9 * speed + phase * 1.7) * 0.25) * (0.06 + 0.3 * strength)
        + strength * 0.25;
    let wind_dir = vec3<f32>(fog_state.wind.x, 0.0, fog_state.wind.y);
    world = vec4<f32>(world.xyz + wind_dir * sway * up * up, 1.0);

    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(v.normal, vertex_in.instance_index);
@if(VERTEX_UVS_A)
    out.uv = v.uv;
@if(VERTEX_UVS_B)
    out.uv_b = v.uv_b;
@if(VERTEX_COLORS)
    out.color = v.color;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
    return out;
}
