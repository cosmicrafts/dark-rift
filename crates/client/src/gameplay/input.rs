use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use dark_rift_shared::game::GameState;

use dark_rift_shared::building::{
    find_upgrade_target_id, resolve_structure_spawn_tile, structure_build_cost_gold,
    existing_structure_positions_with_width, Building,
};
use dark_rift_shared::config;
use dark_rift_shared::protocol::{ClientGameplayMessage, GameplayIntent, AttackIntent};
use lightyear::prelude::MessageSender;
use dark_rift_shared::water_components::WaterComponents;
use dark_rift_shared::pathfinding::WaterPathfinderScratch;
use dark_rift_shared::warp_fleet::{shares_border_with, player_water_components};

use crate::{
    MyPlayerId, AttackRatio, LocalIntentBuffer, BuildStructureMode,
    CameraDragState, TouchGestureState, TILE_SIZE, 
    ActionDockPointerSink, BottomHudRoot, ClientSettings,
    AttackRingMesh,
    try_launch_fleet_intent, spawn_attack_command_ring, send_gameplay_intent,
};

#[derive(Resource, Default)]
pub struct HoveredTile(pub Option<(u32, u32)>);

pub fn clear_inputs_on_focus_loss(
    mut events: MessageReader<bevy::window::WindowFocused>,
    mut kb:     ResMut<ButtonInput<KeyCode>>,
) {
    for ev in events.read() {
        if !ev.focused {
            kb.clear();
        }
    }
}

