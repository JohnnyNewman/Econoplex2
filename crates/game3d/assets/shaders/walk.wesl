// Shader step 07: the crowd walks on the GPU. Explained in
// docs/shaders/07-crowds-compute.md (and, for the stride phase and turning,
// docs/shaders/09-procedural-animation.md).
//
// A compute shader: no triangles, no pixels, just a function run once per
// person, in parallel, in groups of 64 ("workgroups"). Every frame it moves
// each person toward their target and writes the result straight into the
// buffer that people.wesl draws from. The CPU only uploads new targets when the
// simulation finishes a tick.

// One person as people.wesl reads it (12 floats).
struct Person {
    pos: vec3<f32>,
    heading: f32,
    color: vec3<f32>,
    size: f32,         // 1 adult, 0.65 child; negative = walking; 0 = empty slot
    motion: vec4<f32>, // x: work motion, y: stride phase, z: walking blend, w: unused
};

// One target, written by the CPU per tick (8 floats).
struct Target {
    goal: vec2<f32>,   // where to walk to (3D x, z)
    occupied: f32,     // 1 = someone lives in this slot, 0 = empty (`active` is a
                       // reserved word in WGSL)
    size: f32,
    color: vec3<f32>,
    motion: f32,       // what they do with their arms when standing (crowd.rs `Motion`)
};

struct Params {
    dt: f32,           // seconds since the last frame
    walk: f32,         // walking speed (units per second)
    count: u32,        // slots in the buffers
    pad: u32,
    grid: vec4<f32>,   // terrain: x, z of texel (0,0); texel spacing; height scale
    dims: vec4<f32>,   // terrain: texels across (i), deep (j)
    walls: vec4<f32>,  // wall map: x, z of its first corner; size along x, z (0: none)
};

@group(0) @binding(0) var<storage, read_write> people: array<Person>;
@group(0) @binding(1) var<storage, read> targets: array<Target>;
@group(0) @binding(2) var heights: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;
// How far each spot is from the nearest wall of a building or house, signed
// (negative inside): grounding.rs draws it, 0.5 + d / (2 WALL_RANGE).
@group(0) @binding(4) var walls: texture_2d<f32>;

const TAU: f32 = 6.2831853;
// Distance covered in one full stride (two steps), in 3D units.
const STRIDE: f32 = 1.1;
// How fast people turn to a new heading (radians per second) and how fast the
// walking blend follows (per second).
const TURN: f32 = 8.0;
const BLEND: f32 = 6.0;

// Terrain height at a ground point: the heightmap of step 03, filtered
// bilinearly by hand (the same mapping as terrain_common.wesl).
fn texel(t: vec2<i32>) -> f32 {
    let last = vec2<i32>(params.dims.xy) - 1;
    return textureLoad(heights, clamp(t, vec2<i32>(0), last), 0).r * params.grid.w;
}
fn ground(xz: vec2<f32>) -> f32 {
    let tc = vec2<f32>(xz.x - params.grid.x, params.grid.y - xz.y) / params.grid.z;
    let t0 = vec2<i32>(floor(tc));
    let f = fract(tc);
    let top = mix(texel(t0), texel(t0 + vec2(1, 0)), f.x);
    let bottom = mix(texel(t0 + vec2(0, 1)), texel(t0 + vec2(1, 1)), f.x);
    return mix(top, bottom, f.y);
}

// The wall map's range (grounding.rs `WALL_RANGE`), how far a walker keeps
// from a wall (crowd.rs `WALKER`) and how far ahead they look for one, all in
// 3D units.
const WALL_RANGE: f32 = 1.0;
const RADIUS: f32 = 0.25;
const LOOK: f32 = 0.6;

