// Shared toon lighting (step 02), used by toon.wesl and terrain.wesl.
//
// A WESL module: other shaders import its functions with
// `import super::toon_lighting::{ToonParams, toon_terms};` (a path relative to
// the importing file). See docs/shaders/02-toon.md for the maths.

import bevy_pbr::render::{
    pbr_types::PbrInput,
    mesh_view_bindings::{lights, view},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    shadows::fetch_directional_shadow,
};

const PI: f32 = 3.14159265;

// The toon uniform block of the materials; matches `ToonUniform` in
// src/toon.rs (the terrain nests the same struct in its own block).
struct Toon {
    shadow_tint: vec4<f32>, // rgb: colour of the shadow side
    ramp: vec4<f32>,        // x: wrap, y: band edge a, z: band edge b, w: softness
    debug: vec4<f32>,       // x: debug view, y: 1 = toon, 0 = Bevy PBR lighting
    rim: vec4<f32>,         // step 05: x strength, y power (fresnel exponent);
                            // step 11: z 1 = foliage (turns with the seasons)
};

// The ramp parameters out of the uniform block.
fn toon_params(t: Toon) -> ToonParams {
    return ToonParams(t.shadow_tint.rgb, t.ramp.x, t.ramp.y, t.ramp.z, t.ramp.w, t.rim.x, t.rim.y);
}

struct ToonParams {
    shadow_tint: vec3<f32>, // colour of the shadow side
    wrap: f32,              // how far light reaches past 90 degrees (0..1)
    band_a: f32,            // first band edge (shadow -> mid)
    band_b: f32,            // second band edge (mid -> lit)
    softness: f32,          // half-width of a band edge
    rim_strength: f32,      // step 05: how bright the rim light is (0 = off)
    rim_power: f32,         // step 05: higher = thinner rim
};

// The intermediate values, so callers can show them as debug views.
struct ToonTerms {
    ndl: f32,         // Lambert's cosine n . l
    ramp: f32,        // after wrap and banding: 0, 0.5 or 1 (soft edges)
    shadow: f32,      // shadow map: 1 lit, 0 in shadow
    light: vec3<f32>, // the light reaching the surface (multiply by albedo)
    ndv: f32,         // step 05: n . v, how much the surface faces the camera
    fresnel: f32,     // step 05: (1 - n . v)^power, bright at grazing angles
};

fn toon_terms(pbr: PbrInput, p: ToonParams) -> ToonTerms {
    var t: ToonTerms;
    let n = pbr.N;

    // The sun: `direction_to_light` points from the surface toward the light.
    let sun = lights.directional_lights[0];
    t.ndl = dot(n, sun.direction_to_light);

    // Wrap lighting, then two soft steps make three bands.
    let w = clamp((t.ndl + p.wrap) / (1.0 + p.wrap), 0.0, 1.0);
    let e = p.softness;
    t.ramp = 0.5 * smoothstep(p.band_a - e, p.band_a + e, w)
           + 0.5 * smoothstep(p.band_b - e, p.band_b + e, w);

    // Shadow map, if this mesh receives shadows and the sun casts them. The
    // cascade is picked by view-space depth (row z of the view matrix).
    t.shadow = 1.0;
    if (pbr.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
        && (sun.flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
        let view_z = dot(vec4<f32>(
            view.view_from_world[0].z,
            view.view_from_world[1].z,
            view.view_from_world[2].z,
            view.view_from_world[3].z,
        ), pbr.world_position);
        t.shadow = fetch_directional_shadow(0u, pbr.world_position, pbr.world_normal, view_z, pbr.frag_coord.xy);
    }
    let lit = t.ramp * smoothstep(0.35, 0.65, t.shadow);

    // Shadow side: ambient light tinted cool. Lit side: plus the sun. Same
    // units as Bevy's PBR (sun / pi, times the camera exposure).
    let ambient = lights.ambient_color.rgb * p.shadow_tint;
    let sunlight = sun.color.rgb / PI;

    // Step 05, rim light: v points from the surface to the camera. n . v is 1
    // where the surface faces us and 0 at the silhouette; the fresnel term
    // turns that into a thin bright edge. Only on the sunlit side (a rim that
    // ignores the light looks like a glow).
    let v = normalize(view.world_position - pbr.world_position.xyz);
    t.ndv = max(dot(n, v), 0.0);
    t.fresnel = pow(1.0 - t.ndv, p.rim_power);
    let rim = t.fresnel * smoothstep(-0.2, 0.4, t.ndl) * p.rim_strength;

    t.light = (ambient + sunlight * (lit + rim)) * view.exposure;
    return t;
}
