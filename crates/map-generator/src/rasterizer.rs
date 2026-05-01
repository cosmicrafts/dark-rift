use dark_rift_shared::map::MapTile;
use serde_json::Value;

pub fn rasterize_map(
    _data: &Value,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    scale: f64,
) -> (u32, u32, Vec<MapTile>) {
    let width = ((max_lon - min_lon) * scale).ceil() as u32;
    let height = ((max_lat - min_lat) * scale).ceil() as u32;
    
    // Ensure dimensions are multiples of 4 (as Openfront requests in their docs)
    let width = width - (width % 4);
    let height = height - (height % 4);

    let size = (width * height) as usize;
    let grid = vec![MapTile::from_byte(0b10000000); size]; // Default to Land (bit 7 set)

    // TODO: Implement full polygon rasterization here
    // For now, we return a solid landmass. A full polygon-fill algorithm 
    // requires iterating over all "ways" and "relations" in OSM data, projecting them 
    // to map coordinates, and using a scanline fill algorithm to set `MapTile` to Water.
    
    (width, height, grid)
}
