// Shader step 05: outlines by the "inverted hull". Explained in
// docs/shaders/05-rim-outline.md.
//
// Every outlined mesh is drawn a second time with this material:
// - the vertex shader pushes each vertex out along its normal (a fatter copy);
// - the pipeline culls FRONT faces (see OutlineMaterial::specialize), so only
//   the inside of the fat copy is drawn;
// - the original mesh hides most of it; what pokes out past the silhouette is
//   the outline.

import bevy_pbr::render::{
    mesh_functions,
    forward_io::{Vertex, decompress_vertex},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
};
import super::fog::{Fog, apply_fog};

// Matches `OutlineUniform` in src/outline.rs.
struct Outline {
    color: vec4<f32>,  // the line colour
    params: vec4<f32>, // x: width (world units at the reference distance),
                       // y: reference distance
};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> outline: Outline;
// The same fog as everything else, so far-away outlines fade into the haze.
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> fog_state: Fog; // the shared fog (src/fog.rs, FOG_BUFFER)

struct OutlineOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
};

@vertex
fn vertex(vertex_in: Vertex) -> OutlineOut {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0)).xyz;
    // The normal in world space. Bevy uses the inverse-transpose of the model
    // matrix, so stretched boxes still get normals at right angles to their faces.
    let n = mesh_functions::mesh_normal_local_to_world(v.normal, vertex_in.instance_index);

    // Push out along the normal. Scaling by the distance to the camera keeps the
    // line about equally thick on screen, near or far.
    let dist = length(view.world_position - world);
    let pushed = world + n * outline.params.x * dist / outline.params.y;

    var out: OutlineOut;
    out.position = position_world_to_clip(pushed);
    out.world_position = pushed;
    return out;
}

@fragment
fn fragment(in: OutlineOut) -> @location(0) vec4<f32> {
    // A flat colour: only the silhouette of the hull shows anyway.
    return vec4<f32>(apply_fog(outline.color.rgb, in.world_position, fog_state), 1.0);
}
