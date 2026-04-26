use bevy::prelude::*;
use crate::game::GameState;
use crate::warp_fleet::{WarpFleet};
use crate::water_components::WaterComponents;
use crate::pathfinding::WaterPathfinderScratch;
use crate::warp_fleet::{FleetRoute, resolve_fleet_route};
use crate::building::Building;
use crate::config;

pub fn apply_launch_fleet_intent(
    commands: &mut Commands,
    game: &mut GameState,
    path_scratch: &mut WaterPathfinderScratch,
    water_components: &WaterComponents,
    _q_fleet: &mut Query<(Entity, &mut WarpFleet)>,
    player_id: u16,
    target_tile: u32,
    troops: Option<f64>,
    _buildings: &[Building],
) {
    let Some(player) = game.player(player_id) else {
        bevy::log::warn!("apply_launch_fleet_intent: player {} not found", player_id);
        return;
    };
    if !player.alive {
        return;
    }
    let map_area = game.map.width.saturating_mul(game.map.height);
    if map_area == 0 || target_tile >= map_area {
        bevy::log::warn!(
            "apply_launch_fleet_intent: out-of-range target_tile={} (player {})",
            target_tile, player_id
        );
        return;
    }

    let target_owner = game.map.owner_id(target_tile % game.map.width, target_tile / game.map.width);

    if target_owner == player_id {
        return;
    }

    let border_tiles: &[u32] = match game.player(player_id) {
        Some(p) => p.border_tiles.as_slice(),
        None => return,
    };

    let target_border: Option<&[u32]> = if target_owner != 0 {
        match game.player(target_owner) {
            Some(p) => Some(p.border_tiles.as_slice()),
            None => {
                bevy::log::warn!(
                    "apply_launch_fleet_intent: target_owner={} not found (player {})",
                    target_owner, player_id
                );
                return;
            }
        }
    } else {
        None
    };

    let route = match resolve_fleet_route(
        &game.map,
        water_components,
        path_scratch,
        player_id,
        target_owner,
        target_tile,
        border_tiles,
        target_border,
        true,
    ) {
        Ok(r) => r,
        Err(e) => {
            bevy::log::warn!(
                "apply_launch_fleet_intent: {} (player {}, target_owner={}, tile={})",
                e,
                player_id,
                target_owner,
                target_tile
            );
            return;
        }
    };

    let pool_cap = player.troops;
    let launch = troops.unwrap_or(pool_cap).max(0.0).min(pool_cap);
    if launch < config::EXPAND_COST {
        return;
    }

    let FleetRoute {
        src_tile: src,
        landing_tile: landing,
        path,
    } = route;

    let Some(p) = game.player_mut(player_id) else {
        return;
    };
    p.troops -= launch;
    p.troops = p.troops.max(0.0);

    let fid = game.next_fleet_id;
    game.next_fleet_id = game.next_fleet_id.wrapping_add(1).max(1);

    commands.spawn(WarpFleet::new(
        fid,
        player_id,
        target_owner,
        launch,
        src,
        landing,
        path,
    ));
}

