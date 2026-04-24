//! GPU-driven map rendering material for Bevy 0.18.
//!
//! Uses `ShaderStorageBuffer` (bevy::render::storage) storage buffers so the GPU fragment
//! shader reads tile ownership and terrain directly — zero CPU texture uploads at runtime.
//!
//! Bindings:
//!   0 — board_data:    packed u16 owner IDs (2 per u32) — written on tile capture
//!   1 — border_color:  uniform RGBA border darkening multiplier
//!   2 — border_thickness: uniform f32
//!   3 — texture_size:  uniform vec2 (map_w, map_h)
//!   4 — player_colors: packed f32×4 per player ID
//!   5 — map_terrain:   packed MapTile u8s (4 per u32) — written once at game start
//!   6 — time:          uniform f32 for water animation
//!   7 — enable_water_animation: uniform u32
//!   8 — enable_players: uniform u32

use bevy::{
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    sprite_render::Material2d,
};

#[derive(Clone, Default, ShaderType, Debug)]
pub struct BorderUniforms {
    pub border_color: LinearRgba,
    pub border_thickness: Vec4,
    pub texture_size: Vec4,
    pub time: Vec4,
    pub enable_water_animation: UVec4,
    pub enable_players: UVec4,
}

/// Custom 2D material that renders the entire map via a fragment shader.
/// All ownership and terrain data live on GPU storage buffers — no RGBA texture uploads.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct BorderMaterial {
    /// Packed owner IDs: two u16s per u32 (low 16 = tile at even index, high 16 = odd index).
    /// Player ID = low 12 bits of each u16 (matches `GameMap::PLAYER_ID_MASK`).
    #[texture(0, dimension = "2d", sample_type = "u_int", visibility(fragment))]
    pub board_data: Handle<Image>,

    #[uniform(1, visibility(fragment))]
    pub uniforms: BorderUniforms,

    /// Player RGBA colors: one `vec4<f32>` per player ID, index 0 = wilderness.
    #[texture(2, dimension = "2d", sample_type = "float", visibility(fragment))]
    pub player_colors: Handle<Image>,

    /// Packed terrain bytes: four `MapTile` u8s per u32.
    /// `MapTile` bit layout: bit7=land, bit6=shoreline, bit5=ocean, bits[4:0]=magnitude.
    #[texture(3, dimension = "2d", sample_type = "u_int", visibility(fragment))]
    pub map_terrain: Handle<Image>,
}

impl Material2d for BorderMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/border.wgsl".into()
    }
}
