// M4: people as one crowd; step 09: procedural animation; step 10: level of
// detail. See docs/shaders/07-crowds-compute.md,
// docs/shaders/09-procedural-animation.md and docs/shaders/10-lod.md.
//
// Like the grass (step 06): one mesh holds many copies of a small figure, and
// the vertex shader reads each figure's data from a storage buffer by
// `(vertex_index - first_vertex) / VERTS_PER_PERSON`. The compute pass
// (walk.wesl) writes that buffer every frame.
//
// Step 09: the figure is rigid parts (body, head, two legs, two arms), and the
// vertex number tells which part a vertex belongs to. Each limb turns about its
// joint (hip or shoulder) by an angle computed here from the person's state:
// walkers swing legs and arms with their stride phase; people standing at work
// make their trade's motion (a scythe sweep, a hammer blow, an axe chop, busy
// hands); soldiers stand guard; the idle look around. No skeleton, no
// animation clips: a few sines per vertex.
//
// Step 10: each person has 186 vertices: the 180 of the figure, then 6 for a
// card (two triangles). Close up the figure is drawn and the card collapsed to
// nothing; in the middle distance the other way round, and the fragment shader
// draws a silhouette on the card with a distance function; far away, neither.
// Around each switch both are drawn, each dithered by its share, so the change
// is a soft cross-fade instead of a pop.

import bevy_pbr::render::{
    mesh_functions,
    mesh_bindings::mesh,
    forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex},
    mesh_view_bindings::{globals, view},
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
};
import super::toon_lighting::{Toon, toon_params, toon_terms};
import super::fog::{Fog, apply_fog};

// One person; matches the 12 floats per person in src/crowd.rs and walk.wesl.
struct Person {
    pos: vec3<f32>,    // feet, world space
    heading: f32,      // direction of travel (radians around y)
    color: vec3<f32>,
    size: f32,         // 1 adult, 0.65 child; negative = walking; 0 = nobody
    motion: vec4<f32>, // x: work motion, y: stride phase, z: walking blend
};

struct Crowd {
    toon: Toon,
    lod: vec4<f32>, // x: figure-to-card distance, y: far cut-off, z: fade band, w: tint by level
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> crowd: Crowd;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> people: array<Person>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var<storage, read> fog_state: Fog; // the shared fog (src/fog.rs, FOG_BUFFER)

const FIGURE_VERTS: u32 = 180u;
const VERTS_PER_PERSON: u32 = 186u;
// The card's half width and height, in figure units (the figure is 0.77 tall).
const CARD: vec2<f32> = vec2<f32>(0.25, 0.8);
// Joints (figure space: x across, y up, z forward), see crowd.rs `figure`.
const HIP_Y: f32 = 0.32;
const SHOULDER_Y: f32 = 0.53;

// Work motions (crowd.rs `Motion`).
const SCYTHE: u32 = 1u;
const HAMMER: u32 = 2u;
const AXE: u32 = 3u;
const CRAFT: u32 = 4u;
const GUARD: u32 = 5u;

// Turn about the x axis so that a limb hanging down swings forward (+z) for a
// positive angle.
fn swing(v: vec3<f32>, a: f32) -> vec3<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec3<f32>(v.x, v.y * c + v.z * s, -v.y * s + v.z * c);
}

// Turn about the y axis (a heading, or the upper body's twist).
fn turn(v: vec3<f32>, a: f32) -> vec3<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec3<f32>(v.x * c + v.z * s, v.y, -v.x * s + v.z * c);
}

// A pose: how far each arm swings forward, the upper body's twist and its lean.
struct Pose {
    arm_l: f32,
    arm_r: f32,
    twist: f32,
    lean: f32,
};

// The pose of someone standing, by what they do. `t` is their own clock.
fn work_pose(kind: u32, t: f32) -> Pose {
    var p = Pose(0.0, 0.0, 0.0, 0.0);
    switch kind {
        case SCYTHE: {
            // Arms forward holding the handle; the upper body sweeps side to side.
            p.arm_l = 1.1;
            p.arm_r = 1.1;
            p.twist = sin(t * 2.2) * 0.55;
            p.lean = 0.12;
        }
        case HAMMER, AXE: {
            // A slow lift and a quick blow: up over 65 % of the cycle, down fast.
            let u = fract(t * 0.9);
            let lift = smoothstep(0.0, 0.65, u) * (1.0 - smoothstep(0.72, 0.8, u));
            p.arm_l = 0.4 + 2.4 * lift;
            p.arm_r = p.arm_l;
            p.lean = 0.2 * (1.0 - lift);
            if kind == AXE {
                p.twist = 0.5 * lift - 0.25; // a chop from the side
            }
        }
        case CRAFT: {
            // Hands busy in front, alternating.
            let busy = sin(t * 9.0) * 0.2;
            p.arm_l = 1.0 + busy;
            p.arm_r = 1.0 - busy;
            p.lean = 0.1;
        }
        case GUARD: {
            p.arm_r = 0.5; // a spear held up front
            p.twist = sin(t * 0.7) * 0.15;
        }
        default: {
            // Idle: breathing arms and a slow look around.
            p.arm_l = sin(t * 1.4) * 0.05;
            p.arm_r = -p.arm_l;
            p.twist = sin(t * 0.3) * 0.3;
        }
    }
    return p;
}

