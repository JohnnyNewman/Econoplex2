// Shader step 03: terrain from a flat grid. Explained in docs/shaders/03-terrain.md.
//
// The vertex stage lifts each vertex of a flat grid to the height stored in a
// texture and computes its normal from the neighbouring heights. The fragment
// stage paints the biomes procedurally and lights them with the toon lighting
// of step 02.

import bevy_pbr::render::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex},
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};
import super::toon_lighting::{Toon, toon_params, toon_terms};
import super::fog::apply_fog;
// The uniform block, the textures and the height helpers, shared with the
// shadow pass (terrain_prepass.wesl).
import super::terrain_common::{terrain, biomes, texel_coord, height_at, normal_at, displaced};


// ---------------------------------------------------------------- vertex

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    // Bevy may store positions compressed; this gives plain floats back.
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    let flat = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));

    // Displacement: the grid vertex sits exactly on a texel, so we round to it
    // and lift the vertex to that texel's height (terrain_common.wesl).
    let t = vec2<i32>(round(texel_coord(flat.xz)));
    let world = displaced(flat);

    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);   // world -> clip space
    out.world_normal = normal_at(t);
@if(VERTEX_UVS_A)
    out.uv = v.uv;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    // The fragment stage looks up this mesh's flags (shadow receiver etc.).
    out.instance_index = vertex_in.instance_index;
    return out;
}

// ---------------------------------------------------------------- fragment

// Value noise: random values at lattice points, smoothly blended between them.
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);                    // smoothstep weights
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

// One texel of the biome mask, clamped to the edge of the map.
fn biome_texel(t: vec2<i32>) -> vec4<f32> {
    let last = vec2<i32>(terrain.dims.xy) - 1;
    return textureLoad(biomes, clamp(t, vec2<i32>(0), last), 0);
}

// Biome weights at any point: bilinear filtering by hand from 4 texels
// (what a sampler would do for us on a filterable texture).
fn biome_at(xz: vec2<f32>) -> vec4<f32> {
    let tc = texel_coord(xz);
    let t0 = vec2<i32>(floor(tc));
    let f = fract(tc);
    let top = mix(biome_texel(t0), biome_texel(t0 + vec2(1, 0)), f.x);
    let bottom = mix(biome_texel(t0 + vec2(0, 1)), biome_texel(t0 + vec2(1, 1)), f.x);
    return mix(top, bottom, f.y);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    let p = in.world_position.xyz;
    let n = normalize(in.world_normal);   // interpolated: re-normalise
    pbr.N = n;

    // Painterly biomes: big soft patches from noise, two shades per biome.
    let k = noise(p.xz / 12.0) * 0.7 + noise(p.xz / 3.0) * 0.3;
    let meadow = mix(vec3(0.36, 0.55, 0.24), vec3(0.58, 0.68, 0.32), k);
    // Fields: a patchwork, one crop per 6x6 plot (hash of the plot index).
    let plot = hash(floor(p.xz / 6.0));
    var field = vec3(0.86, 0.74, 0.34);                 // ripe grain
    if plot < 0.33 { field = vec3(0.55, 0.68, 0.28); }  // young crop
    else if plot < 0.66 { field = vec3(0.72, 0.58, 0.32); } // stubble
    let water_bed = vec3(0.30, 0.42, 0.40);
    let b = biome_at(p.xz);
    var albedo = meadow * b.r + field * b.g + water_bed * b.b;
    albedo /= max(b.r + b.g + b.b, 1e-3);                // weights to sum 1

    // Height and slope: rock on steep or high ground, whatever the biome.
    let slope = 1.0 - n.y;                               // 0 flat, 1 vertical
    let rock = vec3(0.60, 0.56, 0.50);
    let high = smoothstep(6.0, 12.0, p.y);
    albedo = mix(albedo, rock, max(smoothstep(0.25, 0.45, slope), high * 0.8));

    // Culture overlay (M6): the land around each town takes the colour of its
    // culture, fading out toward the edge of its radius. Where two towns'
    // lands overlap, their colours mix by weight. A loop over a uniform array:
    // the count comes from the CPU (dims.z), the array has a fixed size.
    let tinted = u32(terrain.dims.z);
    var tint = vec3<f32>(0.0);
    var weight = 0.0;
    for (var k = 0u; k < min(tinted, 16u); k++) {
        let tw = terrain.towns[k];
        let w = 1.0 - smoothstep(tw.z * 0.45, tw.z, distance(p.xz, tw.xy));
        tint += terrain.cultures[k].rgb * w;
        weight += w;
    }
    if weight > 0.0 {
        albedo = mix(albedo, tint / weight, min(weight, 1.0) * 0.6);
    }

    // Lighting: the shared toon lighting of step 02 (or PBR with the M key).
    let tp = terrain.toon;
    let t = toon_terms(pbr, toon_params(tp));
    var lit = albedo * t.light;
    if tp.debug.y < 0.5 {
        pbr.material.base_color = vec4<f32>(albedo, 1.0);
        lit = apply_pbr_lighting(pbr).rgb;
    }

    var out: FragmentOutput;
    let id = u32(terrain.dims.w);
    if id == 1u {
        out.color = vec4<f32>(vec3<f32>(fract(p.y / 2.0)), 1.0);  // height contours, 2 apart
    } else if id == 2u {
        out.color = vec4<f32>(n * 0.5 + 0.5, 1.0);                // normals
    } else if id == 3u {
        out.color = vec4<f32>(b.rgb, 1.0);                        // biome weights
    } else if id == 4u {
        out.color = vec4<f32>(vec3<f32>(slope * 3.0), 1.0);       // slope (x3)
    } else {
        let fogged = apply_fog(lit, p, tp.fog);              // step 04
        out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(fogged, 1.0));
    }
    return out;
}
