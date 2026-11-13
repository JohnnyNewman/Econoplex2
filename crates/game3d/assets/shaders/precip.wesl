// Shader step 11: rain and snow. Explained in docs/shaders/11-day-night-weather.md.
//
// Every quad of the mesh is one drop. Its four vertices hold the same random
// point in a unit cube (the position) and which corner they are (the UV). The
// vertex shader moves the point with time and wraps it into a box in front of
// the camera, so a few thousand drops look like rain everywhere.

import bevy_pbr::render::{
    forward_io::{Vertex, decompress_vertex},
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
};

// Matches `PrecipUniform` in src/precip.rs.
struct Precip {
    centre: vec4<f32>, // xyz: centre of the box, w: its edge length
    fall: vec4<f32>,   // xyz: velocity, w: streak length (seconds of fall)
    look: vec4<f32>,   // x: share of drops shown, y: snow, z: width, w: brightness
};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> precip: Precip;

struct PrecipOut {
    @builtin(position) position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) fade: f32,
};

fn hash(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

@vertex
fn vertex(vertex_in: Vertex) -> PrecipOut {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let seed = v.position;           // random point in [0, 1)^3, same for 4 corners
    var corner = vec2<f32>(0.0);
@if(VERTEX_UVS_A)
    corner = v.uv;
    var out: PrecipOut;
    out.corner = corner;

    // Only a share of the drops shows: light rain, few drops.
    let r = hash(seed);
    if r > precip.look.x {
        out.position = vec4<f32>(0.0, 0.0, -2.0, 1.0); // behind the camera: clipped
        out.fade = 0.0;
        return out;
    }

    let size = precip.centre.w;
    let snow = precip.look.y;
    let t = globals.time;
    // Each drop falls at its own speed (90-110 %); snow flutters sideways.
    let vel = precip.fall.xyz * (0.9 + 0.2 * r);
    let flutter = vec3<f32>(sin(t * 1.7 + r * 30.0), 0.0, cos(t * 1.3 + r * 50.0)) * 0.6 * snow;
    // Where the drop would be if it had fallen since time 0, in world units...
    let travelled = seed * size + vel * t + flutter;
    // ...wrapped into the box around the centre: fract() of the offset in box
    // units. A drop that leaves the box at the bottom comes back at the top.
    let rel = (fract((travelled - precip.centre.xyz) / size + 0.5) - 0.5) * size;
    let p = precip.centre.xyz + rel;

    // Fade near the walls of the box (no popping when a drop wraps) and very
    // close to the camera (no huge drops across the screen).
    let edge = 1.0 - smoothstep(0.3, 0.5, max(max(abs(rel.x), abs(rel.y)), abs(rel.z)) / size);
    let near = smoothstep(size * 0.12, size * 0.3, length(p - view.world_position));
    out.fade = edge * near;

    // The quad: rain is a streak along its velocity, snow a square facing the
    // camera. `across` is at right angles to both the streak and the view ray.
    let to_cam = normalize(view.world_position - p);
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let w = precip.look.z;
    var world: vec3<f32>;
    if snow > 0.5 {
        world = p + right * corner.x * w + up * (corner.y * 2.0 - 1.0) * w;
    } else {
        let along = vel * precip.fall.w;
        let across = normalize(cross(normalize(along), to_cam));
        world = p + across * corner.x * w + along * (corner.y - 0.5);
    }
    out.position = position_world_to_clip(world);
    return out;
}

@fragment
fn fragment(in: PrecipOut) -> @location(0) vec4<f32> {
    let snow = precip.look.y;
    let bright = precip.look.w;
    var alpha: f32;
    var rgb: vec3<f32>;
    if snow > 0.5 {
        // A soft round flake.
        let d = length(vec2<f32>(in.corner.x, in.corner.y * 2.0 - 1.0));
        alpha = smoothstep(1.0, 0.3, d) * 0.9;
        rgb = vec3<f32>(0.95, 0.97, 1.0) * bright;
    } else {
        // A thin streak, brightest in its middle.
        alpha = (1.0 - abs(in.corner.x)) * sin(in.corner.y * 3.14159) * 0.35;
        rgb = vec3<f32>(0.75, 0.8, 0.88) * bright;
    }
    return vec4<f32>(rgb, alpha * in.fade);
}
