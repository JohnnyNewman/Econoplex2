// M4: people as one crowd, with procedural walking. See docs/shaders/07-crowds-compute.md.
//
// Like the grass (step 06): one mesh holds many copies of a small figure, and
// the vertex shader reads each figure's data from a storage buffer by
// `(vertex_index - first_vertex) / VERTS_PER_PERSON`. The CPU writes the buffer
// every frame (position, heading, colour, size, walking or not); the shader
// animates the walk: a bob and a forward lean, out of step from person to
// person.

import bevy_pbr::render::{
    mesh_functions,
    mesh_bindings::mesh,
    forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex},
    mesh_view_bindings::globals,
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
};
import super::toon_lighting::{Toon, toon_params, toon_terms};
import super::fog::apply_fog;

// One person; matches the 8 floats per person written in src/crowd.rs.
struct Person {
    pos: vec3<f32>, // feet, world space
    heading: f32,   // direction of travel (radians around y)
    color: vec3<f32>,
    size: f32,      // 1 adult, 0.65 child; negative = walking; 0 = nobody
};

struct Crowd {
    toon: Toon,
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> crowd: Crowd;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> people: array<Person>;

const VERTS_PER_PERSON: u32 = 60u;

@vertex
fn vertex(vertex_in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let first = mesh[vertex_in.instance_index].first_vertex_index;
    let index = (vertex_index - first) / VERTS_PER_PERSON;
    let p = people[index];
    let walking = p.size < 0.0;
    let size = abs(p.size);

    // Procedural walk: each step lifts the body (|sin| makes two bumps per
    // stride) and the body leans into the direction of travel. The phase comes
    // from the person's index, so a crowd doesn't march in lockstep.
    let phase = f32(index) * 1.618;
    let stride = globals.time * 9.0 + phase;
    let bob = select(0.0, abs(sin(stride)) * 0.06, walking);
    let lean = select(0.0, 0.18, walking);

    // The figure in its own space: x across, y up, z forward. Lean forward
    // (rotate around x by `lean`, more at the top), then turn to the heading.
    var local = v.position * size;
    local.z += local.y * lean;
    local.y += bob;
    let c = cos(p.heading);
    let s = sin(p.heading);
    let turned = vec3<f32>(local.x * c + local.z * s, local.y, -local.x * s + local.z * c);
    let n = v.normal;
    let normal = vec3<f32>(n.x * c + n.z * s, n.y, -n.x * s + n.z * c);

    let world = p.pos + turned;
    var out: VertexOutput;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = normalize(normal);
@if(VERTEX_UVS_A)
    out.uv = vec2<f32>(f32(index), 0.0);  // the fragment stage looks the colour up
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    pbr.N = normalize(in.world_normal);
    let albedo = people[u32(in.uv.x + 0.5)].color;
    let t = toon_terms(pbr, toon_params(crowd.toon));
    let fogged = apply_fog(albedo * t.light, in.world_position.xyz, crowd.toon.fog);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(fogged, 1.0));
    return out;
}