// How much of the figure (x) and of the card (y) to draw at distance `d`:
// 1 fully, 0 not at all, in between while cross-fading.
fn lod_shares(d: f32) -> vec2<f32> {
    let lod = crowd.lod;
    let figure = 1.0 - smoothstep(lod.x - lod.z * 0.5, lod.x + lod.z * 0.5, d);
    let far = 1.0 - smoothstep(lod.y - lod.z * 4.0, lod.y, d);
    return vec2<f32>(figure, min(1.0 - figure, far));
}

// A vertex that draws nothing: all three corners of its triangles land on the
// same point outside the screen, a triangle of zero area that the rasterizer
// drops.
fn nothing() -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(10.0, 10.0, 10.0, 1.0);
    out.world_position = vec4<f32>(0.0);
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    return out;
}

// One corner of the card: a quad standing on the person's feet and turned
// about the vertical toward the camera ("cylindrical billboard").
fn card_vertex(p: Person, index: u32, corner: u32, instance: u32) -> VertexOutput {
    let size = abs(p.size);
    // The two triangles, counter-clockwise as seen from the camera; (x, y)
    // with x across (-1..1) and y up (0..1).
    var quad = array<vec2<f32>, 6>(
        vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
        vec2(-1.0, 0.0), vec2(1.0, 1.0), vec2(-1.0, 1.0),
    );
    let c = quad[corner];
    let to_cam = normalize(vec3<f32>(view.world_position.x - p.pos.x, 0.0, view.world_position.z - p.pos.z));
    let right = vec3<f32>(to_cam.z, 0.0, -to_cam.x);
    let world = p.pos + (right * c.x * CARD.x + vec3<f32>(0.0, c.y * CARD.y, 0.0)) * size;
    var out: VertexOutput;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = to_cam;
@if(VERTEX_UVS_A)
    out.uv = vec2<f32>(f32(index), 1.0);  // y 1: a card
@if(VERTEX_UVS_B)
    out.uv_b = c;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = instance;
    return out;
}

@vertex
fn vertex(vertex_in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let first = mesh[vertex_in.instance_index].first_vertex_index;
    let index = (vertex_index - first) / VERTS_PER_PERSON;
    let corner = (vertex_index - first) % VERTS_PER_PERSON;
    let p = people[index];
    // Level of detail first: most vertices of a far crowd stop here.
    let share = lod_shares(distance(view.world_position, p.pos));
    if p.size == 0.0 {
        return nothing();          // an empty slot
    }
    if corner >= FIGURE_VERTS {
        if share.y <= 0.0 {
            return nothing();
        }
        return card_vertex(p, index, corner - FIGURE_VERTS, vertex_in.instance_index);
    }
    if share.x <= 0.0 {
        return nothing();
    }
    let size = abs(p.size);
    let kind = u32(p.motion.x + 0.5);
    let phase = p.motion.y;
    let w = p.motion.z; // 0 standing .. 1 walking

    // Which part this vertex belongs to (the figure's layout in crowd.rs).
    // 0 body and head, 1 and 2 legs, 3 and 4 arms.
    let part = select(0u, (corner - 60u) / 30u + 1u, corner >= 60u);

    // Walking: legs swing opposite each other with the stride; each arm swings
    // with the opposite leg. Soldiers march stiff-armed.
    let leg = sin(phase) * 0.55;
    let arm_walk = select(0.8, 0.25, kind == GUARD) * leg;

    // Standing: the trade's motion, out of step from person to person.
    let pose = work_pose(kind, globals.time + f32(index) * 0.37);

    var angle = 0.0;
    var pivot = vec3<f32>(0.0);
    switch part {
        case 1u: { angle = leg * w; pivot = vec3(0.055, HIP_Y, 0.0); }
        case 2u: { angle = -leg * w; pivot = vec3(-0.055, HIP_Y, 0.0); }
        case 3u: { angle = mix(pose.arm_l, -arm_walk, w); pivot = vec3(0.17, SHOULDER_Y, 0.0); }
        case 4u: { angle = mix(pose.arm_r, arm_walk, w); pivot = vec3(-0.17, SHOULDER_Y, 0.0); }
        default: {}
    }
    var local = pivot + swing(v.position - pivot, angle);
    var normal = swing(v.normal, angle);
    // The upper body (not the legs) twists; everyone leans a little.
    if part == 0u || part >= 3u {
        let twist = pose.twist * (1.0 - w);
        local = turn(local, twist);
        normal = turn(normal, twist);
    }
    let lean = mix(pose.lean, 0.12, w);
    local.z += max(local.y - HIP_Y, 0.0) * lean;
    // A small bob, highest as the legs pass each other.
    local.y += abs(cos(phase)) * 0.035 * w;
    local *= size;

    let world = p.pos + turn(local, p.heading);
    var out: VertexOutput;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = normalize(turn(normal, p.heading));
@if(VERTEX_UVS_A)
    out.uv = vec2<f32>(f32(index), 0.0);  // the fragment stage looks the colour up; y 0: the figure
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
    return out;
}

