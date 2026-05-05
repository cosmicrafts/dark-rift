// ─── Map Fetch Module ─────────────────────────────────────────────────────────
//
// Coordinate system documentation (the ONE source of truth):
//
//   Game coords:  (x, y) where y=0 is NORTH (top), y=height-1 is SOUTH (bottom).
//   Bevy world:   Y increases upward. Camera formula: world_y = (height-1 - game_y) * TILE_SIZE.
//   Texture:      Row 0 is the TOP of the sprite → highest world_y → game y=0 (north).
//                 NO Y-FLIP needed. Game row y writes directly to texture row y.
//   Hover:        world_y → game_y = height - 1 - (world_y / TILE_SIZE).round()
//
// All three systems (texture, camera, hover) are consistent WITHOUT any flip.
// ──────────────────────────────────────────────────────────────────────────────

use bevy::prelude::*;

/// Monotonic epoch counter to distinguish stale fetch results from current ones.
/// Each new fetch increments the epoch; results from old epochs are discarded.
#[derive(Resource)]
pub struct MapFetchChannel {
    pub rx: crossbeam_channel::Receiver<Option<Vec<u8>>>,
    pub epoch: u64,
}

/// Cached map data. Used by both the lobby browser (prefetch) and the loading screen.
#[derive(Resource)]
pub struct MapCache {
    pub map_name: String,
    pub map_data: Option<Vec<u8>>,
    pub state: MapCacheState,
    epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapCacheState {
    /// No fetch in progress, no data.
    Idle,
    /// Async fetch in progress.
    Fetching,
    /// Data is available.
    Ready,
    /// Fetch failed or was discarded.
    Failed,
}

impl Default for MapCache {
    fn default() -> Self {
        Self {
            map_name: String::new(),
            map_data: None,
            state: MapCacheState::Idle,
            epoch: 0,
        }
    }
}

impl MapCache {
    /// Request a map fetch. If the requested map is already cached and ready, does nothing.
    /// If a different map is requested or the current one failed, starts a new fetch.
    /// Returns true if a new fetch was started.
    pub fn request(&mut self, map_name: &str, commands: &mut Commands) -> bool {
        // Already have the right data
        if self.map_name == map_name && self.state == MapCacheState::Ready && self.map_data.is_some() {
            return false;
        }
        // Already fetching the right map
        if self.map_name == map_name && self.state == MapCacheState::Fetching {
            return false;
        }

        // Start new fetch
        self.epoch += 1;
        let epoch = self.epoch;
        self.map_name = map_name.to_string();
        self.state = MapCacheState::Fetching;
        self.map_data = None;

        let (tx, rx) = crossbeam_channel::unbounded();
        commands.insert_resource(MapFetchChannel { rx, epoch });

        let fetch_name = map_name.to_string();

        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("assets/maps/{}/map.bin", fetch_name);
                bevy::log::debug!("🗺️  [MapCache] Fetching map via HTTP: {}", url);
                let res = gloo_net::http::Request::get(&url).send().await;
                if let Ok(resp) = res {
                    if resp.ok() {
                        if let Ok(bytes) = resp.binary().await {
                            bevy::log::debug!("🗺️  [MapCache] Received {} bytes", bytes.len());
                            let _ = tx.send(Some(bytes));
                            return;
                        }
                    }
                }
                bevy::log::warn!("🗺️  [MapCache] HTTP fetch failed for {}", fetch_name);
                let _ = tx.send(None);
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let server_url = crate::NATIVE_NETCODE_SOCKET.to_string();
            std::thread::spawn(move || {
                // Try HTTP from game server first
                let mut ip = server_url.replace("ws://", "").replace("wss://", "");
                if let Some(idx) = ip.find(':') {
                    ip.truncate(idx);
                }
                let url = format!("http://{}/assets/maps/{}/map.bin", ip, fetch_name);
                bevy::log::debug!("🗺️  [MapCache] Fetching map via HTTP: {}", url);
                if let Ok(resp) = ureq::get(&url).call() {
                    let len = resp.header("Content-Length")
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(0);
                    let mut bytes: Vec<u8> = Vec::with_capacity(len);
                    if resp.into_reader().read_to_end(&mut bytes).is_ok() && !bytes.is_empty() {
                        bevy::log::debug!("🗺️  [MapCache] Received {} bytes from server", bytes.len());
                        let _ = tx.send(Some(bytes));
                        return;
                    }
                }
                // HTTP failed — try local filesystem
                bevy::log::debug!("🗺️  [MapCache] HTTP failed, trying local filesystem for '{}'", fetch_name);
                let bin_path = format!("assets/maps/{}/map.bin", fetch_name);
                match std::fs::read(&bin_path) {
                    Ok(bytes) if !bytes.is_empty() => {
                        bevy::log::debug!("🗺️  [MapCache] Loaded {} bytes from local file", bytes.len());
                        let _ = tx.send(Some(bytes));
                    }
                    _ => {
                        bevy::log::warn!("🗺️  [MapCache] No local file found at {}", bin_path);
                        let _ = tx.send(None);
                    }
                }
            });
        }

