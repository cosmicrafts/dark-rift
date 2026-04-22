// ============================================================================
// SIMPLEX NOISE IMPLEMENTATION
// ============================================================================
// 2D Simplex noise implementation for WGSL
// Used for procedural texture generation and animation effects

// --- Simplex Noise Constants ---
const SIMPLEX_C: vec4<f32> = vec4<f32>(0.2113248654, 0.3660254037, -0.5773502691, 0.0243902439);
const SIMPLEX_M_SCALE: f32 = 1.792842914;
const SIMPLEX_M_OFFSET: f32 = 0.853734720;
const SIMPLEX_G_SCALE: f32 = 130.0;

// Permutation constants
const PERMUTE_MOD: f32 = 289.0;
const PERMUTE_MULT: f32 = 34.0;
const PERMUTE_ADD: f32 = 1.0;

// --- Helper Functions ---
fn mod289(x: vec2<f32>) -> vec2<f32> {
    return x - floor(x * (1.0 / PERMUTE_MOD)) * PERMUTE_MOD;
}

fn mod289_3(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x * (1.0 / PERMUTE_MOD)) * PERMUTE_MOD;
}

fn permute3(x: vec3<f32>) -> vec3<f32> {
    return mod289_3(((x * PERMUTE_MULT) + PERMUTE_ADD) * x);
}

// --- Core Simplex Noise Function ---
// Returns a value in the range [-1, 1]
fn simplexNoise2(v: vec2<f32>) -> f32 {
    var i = floor(v + dot(v, SIMPLEX_C.yy));
    let x0 = v - i + dot(i, SIMPLEX_C.xx);
    let i1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), x0.x > x0.y);
    let x1 = x0 - i1 + SIMPLEX_C.xx;
    let x2 = x0 - 1.0 + SIMPLEX_C.yy;
    i = mod289(i);
    var p = permute3(permute3(i.y + vec3<f32>(0.0, i1.y, 1.0)) + i.x + vec3<f32>(0.0, i1.x, 1.0));
    var m = max(0.5 - vec3<f32>(dot(x0, x0), dot(x1, x1), dot(x2, x2)), vec3<f32>(0.0));
    m = m * m;
    m = m * m;
    let x = 2.0 * fract(p * SIMPLEX_C.www) - 1.0;
    let h = abs(x) - 0.5;
    let ox = floor(x + 0.5);
    let a0 = x - ox;
    m = m * (SIMPLEX_M_SCALE - SIMPLEX_M_OFFSET * (a0 * a0 + h * h));
    let g = vec3<f32>(a0.x * x0.x + h.x * x0.y, a0.y * x1.x + h.y * x1.y, a0.z * x2.x + h.z * x2.y);
    return SIMPLEX_G_SCALE * dot(m, g);
}

// --- Animated Simplex Noise ---
// Takes a coordinate (typically in pixel space), speed, scale, and time uniform
// Returns a value in the range [0, 1]
fn animated_simplex_noise(coord: vec2<f32>, speed: f32, scale: f32, time: f32) -> f32 {
    let p = coord * scale;
    let motion = vec2<f32>(time * speed, -time * speed * 0.7);
    let noise = simplexNoise2(p + motion);
    return (noise + 1.0) * 0.5;
}