// A 4x4 ordered-dither threshold (Bayer matrix) for a pixel: 16 levels,
// spread so that any share of the pixels on is an even pattern.
fn bayer(pixel: vec2<f32>) -> f32 {
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let x = u32(pixel.x) % 4u;
    let y = u32(pixel.y) % 4u;
    return (m[y * 4u + x] + 0.5) / 16.0;
}

// Signed distance to a box (centre c, half size h): negative inside.
fn sd_box(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>) -> f32 {
    let q = abs(p - c) - h;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
}

// The silhouette on the card, in figure units (x across, y up): the same
// parts as the mesh. The legs scissor with the stride.
fn silhouette(p: vec2<f32>, person: Person) -> f32 {
    let head = length(p - vec2<f32>(0.0, 0.66)) - 0.11;
    let body = sd_box(p, vec2<f32>(0.0, 0.42), vec2<f32>(0.11 + (p.y - 0.28) * 0.12, 0.135)) - 0.01;
    let stride = sin(person.motion.y) * person.motion.z * 0.35 * max(0.32 - p.y, 0.0);
    let legs = min(
        sd_box(p, vec2<f32>(0.055 + stride, 0.16), vec2<f32>(0.04, 0.16)),
        sd_box(p, vec2<f32>(-0.055 - stride, 0.16), vec2<f32>(0.04, 0.16)),
    );
    let arms = min(
        sd_box(p, vec2<f32>(0.17, 0.40), vec2<f32>(0.03, 0.13)),
        sd_box(p, vec2<f32>(-0.17, 0.40), vec2<f32>(0.03, 0.13)),
    );
    return min(min(head, body), min(legs, arms));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let index = u32(in.uv.x + 0.5);
    let person = people[index];
    let card = in.uv.y > 0.5;
    var pbr = pbr_input_from_standard_material(in, is_front);
    pbr.N = normalize(in.world_normal);

    // Cross-fade: keep this pixel if the dither threshold is below this
    // level's share. The figure keeps the pixels below its share, the card
    // those from there up to both shares, so between them every pixel is
    // covered once.
    let share = lod_shares(distance(view.world_position, person.pos));
    let b = bayer(in.position.xy);
    if (!card && b >= share.x) || (card && (b < share.x || b >= share.x + share.y)) {
        discard;
    }
    var tint = vec3<f32>(0.3, 1.0, 0.4); // the debug tint (K): green figures
    if card {
        // Draw the silhouette: outside it, no pixel.
        var c = vec2<f32>(0.0);
@if(VERTEX_UVS_B)
        c = in.uv_b;
        let p = c * CARD;
        if silhouette(p, person) > 0.0 {
            discard;
        }
        // Light it as if it were round: the normal turns sideways toward the
        // card's edges.
        let to_cam = normalize(in.world_normal);
        let right = vec3<f32>(to_cam.z, 0.0, -to_cam.x);
        let side = clamp(p.x / 0.2, -1.0, 1.0) * 0.8;
        pbr.N = normalize(right * side + to_cam * sqrt(1.0 - side * side) + vec3<f32>(0.0, 0.2, 0.0));
        tint = vec3<f32>(1.0, 0.55, 0.2); // orange cards
    }
    var albedo = person.color;
    if crowd.lod.w > 0.5 {
        albedo = tint;
    }
    let t = toon_terms(pbr, toon_params(crowd.toon));
    let fogged = apply_fog(albedo * t.light, in.world_position.xyz, fog_state);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(fogged, 1.0));
    return out;
}
