use bevy::prelude::*;
use dark_rift_shared::engine::DarkRiftEngine;
use dark_rift_shared::game::GameState;
use dark_rift_shared::map::MapTile;
use dark_rift_shared::protocol::PlayerInfo;
use dark_rift_shared::player::PlayerType;
use rand::{Rng, SeedableRng};

use crate::*;

pub fn setup_loading_map_timeout(mut commands: Commands) {
    commands.insert_resource(LoadingMapTimeout(Timer::from_seconds(15.0, TimerMode::Once)));
}

pub fn system_loading_map(
    time: Res<Time>,
    mut commands: Commands,
    mut timeout: ResMut<LoadingMapTimeout>,
    mut cache: Option<ResMut<MapCache>>,
    pending_msg: Option<ResMut<PendingServerStart>>,
    mock_msg: Option<ResMut<MockSinglePlayerStart>>,
    mut cam_q: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    mut params: StartGameParams,
) {
    timeout.0.tick(time.delta());
    if timeout.0.just_finished() {
        bevy::log::error!("Map load timeout! Returning to lobby.");
        params.next_state.set(AppState::LobbyBrowser);
        commands.remove_resource::<PendingServerStart>();
        commands.remove_resource::<MockSinglePlayerStart>();
        return;
    }

    if pending_msg.is_none() && mock_msg.is_none() {
        return;
    }

    // Ensure MapCache exists
    if cache.is_none() {
        commands.insert_resource(MapCache::default());
        return; // Wait one frame for it to be available
    }
    let cache = cache.as_mut().unwrap();

    let map_name = if let Some(ref p) = pending_msg {
        p.0.config.map_name.clone()
    } else if let Some(ref m) = mock_msg {
        m.0.config.map_name.clone()
    } else {
        return;
    };

    // If cache doesn't have the right map, request it
    if !cache.is_ready() || cache.map_name != map_name {
        if !cache.is_fetching() || cache.map_name != map_name {
            cache.request(&map_name, &mut commands);
        }
        return; // Wait for fetch to complete
    }

    // Map is ready — start the game
    let cached_bytes = cache.take_data();

    if let Some(mut p_msg) = pending_msg {
        p_msg.0.map_data = cached_bytes;
        start_game_core(
            p_msg.0.clone(),
            &mut cam_q,
            &mut commands,
            &mut params,
        );
        commands.remove_resource::<PendingServerStart>();
    } else if let Some(mut m_msg) = mock_msg {
        if let Some(bytes) = &cached_bytes {
            let width = m_msg.0.config.map_width;
            let height = m_msg.0.config.map_height;
            let total_players = 1 + m_msg.0.config.bot_count;

            let mut rng = rand::rngs::StdRng::seed_from_u64(m_msg.0.seed);

            for i in 0..total_players {
                for _ in 0..10000 {
                    let sx = rng.gen_range(0..width);
                    let sy = rng.gen_range(0..height);
                    let idx = (sy * width + sx) as usize;
                    if idx < bytes.len() {
                        let tile = MapTile::from_byte(bytes[idx]);
                        if tile.is_land() {
                            if i == 0 {
                                m_msg.0.players[0].spawn_x = sx;
                                m_msg.0.players[0].spawn_y = sy;
                            } else {
                                m_msg.0.players.push(PlayerInfo {
                                    id: (i + 1) as u16,
                                    name: format!("Bot {}", i),
                                    player_type: PlayerType::Bot,
                                    color: [
                                        rng.gen_range(0.2..0.8),
                                        rng.gen_range(0.2..0.8),
                                        rng.gen_range(0.2..0.8),
                                    ],
                                    spawn_x: sx,
                                    spawn_y: sy,
                                });
                            }
                            break;
                        }
                    }
                }
            }
        }

        m_msg.0.map_data = cached_bytes;
        commands.insert_resource(SinglePlayerState {
            timer: Timer::from_seconds(0.040, TimerMode::Repeating),
            current_turn: 0,
        });
        commands.insert_resource(LocalIntentBuffer::default());
        start_game_core(
            m_msg.0.clone(),
            &mut cam_q,
            &mut commands,
            &mut params,
        );
        commands.remove_resource::<MockSinglePlayerStart>();
    }
}

