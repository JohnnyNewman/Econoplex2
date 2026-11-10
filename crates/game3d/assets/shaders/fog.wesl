// Shader step 04: distance and height fog. Explained in docs/shaders/04-fog.md.
//
// A WESL module used by the toon and terrain shaders: they call `apply_fog`
// with the lit colour and the fragment's world position, after lighting and
// before tonemapping.

import bevy_pbr::render::mesh_view_bindings::view;

// Matches `FogUniform` in src/fog.rs.
struct Fog {
    color: vec4<f32>,     // rgb: fog colour (the sky's horizon); a: 1 on, 0 off
    sun_dir: vec4<f32>,   // xyz: toward the sun; w: exponent of the sun glow
    sun_color: vec4<f32>, // rgb: colour toward the sun; a: glow strength
    params: vec4<f32>,    // x: distance density, y: height density at base,
                          // z: height falloff, w: base height
    debug: vec4<f32>,     // x: debug view
};

// The two parts of the fog and its colour, kept for the debug views.
struct FogTerms {
    distance: f32, // 0..1 from distance alone
    height: f32,   // 0..1 from height fog alone
    amount: f32,   // 0..1 combined
    color: vec3<f32>,
};

fn fog_terms(world_pos: vec3<f32>, fog: Fog) -> FogTerms {
    var t: FogTerms;
    let cam = view.world_position;
    let to_p = world_pos - cam;
    let d = length(to_p);                  // distance camera -> fragment
    let ray = to_p / max(d, 1e-4);         // unit view ray

    // Distance fog: Beer-Lambert, transmittance exp(-density * d).
    t.distance = 1.0 - exp(-fog.params.x * d);

    // Height fog: density rho(y) = rho0 * exp(-(y - h0) / H) along the ray.
    // Integrating rho over the ray from the camera (height yc) to the point
    // (height yp) has a closed form; when the ray is level it is just rho * d.
    let rho0 = fog.params.y;
    let H = fog.params.z;
    let h0 = fog.params.w;
    let yc = cam.y;
    let yp = world_pos.y;
    let ec = exp(-(yc - h0) / H);
    let ep = exp(-(yp - h0) / H);
    var tau: f32;
    if abs(yp - yc) > 1e-3 {
        tau = rho0 * d * H * (ec - ep) / (yp - yc);
    } else {
        tau = rho0 * d * ec;
    }
    t.height = 1.0 - exp(-max(tau, 0.0));

    // Combined: light has to get through both, so the transmittances multiply:
    // (1 - amount) = (1 - distance) * (1 - height).
    t.amount = (1.0 - (1.0 - t.distance) * (1.0 - t.height)) * fog.color.a;

    // In-scattering: looking toward the sun, the haze glows in the sun's colour.
    let toward_sun = pow(max(dot(ray, fog.sun_dir.xyz), 0.0), fog.sun_dir.w);
    t.color = mix(fog.color.rgb, fog.sun_color.rgb, toward_sun * fog.sun_color.a);
    return t;
}

// The lit colour seen through the fog. Debug views 1-4 replace it.
fn apply_fog(lit: vec3<f32>, world_pos: vec3<f32>, fog: Fog) -> vec3<f32> {
    let t = fog_terms(world_pos, fog);
    let id = u32(fog.debug.x);
    if id == 1u { return vec3<f32>(t.amount); }
    if id == 2u { return vec3<f32>(t.distance); }
    if id == 3u { return vec3<f32>(t.height); }
    if id == 4u { return t.color; }
    return mix(lit, t.color, t.amount);
}
