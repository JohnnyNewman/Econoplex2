// M4: people as one crowd; step 09: procedural animation. See
// docs/shaders/07-crowds-compute.md and docs/shaders/09-procedural-animation.md.
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

import bevy_pbr::render::{
    mesh_functions,
    mesh_bindings::mesh,
    forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex},
    mesh_view_bindings::globals,
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
};
import super::toon_lighting::{Toon, toon_params, toon_terms};
import super::fog::apply_fog;

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
};
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> crowd: Crowd;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage, read> people: array<Person>;

const VERTS_PER_PERSON: u32 = 180u;
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

@vertex
fn vertex(vertex_in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let v = decompress_vertex(vertex_in, vertex_in.instance_index);
    let first = mesh[vertex_in.instance_index].first_vertex_index;
    let index = (vertex_index - first) / VERTS_PER_PERSON;
    let corner = (vertex_index - first) % VERTS_PER_PERSON;
    let p = people[index];
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
    out.uv = vec2<f32>(f32(index), 0.0);  // the fragment stage looks the colour up
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    pbr.N = normalize(in.world_normal);
    let albedo = people[u32(in.uv.x + 0.5)].color;
    let t = toon_terms(pbr, toon_params(crowd.toon));
    let fogged = apply_fog(albedo * t.light, in.world_position.xyz, crowd.toon.fog);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr, vec4<f32>(fogged, 1.0));
    return out;
}
