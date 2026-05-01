use crate::ReplayDirector;
use crate::Cli;

use dark_rift_shared::game::*;
use dark_rift_shared::engine::DarkRiftEngine;
use crate::{ClientSettings, TILE_SIZE};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::{Bloom, BloomCompositeMode};
use dark_rift_shared::player::Player;
use dark_rift_shared::water_components::WaterComponents;
use crate::environment::spawn_shader_map;

use bevy::prelude::*;
use crate::state::*;
use crate::ui::*;

pub fn auto_inject_visibility(
    mut commands: Commands,
    q: Query<Entity, (Or<(With<Transform>, With<Node>)>, Without<InheritedVisibility>)>,
) {
    for entity in q.iter() {
        commands.entity(entity).insert((
            Visibility::default(),
            InheritedVisibility::default(),
            ViewVisibility::default(),
        ));
    }
}

pub fn setup_offline(
    mut commands: Commands,
    mut ready: ResMut<GameReady>,
    director: NonSend<ReplayDirector>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::rendering::BorderMaterial>>,
    mut images: ResMut<Assets<Image>>,
    settings: Res<ClientSettings>,
    mut overlay: ResMut<OverlayDiagnostics>,
    shell: Res<HudShellRegions>,
) {
    let mut cam = commands.spawn((
        Camera2d,
        Tonemapping::TonyMcMapface,
        Transform::from_xyz(
            director.record.config.map_width as f32 * TILE_SIZE / 2.0,
            director.record.config.map_height as f32 * TILE_SIZE / 2.0,
            999.0
        ),
    ));
    if settings.config.visuals.hdr_bloom_enabled() {
        cam.insert(Bloom {
            intensity: 0.15,
            composite_mode: BloomCompositeMode::Additive,
            ..default()
        });
    }

    overlay.status_line = "Replay Mode".to_string();
    overlay.ping_value = "offline".to_string();



    commands
        .spawn((
            ChildOf(shell.top_bar),
            Node {
                align_self: AlignSelf::FlexStart,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(12.0),
                ..default()
            },
        ))
        .with_children(|right_group| {
            // Dev Diagnostics (FPS, Ping, Status) Pill
            right_group.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.8)),
            )).with_children(|pill| {
                pill.spawn((
                    FpsText,
                    Text::new("Connecting…"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.4, 0.9, 0.4, 1.0)),
                ));
            });

            // EXIT Button
            right_group.spawn((
                Button,
                ExitButton,
                Node {
                    width: Val::Px(120.0),
                    height: Val::Px(48.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.8, 0.2, 0.2, 1.0)),
                BorderColor::all(Color::srgba(1.0, 0.3, 0.3, 1.0)),
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("EXIT"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 20.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            });
        });

    let mut game = GameState::new(director.record.seed, director.record.config.map_width, director.record.config.map_height, director.record.config.clone());
    for info in &director.record.players {
        let player = Player::new_human(info.id, info.name.clone(), info.color, &director.record.config);
        game.spawn_player(player, info.spawn_x, info.spawn_y);
    }

    let grid = spawn_shader_map(&mut commands, &game, &mut images, &mut materials, &mut meshes, false);
    commands.insert_resource(grid);

    let water = WaterComponents::compute(&game.map);
    commands.insert_resource(DarkRiftEngine::new(game, water));
    ready.0 = true;
}

pub fn setup_live(
    mut commands: Commands,
    _cli: Res<Cli>,
    settings: Res<ClientSettings>,
    asset_server: Res<AssetServer>,
    mut overlay: ResMut<OverlayDiagnostics>,
    shell: Res<HudShellRegions>,
) {
    let mut cmd = commands.spawn((
        Camera2d,
        Tonemapping::TonyMcMapface,
        Transform::from_xyz(0.0, 0.0, 999.0),
    ));
    
    if settings.config.visuals.hdr_bloom_enabled() {
        cmd.insert(Bloom {
            intensity: 0.15,
            composite_mode: BloomCompositeMode::Additive,
            ..default()
        });
    }

    overlay.status_line = "Connecting to server...".to_string();
    overlay.ping_value = "connecting…".to_string();

    commands
        .spawn((
            ChildOf(shell.top_bar),
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                margin: UiRect::left(Val::Auto),
                column_gap: Val::Px(12.0),
                ..default()
            },
        ))
        .with_children(|right_group| {
            // Dev Diagnostics (FPS, Ping, Status) Pill
            right_group.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.8)),
            )).with_children(|pill| {
                pill.spawn((
                    FpsText,
                    Text::new("Connecting…"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.4, 0.9, 0.4, 1.0)),
                ));
            });

            // Game Timer Pill
            right_group.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.8)),
            )).with_children(|pill| {
                pill.spawn((
                    GameTimerText,
                    Text::new("00:00"),
                    TextFont {
                        font: settings.font.clone(),
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgba(0.9, 0.9, 0.9, 1.0)),
                ));
            });

            // SETTINGS Button
            right_group.spawn((
                Button,
                SettingsButton,
                Node {
                    width: Val::Px(48.0),
                    height: Val::Px(48.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(panel_bg()),
                BorderColor::all(crate::ui::theme::panel_border()),
            ))
            .with_children(|b| {
                b.spawn((
                    ImageNode::new(asset_server.load("icons/home.png")),
                    Node {
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        ..default()
                    },
                ));
            });

            // EXIT Button
            right_group.spawn((
                Button,
                ExitButton,
                Node {
                    width: Val::Px(48.0),
                    height: Val::Px(48.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(crate::ui::theme::accent_danger()),
                BorderColor::all(crate::ui::theme::panel_border()),
            ))
            .with_children(|b| {
                b.spawn((
                    ImageNode::new(asset_server.load("icons/exit.png")),
                    Node {
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        ..default()
                    },
                ));
            });
        });
}