        true
    }

    /// Take the cached data, leaving the cache empty. Used when starting the game.
    pub fn take_data(&mut self) -> Option<Vec<u8>> {
        if self.state == MapCacheState::Ready {
            self.state = MapCacheState::Idle;
            self.map_data.take()
        } else {
            None
        }
    }

    /// Check if data is ready to use
    pub fn is_ready(&self) -> bool {
        self.state == MapCacheState::Ready && self.map_data.is_some()
    }

    /// Check if currently fetching
    pub fn is_fetching(&self) -> bool {
        self.state == MapCacheState::Fetching
    }
}

/// System: polls the async channel for fetch results. Runs every frame in all states.
pub fn system_poll_map_fetch(
    mut commands: Commands,
    channel: Option<Res<MapFetchChannel>>,
    cache: Option<ResMut<MapCache>>,
) {
    let (Some(ch), Some(mut cache)) = (channel, cache) else { return };

    if let Ok(result) = ch.rx.try_recv() {
        if ch.epoch == cache.epoch {
            // This result matches the current request
            match result {
                Some(bytes) => {
                    cache.map_data = Some(bytes);
                    cache.state = MapCacheState::Ready;
                }
                None => {
                    cache.state = MapCacheState::Failed;
                }
            }
        } else {
            bevy::log::warn!(
                "🗺️  [MapCache] Discarding stale fetch result (epoch {} vs current {})",
                ch.epoch, cache.epoch
            );
        }
        commands.remove_resource::<MapFetchChannel>();
    }
}

/// Load map bytes into a GameState's terrain. Returns true on success.
/// Tries: 1) cached data, 2) local filesystem (native only).
pub fn apply_map_to_game(
    game: &mut dark_rift_shared::game::GameState,
    map_name: &str,
    cached_bytes: Option<Vec<u8>>,
) -> bool {
    let expected_size = (game.map.width * game.map.height) as usize;

    // Try cached bytes first
    if let Some(bytes) = cached_bytes {
        if bytes.len() == expected_size {
            game.map.terrain = bytes
                .into_iter()
                .map(dark_rift_shared::map::MapTile::from_byte)
                .collect();
            bevy::log::debug!("🗺️  Map loaded from network cache ({} tiles)", expected_size);
            return true;
        }
        bevy::log::warn!(
            "🗺️  Cached map size mismatch: expected {}, got {}",
            expected_size,
            bytes.len()
        );
    }

    // Fallback: local filesystem (only works on native, not WASM)
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some((map, _manifest)) =
            dark_rift_shared::map_loader::load_openfront_map(map_name)
        {
            if (map.width * map.height) as usize == expected_size {
                game.map = map;
                bevy::log::debug!("🗺️  Map loaded from local filesystem fallback");
                return true;
            }
            bevy::log::warn!("🗺️  Local map dimensions don't match config");
        }
    }

    bevy::log::error!(
        "🗺️  Failed to load map '{}' from any source! Using blank default.",
        map_name
    );
    false
}
