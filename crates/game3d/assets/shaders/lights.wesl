// Step 12: lights in the towns at night (docs/shaders/12-town-lights.md).
//
// Baked buildings carry their colour as a vertex colour (bake.rs). Its alpha,
// which an opaque material doesn't need, marks the parts that light up:
//   1.0   ordinary surface
//   0.25  window: dark glass by day, warm light from dusk; fewer are lit as
//         the night goes on
//   0.5   lantern: lit from dusk, a slight flicker
//   0.75  fire (forge, oven, furnace mouth): always a little, bright at night,
//         flickering
// The glow is added to the lit colour, brighter than white, so the bloom pass
// gives it a halo. The glow itself lights nothing; the lamps nearest the
// camera also light their surroundings (`lamp_term`, from src/lights.rs).

import bevy_pbr::render::mesh_view_bindings::globals;
import super::fog::Fog;

// One light near the camera (src/lights.rs): pos xyz and radius; colour times
// brightness (0 by day), and a random number for its flicker.
struct Lamp {
    pos: vec4<f32>,
    color: vec4<f32>,
};

// The buffer every material shares (src/fog.rs, FOG_BUFFER): this frame's fog,
// then the lamps nearest the camera.
struct Env {
    fog: Fog,
    lamps_info: vec4<f32>, // x: how many lamps are in use
    lamps: array<Lamp, 24>,
};

// The light one lamp throws on a surface at p facing n: a soft pool, falling
// to nothing at its radius, wrapped round so walls facing away get a little.
fn lamp_term(l: Lamp, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let d = l.pos.xyz - p;
    let dist = length(d);
    let r = l.pos.w;
    if dist >= r {
        return vec3<f32>(0.0);
    }
    let fall = (1.0 - dist / r) * (1.0 - dist / r);
    let facing = clamp(dot(n, d / max(dist, 1e-4)) * 0.6 + 0.4, 0.0, 1.0);
    let flicker = 0.88 + 0.12 * sin(globals.time * 8.0 + l.color.w * 50.0);
    return l.color.rgb * fall * facing * flicker;
}

// 0..1 pseudo-random from a cell of the ground.
fn cell_hash(c: vec2<f32>) -> f32 {
    return fract(sin(dot(c, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// How dark it is: 0 by day, 1 once the sun is well below the horizon.
fn night(fog: Fog) -> f32 {
    return 1.0 - smoothstep(-0.10, 0.06, fog.sun_dir.y);
}

// The light a marked surface gives off (linear HDR), 0 for the rest.
fn town_glow(marker: f32, p: vec3<f32>, fog: Fog) -> vec3<f32> {
    if marker > 0.9 {
        return vec3<f32>(0.0);
    }
    let dark = night(fog);
    // One random value per house-sized cell of the ground: all windows of a
    // house share it (mostly), so a house is lit or dark as a whole.
    let h = cell_hash(floor(p.xz * 0.6));
    let t = globals.time;
    if marker < 0.4 {
        // Windows. Deeper into the night (sun lower) fewer stay lit.
        let late = smoothstep(-0.15, -0.6, fog.sun_dir.y);
        let lit = step(mix(0.15, 0.7, late), h);
        let breathe = 0.93 + 0.07 * sin(t * 1.3 + h * 40.0);
        return vec3<f32>(1.0, 0.62, 0.28) * 2.4 * dark * lit * breathe;
    }
    let flicker = 0.78 + 0.22 * sin(t * 9.0 + h * 50.0) * sin(t * 4.3 + h * 17.0);
    if marker < 0.6 {
        // Lanterns.
        return vec3<f32>(1.0, 0.78, 0.42) * 3.0 * dark * (0.9 + 0.1 * flicker);
    }
    // Fires: a glow by day, a blaze at night.
    return vec3<f32>(1.0, 0.42, 0.12) * mix(0.8, 3.6, dark) * flicker;
}
