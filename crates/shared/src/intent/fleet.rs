use crate::engine::DarkRiftEngine;
use crate::warp_fleet::{WarpFleet};
use crate::warp_fleet::{FleetRoute, resolve_fleet_route};
use crate::config;

impl DarkRiftEngine {
pub(super) fn apply_launch_fleet_intent(
    &mut self,
    player_id: u16,
    target_tile: u32,
    troops: Option<f64>,
) {
    let Some(player) = self.state.player(player_id) else {
        bevy::log::warn!("apply_launch_fleet_intent: player {} not found", player_id);
        return;
    };
    if !player.alive {
        return;
    }
    let map_area = self.state.map.width.saturating_mul(self.state.map.height);
    if map_area == 0 || target_tile >= map_area {
        bevy::log::warn!(
            "apply_launch_fleet_intent: out-of-range target_tile={} (player {})",
            target_tile, player_id
        );
        return;
    }

    let target_owner = self.state.map.owner_id(target_tile % self.state.map.width, target_tile / self.state.map.width);

    if target_owner == player_id {
        return;
    }

    let border_tiles = match self.state.player(player_id) {
        Some(p) => &p.border_tiles,
        None => return,
    };

    let target_border: Option<&crate::bitset::DenseBitSet> = if target_owner != 0 {
        match self.state.player(target_owner) {
            Some(p) => Some(&p.border_tiles),
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
        &self.state.map,
        &self.water,
        &mut self.path_scratch,
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

    let Some(p) = self.state.player_mut(player_id) else {
        return;
    };
    p.troops -= launch;
    p.troops = p.troops.max(0.0);

    let fid = self.state.next_fleet_id;
    self.state.next_fleet_id = self.state.next_fleet_id.wrapping_add(1).max(1);

    self.add_fleet(WarpFleet::new(
        fid,
        player_id,
        target_owner,
        launch,
        src,
        landing,
        path,
    ));
}
}

