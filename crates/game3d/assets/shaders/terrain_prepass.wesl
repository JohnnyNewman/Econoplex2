// The terrain's vertex shader for Bevy's prepass, which also renders the
// shadow maps: the sun "looks" at the scene from its direction, and this shader
// tells it where the terrain really is. Without it the shadow pass would see
// the flat grid. See docs/shaders/03-terrain.md, exercise 3.

import bevy_pbr::{
    prepass::io::{Vertex, VertexOutput, decompress_vertex},
    render::{mesh_functions, view_transformations::position_world_to_clip},
};
import super::terrain_common::displaced;

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    let flat = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));

    // The same displacement as the main pass; here `view` is the light's view.
    out.world_position = displaced(flat);
    out.position = position_world_to_clip(out.world_position.xyz);
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    // Directional-light shadows use an orthographic projection; Bevy keeps
    // casters behind the near plane by clamping depth (as its own prepass does).
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
}
@if(VERTEX_UVS_A)
    out.uv = v.uv;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
    return out;
}
