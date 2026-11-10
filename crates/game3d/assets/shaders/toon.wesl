// Shader step 02: soft toon lighting. Explained line by line in docs/shaders/02-toon.md.
//
// We extend Bevy's StandardMaterial: its inputs (albedo, vertex colours, the
// world normal) come in through `pbr_input_from_standard_material`, and we
// replace only the lighting. Bevy still applies fog and tonemapping afterwards.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{lights, view},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    shadows::fetch_directional_shadow,
};

// Our uniform block; matches `ToonUniform` in src/toon.rs. Binding 100, because
// the StandardMaterial part of the material uses the low binding numbers.
struct Toon {
    shadow_tint: vec4<f32>, // rgb: colour of the shadow side
    ramp: vec4<f32>,        // x: wrap, y: band edge a, z: band edge b, w: softness
    debug: vec4<f32>,       // x: debug view, y: 1 = toon, 0 = Bevy's PBR lighting
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> toon: Toon;

const PI: f32 = 3.14159265;

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

    let n = pbr.N;                           // unit surface normal, world space
    let albedo = pbr.material.base_color.rgb;

    // The first directional light: the sun. `direction_to_light` points from
    // the surface toward the light, so n . l > 0 means "facing the sun".
    let sun = lights.directional_lights[0];
    let l = sun.direction_to_light;
    let ndl = dot(n, l);                     // Lambert's cosine

    // Wrap lighting: shift and rescale, so the light reaches past 90 degrees
    // (ndl = -wrap already gives 0) and the terminator moves round the back.
    let wrap = toon.ramp.x;
    let w = clamp((ndl + wrap) / (1.0 + wrap), 0.0, 1.0);

    // The ramp: two soft steps give three bands (0, 0.5, 1).
    let a = toon.ramp.y;
    let b = toon.ramp.z;
    let e = toon.ramp.w;
    let ramp = 0.5 * smoothstep(a - e, a + e, w) + 0.5 * smoothstep(b - e, b + e, w);

    // Shadow maps: 1 = lit, 0 = in shadow. The cascade is picked by view-space
    // depth, which we get from the world position and the view matrix row z.
    var shadow = 1.0;
    if (pbr.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
        && (sun.flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
        let view_z = dot(vec4<f32>(
            view.view_from_world[0].z,
            view.view_from_world[1].z,
            view.view_from_world[2].z,
            view.view_from_world[3].z,
        ), pbr.world_position);
        shadow = fetch_directional_shadow(0u, pbr.world_position, pbr.world_normal, view_z, pbr.frag_coord.xy);
    }
    // The shadow edge gets the same soft step as the bands.
    let lit = ramp * smoothstep(0.35, 0.65, shadow);

    // Light: the shadow side gets the ambient light tinted cool; the lit side
    // adds the sun. Bevy's light colours are physical (lux), so we use the same
    // scale as its PBR lighting: sun / pi, everything times the camera exposure.
    let ambient = lights.ambient_color.rgb * toon.shadow_tint.rgb;
    let sunlight = sun.color.rgb / PI;
    let light = (ambient + sunlight * lit) * view.exposure;

    // Debug views (Tab to "toon", then keys 0-4).
    let id = u32(toon.debug.x);
    if id == 1u {
        out.color = vec4<f32>(n * 0.5 + 0.5, 1.0);           // normals as colour
    } else if id == 2u {
        out.color = vec4<f32>(vec3<f32>(ndl * 0.5 + 0.5), 1.0); // 0.5 = terminator
    } else if id == 3u {
        out.color = vec4<f32>(vec3<f32>(ramp), 1.0);         // three tones
    } else if id == 4u {
        out.color = vec4<f32>(vec3<f32>(shadow), 1.0);       // shadow map term
    } else {
        // Fog and (if needed) tonemapping, as for any Bevy material.
        out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(albedo * light, 1.0));
    }
    return out;
}