// The distance to the nearest wall at a ground point (bilinear, as `ground`),
// WALL_RANGE when there is no wall map or it is far.
fn wall_texel(t: vec2<i32>) -> f32 {
    let last = vec2<i32>(textureDimensions(walls)) - 1;
    return (textureLoad(walls, clamp(t, vec2<i32>(0), last), 0).r - 0.5) * 2.0 * WALL_RANGE;
}
fn wall(xz: vec2<f32>) -> f32 {
    if params.walls.z <= 0.0 {
        return WALL_RANGE;
    }
    let size = vec2<f32>(textureDimensions(walls));
    let tc = (xz - params.walls.xy) / params.walls.zw * size - 0.5;
    if any(tc < vec2<f32>(-1.0)) || any(tc > size) {
        return WALL_RANGE;
    }
    let t0 = vec2<i32>(floor(tc));
    let f = fract(tc);
    let top = mix(wall_texel(t0), wall_texel(t0 + vec2(1, 0)), f.x);
    let bottom = mix(wall_texel(t0 + vec2(0, 1)), wall_texel(t0 + vec2(1, 1)), f.x);
    return mix(top, bottom, f.y);
}
// Away from the nearest wall (unit), from the map's slope.
fn away(xz: vec2<f32>) -> vec2<f32> {
    let e = params.walls.z / f32(textureDimensions(walls).x);
    let g = vec2<f32>(
        wall(xz + vec2(e, 0.0)) - wall(xz - vec2(e, 0.0)),
        wall(xz + vec2(0.0, e)) - wall(xz - vec2(0.0, e)),
    );
    let l = length(g);
    return select(vec2<f32>(0.0), g / l, l > 1e-6);
}

// 64 people per workgroup; the CPU dispatches ceil(count / 64) workgroups.
@compute @workgroup_size(64)
fn walk(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= params.count {
        return;                       // the last workgroup may run past the end
    }
    let t = targets[i];
    var p = people[i];
    if t.occupied < 0.5 {
        p.size = 0.0;                 // empty slot: nothing to draw
        people[i] = p;
        return;
    }
    // Someone new in this slot (it was empty last frame): appear at the goal,
    // standing, with a stride phase of their own.
    var here = p.pos.xz;
    if p.size == 0.0 {
        here = t.goal;
        p.motion = vec4<f32>(t.motion, f32(i) * 1.618, 0.0, 0.0);
    }
    let to = t.goal - here;
    let dist = length(to);
    // Walk; far-off goals (a new home, a raid) are reached quicker.
    let step = min(max(params.walk, dist * 0.3) * params.dt, dist);
    let walking = dist > 0.05;
    if walking {
        var dir = to / dist;
        // A wall ahead, with the goal beyond it: turn along the wall, the way
        // nearer the goal, the more the nearer it is.
        if dist > LOOK {
            let ahead = here + dir * LOOK;
            let d = wall(ahead);
            if d < RADIUS + LOOK * 0.5 {
                let out = away(ahead);
                var along = vec2<f32>(-out.y, out.x);
                if dot(along, dir) < 0.0 {
                    along = -along;
                }
                let k = clamp((RADIUS + LOOK * 0.5 - d) / LOOK, 0.0, 1.0);
                dir = normalize(mix(dir, along, k) + out * k * 0.3 + vec2<f32>(1e-6));
            }
        }
        here += dir * step;
        // Never into a wall: out to RADIUS from it.
        let d = wall(here);
        if d < RADIUS {
            here += away(here) * (RADIUS - d);
        }
        // Turn toward the goal at a limited rate instead of snapping. The
        // difference of two angles, wrapped into -pi..pi, is atan2(sin, cos).
        let want = atan2(dir.x, dir.y);
        let d = want - p.heading;
        let diff = atan2(sin(d), cos(d));
        p.heading += clamp(diff, -TURN * params.dt, TURN * params.dt);
        // The stride follows the distance walked (at most walking speed, so a
        // hurried long walk doesn't blur the legs): feet don't slide.
        p.motion.y = (p.motion.y + min(step, params.walk * params.dt) / STRIDE * TAU) % TAU;
    }
    // Ease the walking blend toward 1 (walking) or 0 (standing), so limbs settle.
    let aim = select(0.0, 1.0, walking);
    p.motion.z += (aim - p.motion.z) * min(BLEND * params.dt, 1.0);
    p.motion.x = t.motion;
    p.pos = vec3<f32>(here.x, ground(here), here.y);
    p.color = t.color;
    p.size = select(t.size, -t.size, walking);
    people[i] = p;                    // each invocation writes only its own slot
}
