// Dark Rift — GPU Map Renderer
// Adapted from openfrust/assets/shaders/border.wgsl
//
// Replaces the CPU-side chunked RGBA texture approach with zero-upload GPU rendering.
// The CPU only writes owner IDs (u16) to board_data when a tile changes owner.
// All color computation, border detection, and terrain rendering happens here on the GPU.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput
#import "shaders/simplex.wgsl"::animated_simplex_noise

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var board_data:    texture_2d<u32>;
struct BorderUniforms {
    border_color: vec4<f32>,
    border_thickness: vec4<f32>,
    texture_size: vec4<f32>,
    time: vec4<f32>,
    enable_water_animation: vec4<u32>,
    enable_players: vec4<u32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> uniforms: BorderUniforms;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var player_colors: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var map_terrain:   texture_2d<u32>;

// ============================================================================
// CONSTANTS
// ============================================================================

// Terrain colors — match openfront_tile_color() in main.rs
const PLAINS_COLOR:    vec3<f32> = vec3<f32>(0.4, 0.7, 0.3);   // Light Green (Land)
const HIGHLAND_COLOR:  vec3<f32> = vec3<f32>(0.2, 0.5, 0.2);   // Dark Green (Highland)
const MOUNTAIN_COLOR:  vec3<f32> = vec3<f32>(0.6, 0.5, 0.4);   // Brown/Gray (Mountain)

const DEEP_OCEAN_COLOR:  vec3<f32> = vec3<f32>(0.01, 0.08, 0.23);
const COASTAL_COLOR:     vec3<f32> = vec3<f32>(0.1,  0.5,  0.6);
const FOAM_COLOR:        vec3<f32> = vec3<f32>(0.9,  0.95, 1.0);
const RIVER_COLOR:       vec3<f32> = COASTAL_COLOR;
const LAKE_COLOR:        vec4<f32> = vec4<f32>(0.05, 0.15, 0.4, 1.0);
const SAND_COLOR:        vec4<f32> = vec4<f32>(0.8, 0.8, 0.6, 1.0);
const SAND_BLEND:        f32       = 0.7;

// Water animation
const FOAM_ANIM_SPEED:     f32 = 1.5;
const FOAM_ANIM_AMPLITUDE: f32 = 1.0;
const COASTAL_ANIM_SPEED:  f32 = 0.5;
const COASTAL_TO_OCEAN:    f32 = 400.0;
const COASTAL_PIXELLATION: f32 = 80.0;
const WAVE1_SPEED: f32 = 0.4; const WAVE1_SCALE: f32 = 0.5;  const WAVE1_W: f32 = 1.0;
const WAVE2_SPEED: f32 = 0.2; const WAVE2_SCALE: f32 = 0.04; const WAVE2_W: f32 = 0.2;
const WAVE3_SPEED: f32 = 0.1; const WAVE3_SCALE: f32 = 0.001;const WAVE3_W: f32 = 0.4;
const WAVE_COMBINED: f32 = 0.5;
const WAVE_HIGHLIGHT: vec3<f32> = vec3<f32>(0.1, 0.2, 0.7);
const WAVE_STRENGTH:  f32 = 0.5;

// Player color blending — how much player color overlays terrain
const PLAYER_COLOR_BLEND: f32 = 0.6;

// Simple coastal distance estimate (no SDF texture): pixels from 0..64
const COAST_FAKE_DIST: f32 = 4.0;

// ============================================================================
// TERRAIN DATA HELPERS
// ============================================================================

/// Read raw MapTile byte for a tile coordinate.
/// Packing: 4 u8s per u32, byte index = linear_index % 4, word = linear_index / 4.
fn get_map_tile_at(coord: vec2<i32>) -> u32 {
    if coord.x < 0 || coord.x >= i32(uniforms.texture_size.x) || coord.y < 0 || coord.y >= i32(uniforms.texture_size.y) {
        return 0u;
    }
    return textureLoad(map_terrain, coord, 0).r;
}

/// MapTile bit layout (matches dark_rift_shared::map::MapTile):
///   bit7 = is_land, bit6 = is_shoreline, bit5 = is_ocean, bits[4:0] = magnitude
fn tile_is_land(tile: u32)     -> bool { return (tile & 0x80u) != 0u; }
fn tile_is_ocean(tile: u32)    -> bool { return (tile & 0x20u) != 0u; }
fn tile_is_water(tile: u32)    -> bool { return !tile_is_land(tile); }
fn tile_magnitude(tile: u32)   -> u32  { return tile & 0x1Fu; }

// ============================================================================
// BOARD DATA (OWNER IDs)
// ============================================================================

/// Read owner player_id at a tile coordinate.
/// Packing: 2 u16s per u32. Tile at linear_index:
///   word  = linear_index / 2
///   shift = (linear_index % 2) * 16
///   player_id = low 12 bits of the u16
fn get_owner_at(coord: vec2<i32>) -> u32 {
    if coord.x < 0 || coord.x >= i32(uniforms.texture_size.x) || coord.y < 0 || coord.y >= i32(uniforms.texture_size.y) {
        return 0u;
    }
    let tile_data = textureLoad(board_data, coord, 0).r;
    return tile_data & 0x0FFFu;   // 12-bit player ID (matches GameMap::PLAYER_ID_MASK)
}

// ============================================================================
// TERRAIN COLOR
// ============================================================================

fn get_terrain_color(tile: u32, coord: vec2<i32>) -> vec4<f32> {
    if tile_is_land(tile) {
        let mag = tile_magnitude(tile);
        if mag < 10u {
            return vec4<f32>(PLAINS_COLOR, 1.0);
        } else if mag < 20u {
            return vec4<f32>(HIGHLAND_COLOR, 1.0);
        } else {
            return vec4<f32>(MOUNTAIN_COLOR, 1.0);
        }
    } else if tile_is_ocean(tile) || tile_is_water(tile) {
        // Simple water: approximate coastal gradient without an SDF texture.
        // Check neighbors to estimate proximity to land.
        let f_coord = vec2<f32>(coord);

        var land_neighbors = 0u;
        let n = get_map_tile_at(coord + vec2<i32>( 0,  1));
        let s = get_map_tile_at(coord + vec2<i32>( 0, -1));
        let e = get_map_tile_at(coord + vec2<i32>( 1,  0));
        let w = get_map_tile_at(coord + vec2<i32>(-1,  0));
        if tile_is_land(n) { land_neighbors += 1u; }
        if tile_is_land(s) { land_neighbors += 1u; }
        if tile_is_land(e) { land_neighbors += 1u; }
        if tile_is_land(w) { land_neighbors += 1u; }

        var base_color: vec3<f32>;
        if land_neighbors >= 2u {
            base_color = FOAM_COLOR;
        } else if land_neighbors == 1u {
            base_color = COASTAL_COLOR;
        } else {
            base_color = DEEP_OCEAN_COLOR;
        }

        if uniforms.enable_water_animation.x != 0u {
            let wave1 = animated_simplex_noise(f_coord, WAVE1_SPEED, WAVE1_SCALE, uniforms.time.x);
            let wave2 = animated_simplex_noise(f_coord, WAVE2_SPEED, WAVE2_SCALE, uniforms.time.x);
            let wave3 = animated_simplex_noise(f_coord, WAVE3_SPEED, WAVE3_SCALE, uniforms.time.x);
            let combined = (wave1 * WAVE1_W + wave2 * WAVE2_W + wave3 * WAVE3_W) * WAVE_COMBINED;
            base_color = mix(base_color, WAVE_HIGHLIGHT, combined * WAVE_STRENGTH);
        }

        return vec4<f32>(base_color, 1.0);
    } else {
        // Lake
        return LAKE_COLOR;
    }
}

// ============================================================================
// FRAGMENT SHADER
// ============================================================================

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    // Discrete tile coordinate for this fragment.
    let pixel_coord = vec2<i32>(mesh.uv * uniforms.texture_size.xy);
    let offset = i32(uniforms.border_thickness.x);

