use crate::map::GameMap;
use crate::game::BuildingKind;
use super::core::Building;
use super::placement::{idx_xy, manhattan, STRUCTURE_MIN_DIST};
/// OpenFront `findExistingUnitToUpgrade`: closest structure of `kind` within Manhattan
/// `STRUCTURE_MIN_DIST` of `click_idx`.
pub fn find_upgrade_target_id(
    map: &GameMap,
    owner_id: u16,
    kind: BuildingKind,
    click_idx: u32,
    buildings: &[Building],
) -> Option<u64> {
    let w = map.width;
    let (cx, cy) = idx_xy(click_idx, w);
    let mut best: Option<(i32, u64)> = None;
    for b in buildings {
        if b.owner_id != owner_id || b.kind != kind {
            continue;
        }
        if b.under_construction {
            continue;
        }
        if !kind.upgradable() {
            continue;
        }
        let (bx, by) = idx_xy(b.tile_idx, w);
        let d = manhattan(cx as i32, cy as i32, bx as i32, by as i32);
        if d > STRUCTURE_MIN_DIST {
            continue;
        }
        let cand = (d, b.id);
        match best {
            None => best = Some(cand),
            Some((bd, bid)) => {
                if d < bd || (d == bd && b.id < bid) {
                    best = Some(cand);
                }
            }
        }
    }
    best.map(|(_, id)| id)
}
