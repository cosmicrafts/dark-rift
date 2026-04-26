use crate::game::BuildingKind;
use crate::config;
use super::core::Building;
/// Count buildings owned by `owner` of exact `kind` (for City-only scaling).
pub fn count_kind(buildings: &[Building], owner_id: u16, kind: BuildingKind) -> u32 {
    buildings
        .iter()
        .filter(|b| b.owner_id == owner_id && b.kind == kind)
        .count() as u32
}

/// Count Port + Factory for shared exponential scaling (OpenFront `costWrapper` for Port/Factory).
pub fn count_port_factory(buildings: &[Building], owner_id: u16) -> u32 {
    buildings
        .iter()
        .filter(|b| {
            b.owner_id == owner_id
                && matches!(b.kind, BuildingKind::Port | BuildingKind::Factory)
        })
        .count() as u32
}

#[inline]
pub fn scaled_pow2_cost(count: u32, base: u64, cap: u64, scale: f64) -> f64 {
    let steps = count.min(20);
    let mut val = base;
    for _ in 0..steps {
        val = val.saturating_mul(2);
        if val >= cap {
            val = cap;
            break;
        }
    }
    (val.min(cap)) as f64 / scale
}

/// Gold price for a new structure or one upgrade level (OpenFront-style scaling). Tunable.
pub fn structure_build_cost_gold(kind: BuildingKind, owner_id: u16, buildings: &[Building]) -> f64 {
    let s = config::OPENFRONT_GOLD_SCALE.max(1.0);
    match kind {
        BuildingKind::City => {
            let n = count_kind(buildings, owner_id, BuildingKind::City);
            scaled_pow2_cost(n, 125_000, 1_000_000, s)
        }
        BuildingKind::Factory | BuildingKind::Port => {
            let n = count_port_factory(buildings, owner_id);
            scaled_pow2_cost(n, 125_000, 1_000_000, s)
        }
        BuildingKind::DefensePost => {
            let n = count_kind(buildings, owner_id, BuildingKind::DefensePost);
            ((n as f64 + 1.0) * (50_000.0 / s)).min(250_000.0 / s)
        }
        BuildingKind::SamLauncher => {
            let n = count_kind(buildings, owner_id, BuildingKind::SamLauncher);
            ((n as f64 + 1.0) * (1_500_000.0 / s)).min(3_000_000.0 / s)
        }
        BuildingKind::MissileSilo => 1_000_000.0 / s,
    }
}

#[inline]
pub fn structure_kind_enabled(kind: BuildingKind) -> bool {
    match kind {
        BuildingKind::SamLauncher | BuildingKind::MissileSilo => config::ENABLE_MISSILE_STRUCTURES,
        _ => true,
    }
}

/// Sum of `level` for ready (not under construction) buildings of `kind` for `owner_id`.
pub fn count_kind_levels(buildings: &[Building], owner_id: u16, kind: BuildingKind) -> u32 {
    buildings
        .iter()
        .filter(|b| b.owner_id == owner_id && b.kind == kind && !b.under_construction)
        .map(|b| b.level as u32)
        .sum()
}

/// At least one **ready** Port owned by `player_id` (fleet launches require this).
#[inline]
pub fn player_has_completed_port(buildings: &[Building], player_id: u16) -> bool {
    buildings.iter().any(|b| {
        b.owner_id == player_id && b.kind == BuildingKind::Port && !b.under_construction
    })
}