    let center_terrain = get_map_tile_at(pixel_coord);
    let center_owner   = get_owner_at(pixel_coord);

    let t_n = get_map_tile_at(pixel_coord + vec2<i32>( 0,  offset));
    let t_s = get_map_tile_at(pixel_coord + vec2<i32>( 0, -offset));
    let t_e = get_map_tile_at(pixel_coord + vec2<i32>( offset,  0));
    let t_w = get_map_tile_at(pixel_coord + vec2<i32>(-offset,  0));

    var terrain_color = get_terrain_color(center_terrain, pixel_coord);

    // Shoreline sand blend for land tiles adjacent to water.
    if tile_is_land(center_terrain) {
        if tile_is_water(t_n) || tile_is_water(t_s) || tile_is_water(t_e) || tile_is_water(t_w) {
            terrain_color = mix(terrain_color, SAND_COLOR, SAND_BLEND);
        }
    }

    // Mix in player color.
    var base_color: vec4<f32>;
    if uniforms.enable_players.x != 0u {
        if center_owner == 0u {
            base_color = terrain_color;
        } else {
            let player_color = textureLoad(player_colors, vec2<i32>(i32(center_owner), 0), 0);
            base_color = mix(terrain_color, player_color, PLAYER_COLOR_BLEND);
        }
    } else {
        base_color = terrain_color;
    }

