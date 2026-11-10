// Shader step 06, trees: a vertex shader that sways tree crowns in the wind.
// The fragment shader is the toon shader of step 02 (toon.wesl), unchanged.
//
// Every crown is an ordinary entity with the same mesh and material, so Bevy
// draws them all in one instanced draw call. Each instance finds its own
// transform with `get_world_from_local(instance_index)`; its position gives it
// its own phase, so the forest doesn't sway in lockstep.

import bevy_pbr::render::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_view_bindings::globals,
    view_transformations::position_world_to_clip,
};

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    var world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));

    // How far up the crown this vertex is: the cone runs from y = -1 to 1 in
    // its own space, so 0 at the base, 1 at the tip.
    let up = clamp(v.position.y * 0.5 + 0.5, 0.0, 1.0);
    // This tree's position (the translation column of its matrix) sets its phase.
    let root = world_from_local[3].xyz;
    let phase = root.x * 0.37 + root.z * 0.23;
    let t = globals.time;
    // Two sines of different speed make a less regular sway.
    let sway = (sin(t * 1.3 + phase) * 0.6 + sin(t * 2.9 + phase * 1.7) * 0.25) * 0.12;
    let wind_dir = vec3<f32>(0.8, 0.0, 0.6);
    world = vec4<f32>(world.xyz + wind_dir * sway * up * up, 1.0);

    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(v.normal, vertex_in.instance_index);
@if(VERTEX_UVS_A)
    out.uv = v.uv;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
    return out;
}
