// Shared by terrain.wesl (the main pass) and terrain_prepass.wesl (the shadow
// pass): the terrain's uniform block, its textures, and the height lookup.
// Both passes must lift the grid in exactly the same way, or the shadows
// would not match the ground they fall on.

import super::toon_lighting::Toon;

// Matches `TerrainUniform` (with the nested `ToonUniform`) in src/terrain.rs.
struct Terrain {
    grid: vec4<f32>, // x, z of texel (0,0); distance between texels; height scale
    dims: vec4<f32>, // texels across (i, +x), texels deep (j, -z); towns tinted; debug view
    toon: Toon,
    towns: array<vec4<f32>, 16>,    // culture overlay: x, z, radius per town
    cultures: array<vec4<f32>, 16>, // culture overlay: colour per town
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> terrain: Terrain;
// Heights, read texel by texel with textureLoad (32-bit floats aren't
// filterable everywhere, and we don't need filtering at the grid's vertices).
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var heights: texture_2d<f32>;
// Biome weights: r meadow, g fields, b water.
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var biomes: texture_2d<f32>;

// ---------------------------------------------------------------- helpers

// Which texel a world position falls on, as a continuous coordinate:
// i grows with +x, j grows with -z (north in the simulation's map).
fn texel_coord(xz: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(xz.x - terrain.grid.x, terrain.grid.y - xz.y) / terrain.grid.z;
}

// Height at an integer texel, clamped to the edge of the map.
fn height_at(t: vec2<i32>) -> f32 {
    let last = vec2<i32>(terrain.dims.xy) - 1;
    let c = clamp(t, vec2<i32>(0), last);
    return textureLoad(heights, c, 0).r * terrain.grid.w;
}

// The normal from central differences: the slope along i and along j, each
// over two texels. Along j the world goes toward -z, hence the sign flip.
fn normal_at(t: vec2<i32>) -> vec3<f32> {
    let dhdi = height_at(t + vec2(1, 0)) - height_at(t - vec2(1, 0));
    let dhdj = height_at(t + vec2(0, 1)) - height_at(t - vec2(0, 1));
    let span = 2.0 * terrain.grid.z;                 // world distance of 2 texels
    // Surface y = h(x, z); its normal is (-dh/dx, 1, -dh/dz), normalised.
    // dh/dx = dhdi / span; dh/dz = -dhdj / span.
    return normalize(vec3<f32>(-dhdi / span, 1.0, dhdj / span));
}

// A flat grid vertex (world position, y = 0) lifted to the terrain height.
fn displaced(flat: vec4<f32>) -> vec4<f32> {
    let t = vec2<i32>(round(texel_coord(flat.xz)));
    return vec4<f32>(flat.x, height_at(t), flat.z, 1.0);
}
