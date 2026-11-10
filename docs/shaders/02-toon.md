# 02: Soft toon lighting (normals, dot products, a ramp)

**Concept:** lighting in a fragment shader. The surface normal and the light
direction meet in a dot product (Lambert's cosine law). We then *re-map* that
cosine through a ramp to get the painterly, banded look.

**Where the code is:**
- `crates/game3d/assets/shaders/toon.wesl` and `toon_lighting.wesl`: the shader
  and its lighting module (hot-reload);
- `crates/game3d/src/toon.rs`: `ToonMaterial`, the uniform block, the keys.

**Try it:**
- `cargo run --release -p game3d -- --playground`.
- `M` switches between toon and Bevy's physically based lighting.
- `Tab` makes the number keys control the toon views.

## What it does

It replaces Bevy's physically based diffuse lighting with soft toon shading:
- **Wrap lighting:** the light reaches a little past the terminator (the line
  between lit and dark), so the shadow side stays soft and coloured, not black.
- **Ramp:** the result is quantised into 3 soft bands (shadow, midtone, lit).
- **Shadow tint:** the shadow side is lit by the ambient light tinted cool blue,
  not just darkened.
- **Shadow maps:** Bevy's shadow maps still apply, with the same soft edge.

| toon | Bevy PBR (`M`) | toon at dusk |
| --- | --- | --- |
| ![toon](../images/shader02_toon.jpg) | ![pbr](../images/shader02_pbr.jpg) | ![dusk](../images/shader02_dusk.jpg) |

In the world view: ![world](../images/m1_toon_seed23.jpg)

## How it plugs into Bevy: extending a material

We don't write a material from scratch. `ToonMaterial` is
`ExtendedMaterial<StandardMaterial, ToonExt>`:

- **The `StandardMaterial` half** still does everything around the lighting:
  - base colour times vertex colour (the terrain's biome colours);
  - normals;
  - the shadow-map passes.
- **Our `ToonExt` half** adds one uniform block, at binding 100 (the standard
  half uses the low numbers), and replaces the fragment shader.
- **In the shader:**
  - `pbr_input_from_standard_material(in, is_front)` returns everything the
    standard half computed: albedo, normal, world position;
  - we light it ourselves;
  - `main_pass_post_lighting_processing` then applies fog and tonemapping, as
    for any Bevy material.

That's the usual pattern for a stylised look: keep the engine's inputs and
outputs, and replace the middle.

## The math

For the unit normal `n` and the unit direction to the light `l`:

- **Lambert:** `L = max(n · l, 0)`. This is the cosine of the angle between the
  surface and the light: 1 facing the sun, 0 edge-on.
- **Wrap lighting:** `W = clamp((n · l + w) / (1 + w), 0, 1)`, with `w = 0.4`.
  - At `n · l = −w` (a bit past 90°), W is already 0.
  - So the terminator moves round the back, and the light falls off more gently.
- **Ramp:** `R = ½ smoothstep(a−ε, a+ε, W) + ½ smoothstep(b−ε, b+ε, W)`.
  - `a = 0.28` and `b = 0.62` are the band edges; `ε = 0.05` is the softness.
  - Two soft steps make three tones: 0, ½, 1.
  - (Exercise 1 replaces this with a painted ramp texture.)
- **Shadow map:** `S` is 1 in light and 0 in shadow. It's softened the same way
  (`smoothstep(0.35, 0.65, S)`), so shadow edges match the bands.
- **Colour:** `albedo × (ambient × tint + sun × R × S) × exposure`.
  - On the shadow side only the cool-tinted ambient remains. That's the
    painterly "shadows are a colour" trick.
- **Units:** Bevy's light colours are physical (lux), so we use the same scale
  as its PBR lighting: the sun divided by π, everything times the camera
  exposure. That makes `M` a fair comparison.

## The shader, line by line

The lighting lives in a small module, `toon_lighting.wesl`, because the terrain
(step 03) uses it too. WESL imports it with a path relative to the importing
file: `import super::toon_lighting::{ToonParams, toon_terms};`.

`toon_lighting.wesl`:

```wgsl
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

import super::fog::Fog;

const PI: f32 = 3.14159265;

// The toon uniform block of the materials; matches `ToonUniform` in
// src/toon.rs (the terrain nests the same struct in its own block).
struct Toon {
    shadow_tint: vec4<f32>, // rgb: colour of the shadow side
    ramp: vec4<f32>,        // x: wrap, y: band edge a, z: band edge b, w: softness
    debug: vec4<f32>,       // x: debug view, y: 1 = toon, 0 = Bevy PBR lighting
    fog: Fog,               // step 04
};

// The ramp parameters out of the uniform block.
fn toon_params(t: Toon) -> ToonParams {
    return ToonParams(t.shadow_tint.rgb, t.ramp.x, t.ramp.y, t.ramp.z, t.ramp.w);
}

struct ToonParams {
    shadow_tint: vec3<f32>, // colour of the shadow side
    wrap: f32,              // how far light reaches past 90 degrees (0..1)
    band_a: f32,            // first band edge (shadow -> mid)
    band_b: f32,            // second band edge (mid -> lit)
    softness: f32,          // half-width of a band edge
};

// The intermediate values, so callers can show them as debug views.
struct ToonTerms {
    ndl: f32,         // Lambert's cosine n . l
    ramp: f32,        // after wrap and banding: 0, 0.5 or 1 (soft edges)
    shadow: f32,      // shadow map: 1 lit, 0 in shadow
    light: vec3<f32>, // the light reaching the surface (multiply by albedo)
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
    t.light = (ambient + sun.color.rgb / PI * lit) * view.exposure;
    return t;
}
```

`toon.wesl`, the material's fragment shader:

```wgsl
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
        // Step 04: our fog. Then Bevy's post-lighting step (tonemapping if
        // needed; its own fog is off, the camera has no DistanceFog).
        let fogged = apply_fog(albedo * t.light, pbr.world_position.xyz, toon.fog);
        out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(fogged, 1.0));
    }
    return out;
}
```

```mermaid
flowchart LR
  V["Vertex stage<br/>position, normal<br/>to world + clip"] --> RS["Rasteriser<br/>interpolates the normal"]
  RS --> F["Fragment stage<br/>n . l, wrap, ramp, tint"]
  L[("Light uniforms<br/>sun dir, colour, ambient")] --> F
  SM[("Shadow map")] --> F
  M[("Material uniforms<br/>Toon + StandardMaterial")] --> F
  F --> P["Fog, tonemapping"] --> O["Colour target"]
```

**Why the normal looks smooth on a sphere made of flat triangles:** the vertex
stage outputs one normal per vertex, and the rasteriser *interpolates* it across
each triangle. The fragment shader then sees a smoothly varying normal.
- An interpolated unit vector is slightly shorter than 1, so it must be
  normalised again. Bevy's `pbr_input_from_standard_material` does that for us
  (`pbr.N`). In a fragment shader of your own, call `normalize` first.
- The cube has separate vertices per face, each with that face's normal, so its
  faces stay flat.

## Debug views

`Tab` until the toon line is marked, then `0`–`4`:

| 1 normals | 2 n·l | 3 ramp | 4 shadow map |
| --- | --- | --- | --- |
| ![](../images/shader02_view1.jpg) | ![](../images/shader02_view2.jpg) | ![](../images/shader02_view3.jpg) | ![](../images/shader02_view4.jpg) |

- **1, normals:** `n * 0.5 + 0.5`. Faces pointing up are green, +x red, +z
  blue. The ground is flat green; the sphere shows every direction.
- **2, n·l:** `ndl * 0.5 + 0.5`, so mid-grey marks the terminator. It's a smooth
  gradient, before any ramp.
- **3, ramp:** exactly three tones. Compare with view 2 to see where the band
  edges `a` and `b` fall.
- **4, shadow map:** white in light, black in cast shadows. The ground shows the
  shapes' shadows; the shapes' own dark sides are *not* black here (that's
  `n · l`, not the shadow map).

## Exercises

Edit `toon.wesl` with the playground running, and save after each change.

1. **Band edges:** move `a` and `b` (in `ToonUniform::default`, or hard-code
   them in the shader) and watch view 3.
   - Then make it two bands instead of three.
   - Then four: add a third `smoothstep` and change the weights to ⅓.
2. **Sky fill light:** the shadow side only gets flat ambient light. Add a fill
   from the sky that depends on how much the surface faces up:
   `fill = mix(ground_colour, sky_colour, n.y * 0.5 + 0.5)`.
   Add it to `ambient`, and watch the undersides of the torus turn warmer than
   the tops.
3. **Ramp texture (harder; needs Rust):** replace the two `smoothstep`s with a
   64×1 gradient image sampled at `W`.
   - Add `#[texture(101)] #[sampler(102)] ramp: Handle<Image>` to `ToonExt`, and
     the matching bindings in the shader.
   - `textureSample(ramp_tex, ramp_smp, vec2(w, 0.5))` returns the colour.
   - Paint your own ramp and see the mood change.
