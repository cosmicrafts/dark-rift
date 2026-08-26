// Fullscreen Schwarzschild / Kerr Relativistic Gravitational Lensing & Accretion Disk Shader.

struct BlackHoleUniforms {
    screen_size: vec2<f32>,
    bh_center_uv: vec2<f32>,
    time: f32,
    rs: f32,
    spin: f32,
    accretion_flash: f32,
    penrose_active: f32,
    jet_intensity: f32,
    disk_angle: f32,
    _pad0: f32,
};

var<uniform> uniforms: BlackHoleUniforms;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vertex_index & 2u) * 2.0 - 1.0;
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, y * 0.5 + 0.5);
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    let q = fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
    return q;
}

fn starfield(uv: vec2<f32>) -> vec3<f32> {
    let grid_uv = uv * 35.0;
    let id = floor(grid_uv);
    let g_uv = fract(grid_uv) - 0.5;

    let h = hash21(id);
    var color = vec3<f32>(0.0);

    if (h > 0.94) {
        let size = (h - 0.94) * 15.0;
        let d = length(g_uv);
        let star = smoothstep(size * 0.08, 0.0, d);
        let tint = vec3<f32>(0.7 + 0.3 * hash21(id + 1.0), 0.8 + 0.2 * hash21(id + 2.0), 1.0);
        color += star * tint * (0.8 + 0.2 * sin(uniforms.time * 3.0 + h * 6.28));
    }
    return color;
}

fn nebula(uv: vec2<f32>) -> vec3<f32> {
    let p = uv * 2.5;
    let n1 = sin(p.x * 1.5 + uniforms.time * 0.05) * cos(p.y * 1.5 - uniforms.time * 0.04);
    let n2 = sin(p.x * 3.0 - p.y * 2.0 + n1 * 2.0);
    let glow = max(0.0, n1 * 0.5 + n2 * 0.5);
    let purple = vec3<f32>(0.12, 0.02, 0.25);
    let cyan = vec3<f32>(0.01, 0.08, 0.18);
    return mix(purple, cyan, glow) * glow * 0.7;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = uniforms.screen_size.x / uniforms.screen_size.y;
    // Align with exact unified center UV
    var uv = (in.uv - uniforms.bh_center_uv) * 2.0;
    uv.x *= aspect;

    let r = length(uv);
    let rs = uniforms.rs;

    // 1. Gravitational Lensing
    let b = max(r, 0.001);
    let deflection = (rs * 1.8) / (b + 0.02);
    let bent_uv = uv + normalize(uv) * deflection * (1.0 - smoothstep(rs * 0.8, rs * 4.0, b));

    var col = nebula(bent_uv * 0.5) + starfield(bent_uv);

    // 2. Accretion Disk
    let disk_tilt = 0.35;
    var disk_uv = vec2<f32>(uv.x, uv.y / disk_tilt);
    let disk_r = length(disk_uv);
    let disk_angle = atan2(disk_uv.y, disk_uv.x) + uniforms.disk_angle;

    let isco = rs * 1.2;
    let outer_disk = rs * 3.8;

    if (disk_r > isco && disk_r < outer_disk) {
        let spiral = sin(disk_angle * 3.0 + disk_r * 15.0 - uniforms.time * 4.0);
        let density = smoothstep(isco, isco * 1.4, disk_r) * smoothstep(outer_disk, outer_disk * 0.7, disk_r);
        let gas = (0.5 + 0.5 * spiral) * density;

        let doppler_factor = clamp(1.0 - (uv.x / (disk_r + 0.1)) * (0.6 + uniforms.spin * 0.4), 0.2, 2.5);
        let beamed_intensity = pow(doppler_factor, 3.0);

        let t_norm = (disk_r - isco) / (outer_disk - isco);
        let inner_col = vec3<f32>(0.6, 0.85, 1.0) * 1.8;
        let mid_col = vec3<f32>(1.0, 0.6, 0.15) * 1.4;
        let outer_col = vec3<f32>(0.7, 0.1, 0.05) * 0.8;

        var disk_color = mix(inner_col, mid_col, smoothstep(0.0, 0.4, t_norm));
        disk_color = mix(disk_color, outer_col, smoothstep(0.4, 1.0, t_norm));

        let flash_boost = 1.0 + uniforms.accretion_flash * 1.5 + uniforms.penrose_active * 0.8;
        col += disk_color * gas * beamed_intensity * flash_boost;
    }

    // 3. Photon Sphere
    let photon_r = rs * 1.45;
    let photon_ring = smoothstep(photon_r - 0.015, photon_r, r) * smoothstep(photon_r + 0.015, photon_r, r);
    col += vec3<f32>(0.9, 0.95, 1.0) * photon_ring * 2.2;

    // 4. Polar Jets
    if (uniforms.jet_intensity > 0.01) {
        let jet_x = abs(uv.x);
        let jet_y = uv.y;
        let jet_beam = smoothstep(0.06 * (abs(jet_y) + 0.2), 0.0, jet_x) * (0.8 + 0.2 * sin(jet_y * 30.0 - uniforms.time * 20.0));
        let jet_col = vec3<f32>(0.4, 0.7, 1.0) * uniforms.jet_intensity * 2.5;
        col += jet_col * jet_beam;
    }

    // 5. Gravitational Pulse Wave
    if (uniforms.accretion_flash > 0.01) {
        let pulse_radius = rs + (1.0 - uniforms.accretion_flash) * 0.8;
        let ring = smoothstep(pulse_radius - 0.03, pulse_radius, r) * smoothstep(pulse_radius + 0.03, pulse_radius, r);
        col += vec3<f32>(0.3, 0.9, 1.0) * ring * uniforms.accretion_flash * 2.0;
    }

    // 6. Event Horizon
    let horizon_mask = smoothstep(rs * 0.92, rs * 1.02, r);
    col *= horizon_mask;

    return vec4<f32>(col, 1.0);
}
