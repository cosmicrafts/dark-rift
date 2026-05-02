use reqwest::Client;
use serde_json::Value;
use std::error::Error;

/// Fetches OSM data within the bounding box
pub async fn fetch_bbox(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Result<Value, Box<dyn Error>> {
    let query = format!(
        r#"[out:json][timeout:25];
        (
          way["natural"="water"]({min_lat},{min_lon},{max_lat},{max_lon});
          relation["natural"="water"]({min_lat},{min_lon},{max_lat},{max_lon});
          
          node["place"="city"]({min_lat},{min_lon},{max_lat},{max_lon});
          node["place"="suburb"]({min_lat},{min_lon},{max_lat},{max_lon});
        );
        out body;
        >;
        out skel qt;"#
    );

    let client = Client::new();
    let res = client.post("https://overpass-api.de/api/interpreter")
        .header("User-Agent", "DarkRiftMapGenerator/1.0 (contact: test@example.com)")
        .body(query)
        .send()
        .await?;

    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await?;
        return Err(format!("Overpass API error: {} - {}", status, text).into());
    }

    let json: Value = res.json().await?;
    Ok(json)
}
