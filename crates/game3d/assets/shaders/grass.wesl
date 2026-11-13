// Shader step 06: grass with wind. Explained in docs/shaders/06-grass.md.
//
// Hundreds of thousands of blades, each made of 5 vertices. The mesh of a chunk
// holds many copies of the same little blade shape; the vertex shader finds out
// WHICH blade it belongs to (from its vertex index) and reads that blade's data
// (position, height, lean, tint) from a storage buffer. This is "vertex
// pulling": per-blade data in a buffer, indexed in the shader, the same idea as
// instancing.

import bevy_pbr::render::{
    mesh_functions,
    mesh_bindings::mesh,
    forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex},
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
};
import super::toon_lighting::{Toon, toon_params, toon_terms};
import super::fog::{apply_fog, seasonal};

// One blade; matches the 8 floats per blade written in src/nature.rs.
struct Blade {
    pos: vec3<f32>, // base on the terrain (world space)
    height: f32,    // world units
    lean: f32,      // rotation around y (radians)
    tint: f32,      // 0..1 colour variation
    pad: vec2<f32>, // keeps each blade 32 bytes (storage buffer layout)
};

struct Grass {
    toon: Toon,
    wind: vec4<f32>,   // xy: wind direction, z: amplitude, w: wave frequency
    params: vec4<f32>, // x: fade start, y: fade end (distance to camera), z: debug
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> grass: Grass;
// A storage buffer: a read-only array of any length, indexed in the shader.
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> blades: array<Blade>;

const VERTS_PER_BLADE: u32 = 5u;

@vertex
fn vertex(vertex_in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    // Which blade? Bevy packs many meshes into one big vertex buffer, so the
    // index starts at this mesh's first vertex; subtract it, then divide.
    let first = mesh[vertex_in.instance_index].first_vertex_index;
    let b = blades[(vertex_index - first) / VERTS_PER_BLADE];
    let h = v.position.y;                 // 0 at the root, 1 at the tip (mesh shape)

    // Fade out with distance: shrink far blades to nothing, so they cost
    // vertices but no pixels.
    let d = length(view.world_position - b.pos);
    let keep = smoothstep(grass.params.y, grass.params.x, d);
    // Step 11: deep snow buries the grass.
    let height = b.height * keep * (1.0 - 0.7 * grass.toon.fog.ground.x);

    // The blade's width direction, rotated by its lean.
    let across = vec3<f32>(cos(b.lean), 0.0, sin(b.lean)) * v.position.x * keep;

    // Wind: a wave rolling across the field, times slow gusts. The bend grows
    // with h squared, like a bending stalk: the root stays put.
    // Step 11: the wind of the weather; a gale bends the blades much further.
    let dir = grass.toon.fog.wind.xy;
    let strength = 0.35 + 1.8 * grass.toon.fog.wind.z;
    let wave = sin(dot(b.pos.xz, dir) * grass.wind.w - globals.time * 2.0) * 0.5 + 0.5;
    let gust = sin(globals.time * 0.7 + b.pos.x * 0.05 + b.pos.z * 0.03) * 0.5 + 0.5;
    let bend = (0.25 + wave * gust) * grass.wind.z * strength * h * h;
    // Keep the blade's length: as the tip moves sideways it also drops.
    let up = h * height * cos(bend);
    let side = sin(bend) * height;
    let world = b.pos + across + vec3<f32>(dir.x * side, up, dir.y * side);

    var out: VertexOutput;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    // Light the blade as if it faced up, tilted with the wind: soft and even.
    out.world_normal = normalize(vec3<f32>(dir.x * bend, 1.0, dir.y * bend));
@if(VERTEX_UVS_A)
    out.uv = vec2<f32>(h, b.tint);       // the fragment stage needs h and tint
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    pbr.N = normalize(in.world_normal);   // the same on both sides of the blade
    let h = in.uv.x;
    let tint = in.uv.y;
    // Dark at the root, light at the tip, varied per blade.
    let root = vec3<f32>(0.16, 0.30, 0.12);
    let tip = mix(vec3<f32>(0.50, 0.68, 0.26), vec3<f32>(0.70, 0.72, 0.32), tint);
    // Step 11: seasons; snow settles on the tips (the normal tilts up along the blade).
    let albedo = seasonal(mix(root, tip, h), vec3<f32>(0.0, h, 0.0), in.world_position.xyz, 0.5, grass.toon.fog);

    let t = toon_terms(pbr, toon_params(grass.toon));
    var out: FragmentOutput;
    if u32(grass.params.z) == 1u {
        out.color = vec4<f32>(vec3<f32>(h), 1.0);            // debug: height along the blade
    } else {
        let fogged = apply_fog(albedo * t.light, in.world_position.xyz, grass.toon.fog);
        out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(fogged, 1.0));
    }
    return out;
}
