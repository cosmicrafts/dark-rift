use serde_json::Value;

pub struct POISpawn {
    pub name: String,
    pub x: u32,
    pub y: u32,
}

pub fn extract_bots(
    data: &Value,
    min_lon: f64,
    _min_lat: f64,
    _max_lon: f64,
    max_lat: f64,
    scale: f64,
    _map_height: u32,
) -> Vec<POISpawn> {
    let mut spawns = Vec::new();

    if let Some(elements) = data.get("elements").and_then(|e| e.as_array()) {
        for element in elements {
            if element.get("type").and_then(|t| t.as_str()) == Some("node") {
                let tags = element.get("tags");
                let place = tags.and_then(|t| t.get("place")).and_then(|p| p.as_str());
                let name = tags.and_then(|t| t.get("name")).and_then(|n| n.as_str());
                
                if let (Some(place_type), Some(poi_name)) = (place, name) {
                    if place_type == "city" || place_type == "suburb" {
                        if let (Some(lat), Some(lon)) = (
                            element.get("lat").and_then(|v| v.as_f64()),
                            element.get("lon").and_then(|v| v.as_f64()),
                        ) {
                            let x = ((lon - min_lon) * scale).round() as u32;
                            // Latitudes increase northwards, but image Y increases southwards
                            let y = ((max_lat - lat) * scale).round() as u32;

                            spawns.push(POISpawn {
                                name: poi_name.to_string(),
                                x,
                                y,
                            });
                        }
                    }
                }
            }
        }
    }

    spawns
}