    // Border: narrow band along tile edges (border_thickness.y = fraction of tile, e.g. 0.08).
    // Color = mix toward black with a touch of territory tint (border_color.a = tint strength, ~0.1).
    if uniforms.enable_players.x != 0u {
        let center_is_water = tile_is_water(center_terrain);
        if !center_is_water {
            let own_n = get_owner_at(pixel_coord + vec2<i32>( 0,  offset));
            let own_s = get_owner_at(pixel_coord + vec2<i32>( 0, -offset));
            let own_e = get_owner_at(pixel_coord + vec2<i32>( offset,  0));
            let own_w = get_owner_at(pixel_coord + vec2<i32>(-offset,  0));

            var edge_n = false;
            var edge_s = false;
            var edge_e = false;
            var edge_w = false;
            if center_owner == 0u {
                edge_n = (center_owner != own_n) || tile_is_water(t_n);
                edge_s = (center_owner != own_s) || tile_is_water(t_s);
                edge_e = (center_owner != own_e) || tile_is_water(t_e);
                edge_w = (center_owner != own_w) || tile_is_water(t_w);
            } else {
                edge_n = center_owner != own_n;
                edge_s = center_owner != own_s;
                edge_e = center_owner != own_e;
                edge_w = center_owner != own_w;
            }

            // One arc per shared edge: paint +x and +y sides always; paint -x/-y only when that
            // neighbor is water (water fragments skip this block, so land must draw the seam).
            // Opposite land tiles otherwise draw the same line twice (double thickness).
            let paint_n = edge_n;
            let paint_e = edge_e;
            let paint_s = edge_s && tile_is_water(t_s);
            let paint_w = edge_w && tile_is_water(t_w);

            let ft = fract(mesh.uv * uniforms.texture_size.xy);
            let band = clamp(uniforms.border_thickness.y, 0.001, 0.49);
            var edge_mix = 0.0;
            if paint_n {
                edge_mix = max(edge_mix, smoothstep(1.0 - band, 1.0, ft.y));
            }
            if paint_s {
                edge_mix = max(edge_mix, smoothstep(1.0 - band, 1.0, 1.0 - ft.y));
            }
            if paint_e {
                edge_mix = max(edge_mix, smoothstep(1.0 - band, 1.0, ft.x));
            }
            if paint_w {
                edge_mix = max(edge_mix, smoothstep(1.0 - band, 1.0, 1.0 - ft.x));
            }

            if edge_mix > 0.0 {
                let tint = uniforms.border_color.a;
                let border_rgb = mix(vec3<f32>(0.0), base_color.rgb, tint);
                let out_rgb = mix(base_color.rgb, border_rgb, edge_mix);
                return vec4<f32>(out_rgb, base_color.a);
            }
        }
    }

    return base_color;
}