pub fn hover_system(
    windows:     Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera_q:    Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut hovered: ResMut<HoveredTile>,
    game_opt:    Option<Res<GameState>>,
) {
    hovered.0 = None;
    let Some(game) = game_opt else { return };
    let Ok(window) = windows.single() else { return };
    let Ok((camera, cam_t)) = camera_q.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok(world) = camera.viewport_to_world_2d(cam_t, cursor) else { return };

    let gx = (world.x / TILE_SIZE).round() as i32;
    let gy = game.map.height as i32 - 1 - (world.y / TILE_SIZE).round() as i32;
    if game.map.is_valid_coord(gx, gy) {
        hovered.0 = Some((gx as u32, gy as u32));
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct ForceBoatKeyCtx<'w> {
    pub game_opt: Option<Res<'w, GameState>>,
    pub my_id: Res<'w, MyPlayerId>,
    pub attack_ratio: Res<'w, AttackRatio>,
    pub components: Option<Res<'w, WaterComponents>>,
    pub path_scratch: ResMut<'w, WaterPathfinderScratch>,
}

pub fn system_force_boat_key(
    kb: Res<ButtonInput<KeyCode>>,
    hovered: Res<HoveredTile>,
    mut hud_hit: ParamSet<(
        Query<&RelativeCursorPosition, With<BottomHudRoot>>,
        Query<&RelativeCursorPosition, With<ActionDockPointerSink>>,
    )>,
    mut commands: Commands,
    ring_mesh: Res<AttackRingMesh>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    settings: Res<ClientSettings>,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
    local_buf: Option<ResMut<LocalIntentBuffer>>,
    mut ctx: ForceBoatKeyCtx,
) {
    if !kb.just_pressed(KeyCode::KeyB) {
        return;
    }
    let over_hud = hud_hit.p0().iter().any(|rcp| rcp.cursor_over)
        || hud_hit.p1().iter().any(|rcp| rcp.cursor_over);
    if over_hud {
        return;
    }
    let Some(game) = ctx.game_opt.as_deref() else {
        return;
    };
    let Some((x, y)) = hovered.0 else {
        bevy::log::warn!("force boat (B): no tile under cursor");
        return;
    };
    let pid = ctx.my_id.0;
    if pid == 0 {
        return;
    }
    let Some(player) = game.player(pid) else {
        return;
    };
    if !player.alive || player.troops < config::EXPAND_COST {
        return;
    }
    let target_owner = game.map.owner_id(x, y);
    if target_owner == pid {
        return;
    }
    let target_tile = y * game.map.width + x;
    let ratio = ctx.attack_ratio.0.clamp(0.01, 1.0) as f64;
    let send = (player.troops * ratio).floor().max(0.0).min(player.troops);
    if send < config::EXPAND_COST {
        return;
    }
    let Some(wc) = ctx.components.as_deref() else {
        bevy::log::warn!("force boat (B): WaterComponents not loaded");
        return;
    };
    let payload_troops = if (ratio - 1.0).abs() < f64::EPSILON {
        None
    } else {
        Some(send)
    };
    let Some(intent) = try_launch_fleet_intent(
        game,
        wc,
        &mut *ctx.path_scratch,
        pid,
        target_owner,
        target_tile,
        payload_troops,
        "KeyB force boat",
    ) else {
        return;
    };
    spawn_attack_command_ring(
        &mut commands,
        &ring_mesh,
        &mut materials,
        &settings,
        player.color,
        x,
        y,
        game.map.height,
        send,
    );
    send_gameplay_intent(intent, pid, local_buf, &mut senders);
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct ClickCtx<'w> {
    pub game_opt: Option<Res<'w, GameState>>,
    pub my_id: Res<'w, MyPlayerId>,
    pub attack_ratio: Res<'w, AttackRatio>,
    pub components: Option<Res<'w, WaterComponents>>,
    pub path_scratch: ResMut<'w, WaterPathfinderScratch>,
}

pub fn player_click(
    mut commands: Commands,
    ring_mesh: Res<AttackRingMesh>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mouse:       Res<ButtonInput<MouseButton>>,
    touches:     Res<Touches>,
    q_camera:    Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    drag_state:  Res<CameraDragState>,
    touch_gesture: Res<TouchGestureState>,
    hovered:     Res<HoveredTile>,
    settings:    Res<ClientSettings>,
    mut hud_hit: ParamSet<(
        Query<&RelativeCursorPosition, With<BottomHudRoot>>,
        Query<&RelativeCursorPosition, With<ActionDockPointerSink>>,
    )>,
    mut senders: Query<&mut MessageSender<ClientGameplayMessage>>,
    local_buf: Option<ResMut<LocalIntentBuffer>>,
    build_mode: Res<BuildStructureMode>,
    q_building: Query<(Entity, &Building)>,
    mut ctx: ClickCtx,
) {
    // We trigger ONLY on release to support dragging.
    // Additionally, if the user accumulated too much drag, we abort the intent payload.
    let left_mouse_release = mouse.just_released(MouseButton::Left);
    let right_mouse_release = mouse.just_released(MouseButton::Right);
    let touch_release = touches.any_just_released();
    if !left_mouse_release && !right_mouse_release && !touch_release {
        return;
    }
    let wants_force_boat = right_mouse_release && !touch_release;
    if touch_release && touch_gesture.max_touches_in_gesture > 1 {
        return; // Ignore pinch/two-finger releases.
    }
    if drag_state.accumulated_drag > 10.0 { return; } // It was a drag, not a click

    // Clicks/releases over the combat HUD must not issue map attacks (OpenFront: UI eats pointer).
    let over_hud = hud_hit.p0().iter().any(|rcp| rcp.cursor_over)
        || hud_hit.p1().iter().any(|rcp| rcp.cursor_over);
    if over_hud {
        return;
    }

    let Some(game) = ctx.game_opt.as_deref() else { return };
    let target = hovered.0.or_else(|| {
        let viewport = touch_gesture.last_touch_pos?;
        let (camera, cam_t) = q_camera.single().ok()?;
        let world = camera.viewport_to_world_2d(cam_t, viewport).ok()?;
        let gx = (world.x / TILE_SIZE).round() as i32;
        let gy = game.map.height as i32 - 1 - (world.y / TILE_SIZE).round() as i32;
        if game.map.is_valid_coord(gx, gy) {
            Some((gx as u32, gy as u32))
        } else {
            None
        }
    });
    let Some((x, y)) = target else { return };

    let pid = ctx.my_id.0;
    let Some(player) = game.player(pid) else { return };
    if !player.alive {
        return;
    }

    let target_owner = game.map.owner_id(x, y);
    let map_w = game.map.width;
    let target_tile = y * map_w + x;

    if target_owner == pid {
        if wants_force_boat {
            return;
        }
        if let Some(kind) = build_mode.0 {
            let place_release = left_mouse_release || touch_release;
            if place_release {
                let snapshot: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();
                let positions =
                    existing_structure_positions_with_width(snapshot.iter(), map_w, None);
                if let Some(bid) =
                    find_upgrade_target_id(&game.map, pid, kind, target_tile, &snapshot)
                {
                    let cost = structure_build_cost_gold(kind, pid, &snapshot);
                    if player.gold >= cost && cost.is_finite() {
                        send_gameplay_intent(
                            GameplayIntent::UpgradeStructure { building_id: bid },
                            pid,
                            local_buf,
                            &mut senders,
                        );
                    }
                    return;
                }
                if resolve_structure_spawn_tile(&game.map, pid, kind, target_tile, &positions)
                    .is_some()
                {
                    let cost = structure_build_cost_gold(kind, pid, &snapshot);
                    if player.gold >= cost && cost.is_finite() {
                        send_gameplay_intent(
                            GameplayIntent::BuildStructure {
                                kind,
                                target_tile,
                            },
                            pid,
                            local_buf,
                            &mut senders,
                        );
                    }
                    return;
                }
            }
            return;
        }
        return;
    }

    if player.troops < config::EXPAND_COST {
        return;
    }

    // OpenFront `canAttack` / `canBoatAttack` decision tree:
    let border_tiles: &[u32] = &player.border_tiles;

    let can_ground_attack = if target_owner != 0 {
        shares_border_with(&game.map, border_tiles, target_owner)
    } else {
        let scratch = &mut *ctx.path_scratch;
        dark_rift_shared::warp_fleet::can_ground_attack_neutral(
            &game.map,
            pid,
            target_tile,
            200,
            &mut scratch.bfs_queue,
            &mut scratch.bfs_visited,
            &mut scratch.bfs_stamp,
        )
    };

    let has_water_access = match ctx.components.as_deref() {
        Some(wc) => !player_water_components(&game.map, wc, pid, border_tiles).is_empty(),
        None => false,
    };

    let ratio = ctx.attack_ratio.0.clamp(0.01, 1.0) as f64;
    let send_floor = (player.troops * ratio).floor();
    let send = send_floor.max(0.0).min(player.troops);
    if send < config::EXPAND_COST {
        return;
    }

    let payload_troops = if (ratio - 1.0).abs() < f64::EPSILON {
        None
    } else {
        Some(send)
    };

    let intent = if wants_force_boat {
        let Some(wc) = ctx.components.as_deref() else {
            bevy::log::warn!("player_click: right-click boat — WaterComponents not loaded");
            return;
        };
        try_launch_fleet_intent(
            game,
            wc,
            &mut *ctx.path_scratch,
            pid,
            target_owner,
            target_tile,
            payload_troops,
            "player_click: right-click boat",
        )
    } else if can_ground_attack {
        Some(GameplayIntent::Attack(AttackIntent {
            target_owner,
            troops: payload_troops,
        }))
    } else if has_water_access {
        let Some(wc) = ctx.components.as_deref() else {
            return;
        };
        try_launch_fleet_intent(
            game,
            wc,
            &mut *ctx.path_scratch,
            pid,
            target_owner,
            target_tile,
            payload_troops,
            "player_click: auto boat",
        )
    } else {
        return;
    };

    let Some(intent) = intent else {
        return;
    };

    spawn_attack_command_ring(
        &mut commands,
        &ring_mesh,
        &mut materials,
        &settings,
        player.color,
        x,
        y,
        game.map.height,
        send,
    );
    send_gameplay_intent(intent, pid, local_buf, &mut senders);
}
