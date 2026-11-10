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
// Paths relative to this file: shaders/toon_lighting.wesl and shaders/fog.wesl.
import super::toon_lighting::{Toon, toon_params, toon_terms};
import super::fog::apply_fog;

// Our uniform block (struct `Toon` in toon_lighting.wesl, `ToonUniform` in
// src/toon.rs). Binding 100, because the StandardMaterial part of the material
// uses the low binding numbers.
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

    let t = toon_terms(pbr, toon_params(toon));
    let albedo = pbr.material.base_color.rgb;

    // Debug views (Tab to "toon", then keys 0-6).
    let id = u32(toon.debug.x);
    if id == 1u {
        out.color = vec4<f32>(pbr.N * 0.5 + 0.5, 1.0);            // normals as colour
    } else if id == 2u {
        out.color = vec4<f32>(vec3<f32>(t.ndl * 0.5 + 0.5), 1.0); // 0.5 = terminator
    } else if id == 3u {
        out.color = vec4<f32>(vec3<f32>(t.ramp), 1.0);           // three tones
    } else if id == 4u {
        out.color = vec4<f32>(vec3<f32>(t.shadow), 1.0);         // shadow map term
    } else if id == 5u {
        out.color = vec4<f32>(vec3<f32>(t.fresnel), 1.0);        // step 05: fresnel
    } else if id == 6u {
        out.color = vec4<f32>(vec3<f32>(t.ndv), 1.0);            // step 05: n . v
    } else {
        // Step 04: our fog. Then Bevy's post-lighting step (tonemapping if
        // needed; its own fog is off, the camera has no DistanceFog).
        let fogged = apply_fog(albedo * t.light, pbr.world_position.xyz, toon.fog);
        out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(fogged, 1.0));
    }
    return out;
}
