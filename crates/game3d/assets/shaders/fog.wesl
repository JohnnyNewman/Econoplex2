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
    ground: vec4<f32>,    // step 11: x snow cover, y wet, z autumn, w winter
    wind: vec4<f32>,      // step 11: xy wind direction, z strength 0..1, w spring
};

// ---------------------------------------------------------------- step 11
// Seasons and weather on surfaces. The fog block travels to every material
// anyway, so it also carries the state of the ground.

fn season_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(41.3, 289.1))) * 15731.743);
}

// Smooth value noise, so seasonal patches have soft edges.
fn season_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(season_hash(i), season_hash(i + vec2(1.0, 0.0)), u.x),
               mix(season_hash(i + vec2(0.0, 1.0)), season_hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

// A surface's colour in this season and weather.
// - `foliage`: what kind of plant the surface is: 0 none (stone, wood, roofs),
//   0.5 grass and meadow (turns straw-gold in autumn, brown in winter),
//   1 tree crowns (most stay green: they are conifers; about a third are
//   larches and broadleaves that turn orange, gold or red, then go bare);
// - snow settles on surfaces that face up (normal `n`), patchy at the edge of
//   the cover; never on the water (below y = -0.5);
// - wet surfaces are darker.
fn seasonal(albedo: vec3<f32>, n: vec3<f32>, p: vec3<f32>, foliage: f32, fog: Fog) -> vec3<f32> {
    var c = albedo;
    let g = fog.ground;
    let lum = dot(c, vec3<f32>(0.3, 0.59, 0.11));
    let grass = clamp(foliage * 2.0, 0.0, 1.0) * (1.0 - step(0.75, foliage));
    let tree = step(0.75, foliage);
    // Spring: a little more saturated and yellow-green.
    c = mix(c, c * vec3<f32>(1.0, 1.12, 0.8), fog.wind.w * foliage * 0.6);

    // Grass: straw-gold in autumn, dull brown in winter, in soft patches.
    let field = season_noise(p.xz / 7.0);
    let straw = mix(vec3<f32>(0.62, 0.55, 0.28), vec3<f32>(0.70, 0.58, 0.30), field) * (lum * 0.9 + 0.35);
    c = mix(c, straw, g.z * grass * (0.35 + 0.35 * field));
    let brown = mix(vec3<f32>(0.46, 0.42, 0.30), vec3<f32>(0.40, 0.38, 0.30), field) * (lum * 0.8 + 0.4);
    c = mix(c, brown, g.w * grass * 0.7);

    // Trees: which ones are broadleaves follows soft noise (stands of them).
    let kind = season_noise(p.xz / 5.0 + 11.0);
    let leafy = smoothstep(0.58, 0.68, kind) * tree;
    let shade = season_hash(floor(p.xz / 2.0));
    var autumn_col = mix(vec3<f32>(0.85, 0.40, 0.10), vec3<f32>(0.80, 0.62, 0.14), shade);
    if shade > 0.8 { autumn_col = vec3<f32>(0.66, 0.18, 0.08); }
    c = mix(c, autumn_col * (lum * 1.6 + 0.15), g.z * leafy);
    let bare = vec3<f32>(0.36, 0.30, 0.25) * (lum * 0.8 + 0.35);
    c = mix(c, bare, g.w * leafy * 0.85);
    // Conifers just darken a little in winter.
    c *= 1.0 - 0.15 * g.w * tree * (1.0 - leafy);

    // Wet: darker.
    c *= 1.0 - 0.3 * g.y;
    // Snow on up-facing surfaces: the cover grows in from the flattest ground,
    // with noisy edges.
    let up = smoothstep(0.45, 0.8, n.y);
    let edge = season_noise(p.xz * 1.3) * 0.35;
    let snow = smoothstep(0.0, 0.25, g.x * 1.25 - edge - (1.0 - up)) * step(-0.5, p.y);
    c = mix(c, vec3<f32>(0.92, 0.94, 0.98), snow);
    return c;
}

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
