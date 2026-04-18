use serde::Deserialize;
use std::fs;
use std::path::Path;
use crate::map::GameMap;

#[derive(Deserialize, Debug)]
pub struct MapManifestDimensions {
    pub height: u32,
    pub width: u32,
    pub num_land_tiles: u32,
}

#[derive(Deserialize, Debug)]
pub struct MapNationSpawn {
    pub coordinates: [u32; 2],
    pub flag: String,
    pub name: String,
}

#[derive(Deserialize, Debug)]
pub struct MapManifest {
    pub map: MapManifestDimensions,
    pub map16x: MapManifestDimensions,
    pub map4x: MapManifestDimensions,
    pub name: String,
    pub nations: Vec<MapNationSpawn>,
}

pub fn load_openfront_map(map_name: &str) -> Option<(GameMap, MapManifest)> {
    let manifest_path_str = format!("assets/maps/{}/manifest.json", map_name);
    let bin_path_str = format!("assets/maps/{}/map.bin", map_name);

    let manifest_path = Path::new(&manifest_path_str);
    let bin_path = Path::new(&bin_path_str);

    if !manifest_path.exists() || !bin_path.exists() {
        return None;
    }

    let manifest_str = fs::read_to_string(manifest_path).ok()?;
    let manifest: MapManifest = serde_json::from_str(&manifest_str).ok()?;

    let bin_data = fs::read(bin_path).ok()?;

    if bin_data.len() != (manifest.map.width * manifest.map.height) as usize {
        return None;
    }

    let mut game_map = GameMap::new(manifest.map.width, manifest.map.height);
    game_map.terrain = bin_data.into_iter().map(crate::map::MapTile::from_byte).collect();

    Some((game_map, manifest))
}
