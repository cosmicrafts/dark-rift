use bevy::prelude::*;
use dark_rift_shared::game::GameState;

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
    mut cam_q: Query<&mut Transform, With<Camera2d>>,
    mut params: StartGameParams,
) {
    timeout.0.tick(time.delta());
    if timeout.0.just_finished() {
        bevy::log::error!("Map load timeout! Returning to lobby.");
        params.next_state.set(AppState::LobbyBrowser);
        commands.remove_resource::<PendingServerStart>();
        return;
    }

    let Some(mut p_msg) = pending_msg else { return; };

    // Ensure MapCache exists
    if cache.is_none() {
        commands.insert_resource(MapCache::default());
        return; // Wait one frame for it to be available
    }
    let cache = cache.as_mut().unwrap();

    let map_name = &p_msg.0.config.map_name;

    // If cache doesn't have the right map, request it
    if !cache.is_ready() || cache.map_name != *map_name {
        if !cache.is_fetching() || cache.map_name != *map_name {
            cache.request(map_name, &mut commands);
        }
        return; // Wait for fetch to complete
    }

    // Map is ready — start the game
    let cached_bytes = cache.take_data();
    p_msg.0.map_data = cached_bytes;

    start_game_core(
        p_msg.0.clone(),
        &mut cam_q,
        &mut commands,
        &mut params,
    );

    commands.remove_resource::<PendingServerStart>();
}

pub fn handle_tile_change(
    game_opt: Option<ResMut<GameState>>,
    shader_map_opt: Option<ResMut<ShaderMapData>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut game) = game_opt else { return };
    if !game.is_changed() { return; }
    let Some(mut shader_map) = shader_map_opt else { return };

    if shader_map.colors_dirty {
        if let Some(colors_img) = images.get_mut(&shader_map.player_colors) {
            sync_player_colors(&game, colors_img);
            shader_map.colors_dirty = false;
        }
    }

    if let Some(board_img) = images.get_mut(&shader_map.board_data) {
        if let Some(ref mut data) = board_img.data {
            for ev in &game.events {
                if let GameEvent::TileCaptured { x, y, new_owner: owner, .. } = ev {
                    // Update single owner ID at (x, y)
                    let linear_index = (*y as usize * game.map.width as usize) + *x as usize;
                    let byte_offset = linear_index * 2;
                    if byte_offset + 1 < data.len() {
                        data[byte_offset..byte_offset+2].copy_from_slice(&owner.to_ne_bytes());
                    }
                }
            }
        }
    }

    game.events.clear();
}

pub fn spawn_shader_map(
    commands: &mut Commands,
    game: &GameState,
    images: &mut Assets<Image>,
    materials: &mut Assets<crate::rendering::BorderMaterial>,
    meshes: &mut Assets<Mesh>,
    in_game: bool,
) -> ShaderMapData {
    let width = game.map.width;
    let height = game.map.height;

    // 1. Board Data (Owner IDs): R16Uint
    let mut board_data = vec![0u8; (width * height * 2) as usize];
    for y in 0..height {
        for x in 0..width {
            let owner = game.map.owner_id(x, y) as u16;
            let idx = ((y * width) + x) as usize * 2;
            board_data[idx..idx+2].copy_from_slice(&owner.to_ne_bytes());
        }
    }
    let mut board_img = Image::new(
        bevy::render::render_resource::Extent3d { width, height, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        board_data,
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

    // 4. Material
    let material = crate::rendering::BorderMaterial {
        board_data: board_handle.clone(),
        uniforms: crate::rendering::BorderUniforms {
            border_color: LinearRgba::new(0.6, 0.6, 0.6, 1.0),
            border_thickness: Vec4::new(1.0, 0.0, 0.0, 0.0),
            texture_size: Vec4::new(width as f32, height as f32, 0.0, 0.0),
            time: Vec4::new(0.0, 0.0, 0.0, 0.0),
            enable_water_animation: UVec4::new(1, 0, 0, 0),
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
        Mesh2d(mesh.into()),
        bevy::sprite_render::MeshMaterial2d(material_handle),
        Transform::from_xyz(offset_x, offset_y, 0.0),
    ));
    if in_game {
        ecmd.insert(InGameEntity);
    }

    ShaderMapData {
        board_data: board_handle,
        player_colors: colors_handle,
        colors_dirty: false,
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
    mut materials: ResMut<Assets<crate::rendering::BorderMaterial>>,
) {
    for (_, material) in materials.iter_mut() {
        material.uniforms.time.x = time.elapsed_secs();
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