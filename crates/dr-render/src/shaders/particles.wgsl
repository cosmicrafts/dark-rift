// Instanced Particle Quad Shader with Tidal Spaghettification & Relativistic Stretching.

struct ParticleGlobals {
    screen_size: vec2<f32>,
    bh_center_uv: vec2<f32>,
    time: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

var<uniform> globals: ParticleGlobals;

struct ParticleInstance {
    @location(0) pos: vec2<f32>,
    @location(1) vel: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) size: f32,
    @location(4) spaghettification: f32,
    @location(5) tier: u32,
    @location(6) _pad: f32,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: ParticleInstance,
) -> VertexOutput {
    var out: VertexOutput;

    let quad_idx = vertex_index % 6u;
    var local_pos = vec2<f32>(0.0);
    var uv = vec2<f32>(0.0);

    if (quad_idx == 0u) {
        local_pos = vec2<f32>(-1.0, -1.0);
        uv = vec2<f32>(0.0, 0.0);
    } else if (quad_idx == 1u) {
        local_pos = vec2<f32>(1.0, -1.0);
        uv = vec2<f32>(1.0, 0.0);
    } else if (quad_idx == 2u) {
        local_pos = vec2<f32>(1.0, 1.0);
        uv = vec2<f32>(1.0, 1.0);
    } else if (quad_idx == 3u) {
        local_pos = vec2<f32>(1.0, 1.0);
        uv = vec2<f32>(1.0, 1.0);
    } else if (quad_idx == 4u) {
        local_pos = vec2<f32>(-1.0, 1.0);
        uv = vec2<f32>(0.0, 1.0);
    } else {
        local_pos = vec2<f32>(-1.0, -1.0);
        uv = vec2<f32>(0.0, 0.0);
    }

    let speed = length(instance.vel);
    var dir = vec2<f32>(1.0, 0.0);
    if (speed > 0.001) {
        dir = instance.vel / speed;
    }
    let normal = vec2<f32>(-dir.y, dir.x);

    let stretch = instance.spaghettification;
    let size = instance.size;
    let world_offset = dir * (local_pos.x * size * stretch) + normal * (local_pos.y * size);
    let world_pos = instance.pos + world_offset;

    let aspect = globals.screen_size.x / globals.screen_size.y;
    // Align with exact unified center UV
    let screen_x = (world_pos.x / aspect) + (globals.bh_center_uv.x * 2.0 - 1.0);
    let screen_y = world_pos.y - (globals.bh_center_uv.y * 2.0 - 1.0);
    out.clip_pos = vec4<f32>(screen_x, screen_y, 0.0, 1.0);
    out.uv = uv;
    out.color = instance.color;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = length(in.uv - vec2<f32>(0.5)) * 2.0;
    if (d > 1.0) {
        discard;
    }

    let alpha = (1.0 - d * d) * in.color.a;
    let glow = exp(-d * 3.0) * 1.5;

    return vec4<f32>(in.color.rgb * glow, alpha);
}