pub fn handle_tile_change(
    mut engine_opt: Option<ResMut<DarkRiftEngine>>,
    shader_map_opt: Option<ResMut<ShaderMapData>>,
    mut images: ResMut<Assets<Image>>,
    mut last_event_idx: Local<usize>,
    sim_hud: Res<crate::SimPerfHudEnabled>,
    mut sim_perf: ResMut<crate::SimPerfDiagnostics>,
    time: Res<Time>,
    mut upload_timer: Local<f32>,
) {
    let Some(ref mut engine) = engine_opt else { return };
    let Some(mut shader_map) = shader_map_opt else { return };

    if shader_map.colors_dirty
        && let Some(colors_img) = images.get_mut(&shader_map.player_colors) {
            sync_player_colors(&engine.state, colors_img);
            shader_map.colors_dirty = false;
        }

    let current_len = engine.state.events.len();
    // `Local` survives across matches — after a huge map, idx can exceed the new game's event log.
    if *last_event_idx > current_len {
        *last_event_idx = 0;
    }
    if *last_event_idx < current_len {
        if sim_hud.0 {
            let scanned = current_len.saturating_sub(*last_event_idx);
            sim_perf.tile_handler_events_scanned = sim_perf
                .tile_handler_events_scanned
                .saturating_add(scanned);
        }
        
        let mut cap_writes = 0u32;
        let mut dirtied = false;
        for i in *last_event_idx..current_len {
            let ev = &engine.state.events[i];
            if let GameEvent::TileCaptured { x, y, new_owner: owner, .. } = ev {
                if sim_hud.0 {
                    cap_writes = cap_writes.saturating_add(1);
                }
                // Update single owner ID at (x, y) on CPU buffer
                let linear_index = (*y as usize * engine.state.map.width as usize) + *x as usize;
                let byte_offset = linear_index * 2;
                if byte_offset + 1 < shader_map.cpu_board_data.len() {
                    shader_map.cpu_board_data[byte_offset..byte_offset+2].copy_from_slice(&owner.to_ne_bytes());
                    dirtied = true;
                }
            }
        }
        
        if dirtied {
            shader_map.board_dirty = true;
        }

        if sim_hud.0 {
            sim_perf.tile_captured_writes = sim_perf
                .tile_captured_writes
                .saturating_add(cap_writes);
        }
        
        *last_event_idx = current_len;
    }

    // Throttle GPU upload to 15 FPS (66ms)
    *upload_timer += time.delta_secs();
    if shader_map.board_dirty && *upload_timer >= 0.066 {
        if let Some(board_img) = images.get_mut(&shader_map.board_data) {
            if let Some(ref mut data) = board_img.data {
                data.copy_from_slice(&shader_map.cpu_board_data);
            }
        }
        shader_map.board_dirty = false;
        *upload_timer = 0.0;
    }

    // Prevent unbounded event growth: once past 512 processed events, drain the older half.
    // This keeps memory bounded while allowing other consumers (hud_events) to catch up.
    const EVENT_DRAIN_THRESHOLD: usize = 128;
    if *last_event_idx >= EVENT_DRAIN_THRESHOLD {
        let drain_count = *last_event_idx;
        engine.state.events.drain(..drain_count);
        *last_event_idx = 0;
    }
}

