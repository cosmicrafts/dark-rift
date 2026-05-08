use crate::poi_extractor::POISpawn;
use dark_rift_shared::map::MapTile;
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn export_map(
    map_name: &str,
    width: u32,
    height: u32,
    terrain: Vec<MapTile>,
    spawns: Vec<POISpawn>,
    single_player_config: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = format!("assets/maps/{}", map_name);
    fs::create_dir_all(&output_dir)?;

    // 1. Write map.bin
    let bin_path = Path::new(&output_dir).join("map.bin");
    let bin_data: Vec<u8> = terrain.iter().map(|t| t.as_byte()).collect();
    fs::write(&bin_path, bin_data)?;

    // 2. Write info.json / manifest.json
    let nations: Vec<serde_json::Value> = spawns.iter().map(|spawn| {
        json!({
            "name": spawn.name,
            "coordinates": [spawn.x, spawn.y],
            "flag": "xx" // Placeholder flag for bots
        })
    }).collect();

    // Map the manifest format used by Openfront
    let manifest = json!({
        "name": map_name,
        "map": {
            "width": width,
            "height": height,
            "num_land_tiles": width * height // Approximation for now
        },
        "map16x": { "width": width / 16, "height": height / 16, "num_land_tiles": 0 },
        "map4x": { "width": width / 4, "height": height / 4, "num_land_tiles": 0 },
        "nations": nations
    });

    let manifest_path = Path::new(&output_dir).join("manifest.json");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;

    if single_player_config {
        let ron_path = "crates/client/assets/configs/default_single_player.ron";
        let bot_count = spawns.len();
        let ron_content = format!(
r#"(
    max_players: 1,
    bot_count: {bot_count},
    map_name: "{map_name}",
    map_width: {width},
    map_height: {height},
    random_spawn: false,
)"#
        );
        fs::write(ron_path, ron_content)?;
        println!("📝 Wrote default_single_player.ron for {} with {} bots", map_name, bot_count);
    }

    Ok(())
}
