// Shader step 02: soft toon lighting. Explained in docs/shaders/02-toon.md.
//
// We extend Bevy's StandardMaterial: its inputs (albedo, vertex colours, the
// world normal) come in through `pbr_input_from_standard_material`, and we
// replace only the lighting, which lives in the shared module toon_lighting.wesl
// (the terrain uses it too). Bevy still applies fog and tonemapping afterwards.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
};
// A path relative to this file: shaders/toon_lighting.wesl.
import super::toon_lighting::{ToonParams, toon_terms};

// Our uniform block; matches `ToonUniform` in src/toon.rs. Binding 100, because
// the StandardMaterial part of the material uses the low binding numbers.
struct Toon {
    shadow_tint: vec4<f32>, // rgb: colour of the shadow side
    ramp: vec4<f32>,        // x: wrap, y: band edge a, z: band edge b, w: softness
    debug: vec4<f32>,       // x: debug view, y: 1 = toon, 0 = Bevy's PBR lighting
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> toon: Toon;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // Albedo, normal, world position etc. as Bevy computes them for any
    // StandardMaterial (base colour times vertex colour, normal maps, ...).
    let pbr = pbr_input_from_standard_material(in, is_front);
    var out: FragmentOutput;

    // M key: Bevy's own physically based lighting, for comparison.
    if toon.debug.y < 0.5 {
        out.color = main_pass_post_lighting_processing(pbr, apply_pbr_lighting(pbr));
        return out;
    }

    let params = ToonParams(toon.shadow_tint.rgb, toon.ramp.x, toon.ramp.y, toon.ramp.z, toon.ramp.w);
    let t = toon_terms(pbr, params);
    let albedo = pbr.material.base_color.rgb;

    // Debug views (Tab to "toon", then keys 0-4).
    let id = u32(toon.debug.x);
    if id == 1u {
        out.color = vec4<f32>(pbr.N * 0.5 + 0.5, 1.0);            // normals as colour
    } else if id == 2u {
        out.color = vec4<f32>(vec3<f32>(t.ndl * 0.5 + 0.5), 1.0); // 0.5 = terminator
    } else if id == 3u {
        out.color = vec4<f32>(vec3<f32>(t.ramp), 1.0);           // three tones
    } else if id == 4u {
        out.color = vec4<f32>(vec3<f32>(t.shadow), 1.0);         // shadow map term
    } else {
        // Fog and (if needed) tonemapping, as for any Bevy material.
        out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(albedo * t.light, 1.0));
    }
    return out;
}
