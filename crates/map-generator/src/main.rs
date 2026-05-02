use clap::Parser;
use std::error::Error;

mod overpass;
mod rasterizer;
mod poi_extractor;
mod exporter;

/// Dark Rift Automated Map Generator
/// Fetches OpenStreetMap data for a bounding box and generates a playable map.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Bounding box (min_lon, min_lat, max_lon, max_lat)
    #[arg(short, long, allow_hyphen_values = true)]
    pub bbox: String,

    /// Output map name (e.g., 'guadalajara')
    #[arg(short, long)]
    pub name: String,

    /// Scale factor (how many pixels per degree of longitude)
    #[arg(short, long, default_value_t = 1000.0)]
    pub scale: f64,

    /// Automatically generate assets/configs/default_single_player.ron for this map
    #[arg(long, default_value_t = false)]
    pub single_player_config: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    
    // Parse bbox: "min_lon,min_lat,max_lon,max_lat"
    let parts: Vec<&str> = args.bbox.split(',').collect();
    if parts.len() != 4 {
        return Err("Bounding box must be in format 'min_lon,min_lat,max_lon,max_lat'".into());
    }

    let min_lon: f64 = parts[0].parse()?;
    let min_lat: f64 = parts[1].parse()?;
    let max_lon: f64 = parts[2].parse()?;
    let max_lat: f64 = parts[3].parse()?;

    println!("🌍 Generating map '{}' for bbox [{}, {}, {}, {}]", args.name, min_lon, min_lat, max_lon, max_lat);
    
    // 1. Fetch from Overpass
    println!("📡 Fetching data from OpenStreetMap (Overpass API)...");
    let overpass_data = overpass::fetch_bbox(min_lon, min_lat, max_lon, max_lat).await?;
    
    // 2. Rasterize Map
    println!("🗺️  Rasterizing terrain...");
    let (map_width, map_height, terrain_grid) = rasterizer::rasterize_map(&overpass_data, min_lon, min_lat, max_lon, max_lat, args.scale);
    
    // 3. Extract POIs (Bots/Tribes)
    println!("🤖 Extracting points of interest for bots...");
    let spawns = poi_extractor::extract_bots(&overpass_data, min_lon, min_lat, max_lon, max_lat, args.scale, map_height);
    
    // 4. Export
    println!("💾 Exporting files...");
    exporter::export_map(&args.name, map_width, map_height, terrain_grid, spawns, args.single_player_config)?;

    println!("✅ Generation complete! Saved to assets/maps/{}", args.name);

    Ok(())
}
