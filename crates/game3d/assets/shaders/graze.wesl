// Shader step 19, livestock: sheep wander slowly and dip their heads to graze.
// Explained in docs/shaders/19-livestock.md. The fragment shader is the toon
// shader of step 02 (toon.wesl), unchanged.
//
// A town's sheep are baked into one mesh (livestock.rs), every vertex in world
// space. UV 1 is the sheep's home spot (x, z): all its vertices move with it,
// and it gives each sheep its own timing. UV 0 x is 1 for the head. The colour
// is a vertex colour (white wool, a black sheep now and then).

import bevy_pbr::render::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_view_bindings::globals,
    view_transformations::position_world_to_clip,
};
import super::toon_lighting::Toon;
import super::fog::Fog;
import super::lights::Env;

// The toon block of the material (the fragment shader reads it too).
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> toon: Toon;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> env: Env; // the shared fog and lamps (src/fog.rs, FOG_BUFFER)

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    var world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));

    var head = 0.0;
    var spot = vec2<f32>(0.0);
@if(VERTEX_UVS_A)
    head = v.uv.x;
@if(VERTEX_UVS_B)
    spot = v.uv_b;
    // Each sheep its own timing, from its spot.
    let phase = fract(sin(dot(spot, vec2<f32>(12.9898, 78.233))) * 43758.547);
    let t = globals.time;
    // Wander: a slow loop of up to half a unit around the spot (minutes per
    // round), the whole sheep together.
    let wander = vec2<f32>(sin(t * 0.06 + phase * 6.28), sin(t * 0.045 + phase * 3.7)) * 0.5;
    // Graze: the head dips for a few seconds, then comes up to look round.
    let dip = smoothstep(0.2, 0.6, sin(t * 0.5 + phase * 17.0));
    world = vec4<f32>(world.x + wander.x, world.y - head * dip * 0.1, world.z + wander.y, 1.0);

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