pub fn spawn_shader_map(
    commands: &mut Commands,
    game: &GameState,
    images: &mut Assets<Image>,
    materials: &mut Assets<crate::rendering::BorderMaterial>,
    meshes: &mut Assets<Mesh>,
    in_game: bool,
    visuals: &crate::VisualSettingsConfig,
) -> ShaderMapData {
    let width = game.map.width;
    let height = game.map.height;

    // 1. Board Data (Owner IDs): R16Uint
    let mut board_data = vec![0u8; (width * height * 2) as usize];
    for y in 0..height {
        for x in 0..width {
            let owner = game.map.owner_id(x, y);
            let idx = ((y * width) + x) as usize * 2;
            board_data[idx..idx+2].copy_from_slice(&owner.to_ne_bytes());
        }
    }
    let mut board_img = Image::new(
        bevy::render::render_resource::Extent3d { width, height, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        board_data.clone(),
        bevy::render::render_resource::TextureFormat::R16Uint,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    board_img.sampler = bevy::image::ImageSampler::nearest();
    let board_handle = images.add(board_img);

    // 2. Player Colors: Rgba8Unorm
    let colors_data = vec![0u8; 4096 * 4];
    let mut colors_img = Image::new(
        bevy::render::render_resource::Extent3d { width: 4096, height: 1, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        colors_data,
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    colors_img.sampler = bevy::image::ImageSampler::nearest();
    sync_player_colors(game, &mut colors_img);
    let colors_handle = images.add(colors_img);

    // 3. Map Terrain: R16Uint
    let mut terrain_data = vec![0u8; (width * height * 2) as usize];
    for y in 0..height {
        for x in 0..width {
            let tile_idx = (y * width) + x;
            let raw_byte = game.map.terrain[tile_idx as usize].as_byte();
            let tile_u16 = raw_byte as u16;
            let byte_idx = tile_idx as usize * 2;
            terrain_data[byte_idx..byte_idx+2].copy_from_slice(&tile_u16.to_ne_bytes());
        }
    }
    let mut terrain_img = Image::new(
        bevy::render::render_resource::Extent3d { width, height, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        terrain_data,
        bevy::render::render_resource::TextureFormat::R16Uint,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    terrain_img.sampler = bevy::image::ImageSampler::nearest();
    let terrain_handle = images.add(terrain_img);

    // 4. Material — border darken / neighbor offset come from `settings.ron` `visuals.border.*`.
    let darken = visuals.border.darken.clamp(0.05, 1.0);
    let neighbor = visuals
        .border
        .neighbor_offset_tiles
        .clamp(1.0, 64.0);
    let material = crate::rendering::BorderMaterial {
        board_data: board_handle.clone(),
        uniforms: crate::rendering::BorderUniforms {
            border_color: LinearRgba::new(darken, darken, darken, 1.0),
            border_thickness: Vec4::new(neighbor, 0.0, 0.0, 0.0),
            texture_size: Vec4::new(width as f32, height as f32, 0.0, 0.0),
            time: Vec4::new(0.0, 0.0, 0.0, 0.0),
            enable_water_animation: UVec4::new(if visuals.enable_water_animation { 1 } else { 0 }, 0, 0, 0),
            enable_players: UVec4::new(1, 0, 0, 0),
        },
        player_colors: colors_handle.clone(),
        map_terrain: terrain_handle,
    };
    let material_handle = materials.add(material);

    let screen_w = width as f32 * TILE_SIZE;
    let screen_h = height as f32 * TILE_SIZE;
    let mesh = meshes.add(Rectangle::new(screen_w, screen_h));

    // Center the map at (screen_w / 2, screen_h / 2) because coordinates are from (0,0) bottom-left.
    let offset_x = screen_w / 2.0 - TILE_SIZE / 2.0;
    let offset_y = screen_h / 2.0 - TILE_SIZE / 2.0;

    let mut ecmd = commands.spawn((
        Mesh2d(mesh),
        bevy::sprite_render::MeshMaterial2d(material_handle.clone()),
        Transform::from_xyz(offset_x, offset_y, 0.0),
    ));
    if in_game {
        ecmd.insert(InGameEntity);
    }

    ShaderMapData {
        board_data: board_handle,
        player_colors: colors_handle,
        colors_dirty: false,
        cpu_board_data: board_data,
        board_dirty: false,
        border_material: material_handle,
    }
}

pub fn sync_player_colors(game: &GameState, image: &mut Image) {
    if let Some(ref mut data) = image.data {
        for p in &game.players {
            let idx = (p.id & 0x0FFF) as usize;
            let byte_offset = idx * 4;
            if byte_offset + 3 < data.len() {
                let rgba = bevy::color::Color::srgb(p.color[0], p.color[1], p.color[2]).to_srgba();
                data[byte_offset] = (rgba.red * 255.0) as u8;
                data[byte_offset+1] = (rgba.green * 255.0) as u8;
                data[byte_offset+2] = (rgba.blue * 255.0) as u8;
                data[byte_offset+3] = (rgba.alpha * 255.0) as u8;
            }
        }
    }
}

pub fn update_water_animation_time(
    time: Res<Time>,
    settings: Res<ClientSettings>,
    shader_map: Option<Res<ShaderMapData>>,
    mut materials: ResMut<Assets<crate::rendering::BorderMaterial>>,
) {
    if !settings.config.visuals.enable_water_animation {
        return;
    }
    let t = time.elapsed_secs();
    if let Some(sm) = shader_map.as_ref()
        && let Some(m) = materials.get_mut(&sm.border_material) {
            m.uniforms.time.x = t;
        }
}

pub fn system_update_scroll_position(
    mut mouse_wheel_reader: MessageReader<bevy::input::mouse::MouseWheel>,
    hover_map: Res<bevy::picking::hover::HoverMap>,
    parent_query: Query<&ChildOf>,
    mut scrolled: Query<(&mut bevy::ui::ScrollPosition, &bevy::ui::ComputedNode)>,
    keyboard: Res<ButtonInput<KeyCode>>,
) {
    for event in mouse_wheel_reader.read() {
        let (mut dx, mut dy) = match event.unit {
            bevy::input::mouse::MouseScrollUnit::Line  => (event.x * 45., event.y * 45.),
            bevy::input::mouse::MouseScrollUnit::Pixel => (event.x * 2., event.y * 2.),
        };
        if keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            std::mem::swap(&mut dx, &mut dy);
        }

        let mut scroll_targets = std::collections::HashSet::<Entity>::new();
        use bevy::ecs::relationship::Relationship;
        for pointer_map in hover_map.values() {
            for entity in pointer_map.keys().copied() {
                let mut current = entity;
                loop {
                    if scrolled.get(current).is_ok() {
                        scroll_targets.insert(current);
                        break;
                    }
                    let Some(parent) = parent_query.get(current).ok().map(|c| c.get()) else {
                        break;
                    };
                    current = parent;
                }
            }
        }

        for entity in scroll_targets {
            if let Ok((mut pos, node)) = scrolled.get_mut(entity) {
                let range = (node.content_size() - node.size()).max(Vec2::ZERO)
                    * node.inverse_scale_factor();
                pos.x = (pos.x - dx).clamp(0., range.x);
                pos.y = (pos.y - dy).clamp(0., range.y);
            }
        }
    }
}