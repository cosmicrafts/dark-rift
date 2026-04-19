# Dark Rift - Developer Journal

> *"Bringing the organic chaos of OpenFront into the deterministic rigor of Rust"*

Welcome to **Dark Rift**, the ECS-driven (Bevy + Lightyear) child of OpenFront.


## Philosophy: "Organic Feel" vs Strict Math
In OpenFront (Typescript), we achieved an incredible dynamic combat feel: attacks looked alive, expanding like mycelium, waves, and organic tentacles. When porting this to Rust for flawless Lockstep synchronization and WASM/Mobile cross-play, we quickly realized that a 1-to-1 math translation wasn't enough. Rust is mathematically rigid, and its standard library contracts exposed deep underlying flaws that Typescript's V8 engine implicitly hid.

## Core Architectural Learnings

### 1. The "Hyperplanes" Defect (Blocky/Square Expansion)
Initially, we used a standard pseudo-random number generator (LCG). This generator had a fatal algorithmic flaw: on a 2D spatial map, it grouped randomness into perfectly straight lines (Marsaglia Hyperplanes). Our attacks kept expanding like Minecraft blocks.
* **The Fix:** We rebuilt the RNG around the **PCG32** permutation algorithm. It's just as fast as an LCG but mathematically shatters linearity, successfully mimicking the exact organic "white noise" we used to get from ARC4 (`seedrandom`) in Typescript.

### 2. The DFS "Spike" (The Importance of Observation Timing)
We suffered from a "DFS Spike": whenever a massive burst of troops accumulated and was fully deployed at once, the attack would drill deeply into enemy territory (Depth-First-Search), forming a single long tentacle and ignoring the flanks.
* **The Problem:** In Dark Rift, we were flipping the color/ownership of the conquered cell (`set_owner_id`) **before** assessing its neighbors. When the neighbors subsequently analyzed their surroundings to calculate their expansion penalty, they falsely viewed their newly-conquered parent as an ally! This lowered their penalty, skyrocketed their priority, and shot them to the absolute front of the Min-Heap.
* **The Fix:** Adhering perfectly to OpenFront's TS order, we pushed `set_owner_id` to the absolute **end** of the loop, *after* the neighbors are evaluated. Now, the new neighbors are pessimistic, have max penalty, and fall to the back of the queue, forcing the simulation to expand across the full initial boundary first (flawlessly reconstructing an organic Breadth-First-Search wavefront).

### 3. Faking Instability in a Total Order Contract (`insert_seq_counter`)
To organically tie-break priorities, we historically avoided Floating Point Math (`f32`), under the heavy assumption that floating-point variations across CPU architectures would utterly destroy Lockstep determinism. (Note: We later proved this false! Rust's `wasm32-unknown-unknown` pipeline compiles to strict IEEE-754 identical to Linux `glibc`, meaning basic floats were entirely safe; the real phantom bug was `HashMap` entropy, detailed in Point 4). 
Regardless, relying entirely on integer priorities meant we suffered massive amounts of literal "ties". Initially, we returned `Ordering::Equal` to the Rust `BinaryHeap` during ties. Rust hated this. We broke the `Ord` Total Ordering contract, triggering arbitrary memory-based sorting that instantly desynchronized clients.
* **The First Fix:** We injected a monotonic internal clock into `AttackExecution` via `insert_seq`. On an exact priority tie, the cell that queued *first* wins. This partially satisfied Rust's strict Total Ordering demand and simulated the stable-ish FIFO behavior that Typescript implicitly provided with its array.
* **The Realization:** While `insert_seq` fixed the tree balance *within a single attack*, it catastrophically failed when merging multiple attack frontiers globally (because two separate attacks could both hit `insert_seq = 0`). This directly fed into a massive systemic collapse explained below.

### 4. The `HashMap` Entropy Trap (Poka-Yoke Determinism)
When merging attack frontiers, we used a temporary `HashMap` to filter out duplicate tiles. In Rust, `HashMap` intentionally randomizes its iteration order (SipHash) on every machine to prevent DDoS attacks. When we sorted these hashed tiles using `.sort_by()`, any two tiles that mathematically "tied" in priority and `insert_seq` returned `Ordering::Equal`. When `.sort()` sees `Equal`, it respects the original order—unleashing the random `HashMap` chaos and sending the Mac client to conquer the left flank while the Linux client conquered the right.
* **The Poka-Yoke (Mistake-Proofing):** Never terminate an `Ord` or `cmp` implementation with `Ordering::Equal` for game state structs. You must **ALWAYS** define a flawless, absolute fallback tie-breaker. By enforcing `match self.y.cmp(&other.y) { Ordering::Equal => self.x.cmp(&other.x) ... }`, we bind the equality to the immutable spatial coordinates of the map. It is physically impossible for two distinct expansion tiles to occupy the exact same `X, Y`, mathematically locking the array order against the underlying `HashMap` entropy.

### 5. ECS Asynchronous Drift (Resource Loading Poka-Yoke)
To make bot difficulty dynamic, we upgraded the AI system to read a .ron config file inserted via Bevy's `Commands::insert_resource`. Because Bevy flushes commands asynchronously at the end of a frame (`apply_deferred`), the WASM Mac client (running at 120hz ProMotion) ticked the `FixedUpdate` physics loop before the resource was fully committed to RAM. Our AI system saw `None`, hit an early `return;`, and completely skipped a simulation tick, immediately destroying the PRNG sequence and lockstep vs the Linux client.
* **The Poka-Yoke (Mistake-Proofing):** **NEVER** use early-exit `return;` paths in your deterministic `SimulationSchedule` systems based on the presence of an asynchronous ECS Resource. The simulation must ALWAYS execute identically. If a resource is pending, gracefully synthesize a default directly in the loop (e.g., `unwrap_or_default()`) so the loop computes the exact same PRNG consumption and state mutations on all platforms, ignoring async rendering latency.

### 6. Visual Data Leaks Across Phase Transitions (Lobby Switching)
When transitioning out of an `InGame` phase or disconnecting from a server, we despawned 3D models and entities but forgot to clear headless standard Resources (`FreshlyPaintedTiles`). A player played a 1000x1000 local Single Player map, and `(x=133, y=844)` was recorded as "freshly painted". They hit "Go Back To Lobby" and joined a 200x200 Multiplayer server. Because the UI animations `Update` system ran unconditionally out of bounds, it queried `y=844` on a `200x200` memory array, crashing the Rust thread instantly.
* **The Poka-Yoke (Mistake-Proofing):** Always tie localized visual systems entirely to `.run_if(in_state(AppState::InGame))`. When creating a phase `cleanup` system, you must exhaustively call `.clear()` on **every single** transient HashMap, Queue, or Array resource. State transitions don't automatically free `ResMut<T>`.

### 7. The Temporal Paradox (Tick Rates vs Time)
When converting "docile" Bot behaviors from TypeScript OpenFront to Dark Rift, we mapped the "attack roughly every 40 ticks" logic 1-to-1. However, the bots acted like enraged Terminators, playing inexplicably hyper-aggressive. The hidden variable was time: OpenFront runs its bot logic loop strictly at **4 Ticks/Sec**. Dark Rift runs its ECS `FixedUpdate` physics loop natively at **60 Ticks/Sec**. 40 Ticks in OpenFront equals 10 seconds of peace; 40 Ticks in Dark Rift equals 0.6 seconds of absolute terror.
* **The Poka-Yoke (Mistake-Proofing):** When porting values, delays, or arrays from the TypeScript architecture to Rust, you must multiply or divide by `15` to preserve real-world seconds. `600 ticks` in Rust = `40 ticks` in Typescript. Never port raw tick integers blindly.

### 8. The O(N) Asymptote Death (The 4.8 Million Loop Trap)
When transitioning from a 500x500 map to a 2900x1600 map (4.8 million tiles), the simulation grinded to a halt (5 FPS). We discovered two silent assassins doing full linear map scans every frame: the UI was iterating the entire map to find the center of mass for player labels, and the Bots were iterating the entire map every 50ms to find their borders.
* **The Fix:** We implemented `O(1)` dynamic caching directly on the `Player` struct. By tracking `sum_x`, `sum_y`, `tile_count`, and a `BTreeSet` of `border_tiles`, and updating them incrementally strictly when a tile changes ownership in `set_tile_owner`, we eliminated billions of wasted CPU cycles per second.

### 9. The Monolithic Mesh Collapse (GPU Starvation)
Even after fixing the CPU, the game stuttered. Bevy's `TilemapChunk` was attempting to render all 4.8 million tiles as a single colossal mesh (19 million vertices). Any time a single tile changed color, Bevy marked the component as changed, forcing the GPU to re-mesh and re-upload the entire continent of Europe.
* **The Fix:** Spatial Chunking. We split the map into a grid of 64x64 `TilemapChunk` entities (~1,200 tiny chunks). When an attack occurs, we use an O(1) mathematical lookup to find which chunk was hit, and only mutate that specific 4,096-tile sector. The other 1,199 chunks remain perfectly cached on the GPU, dropping rendering overhead by ~1,200x.

### 10. The WebGL Coordinate Paradox & The "Spawn in the Sea" Trap
When porting the map renderer to WASM, players consistently spawned in the ocean instead of their nations. Initially, we blamed WebGL's default coordinate system (WebGL inherently reads texture buffers `bottom-up`, while Vulkan/Metal read them `top-down`). We attempted to manually patch this by injecting a `flip_y: cfg!(target_arch = "wasm32")` into the Sprite component.
* **The Symptom:** Despite the "fix", players still spawned in the sea. To make matters worse, the VPS cloud server began rejecting map loads with "size mismatches" (expected 4.8M tiles, got 2M).
* **The Cloud Trap:** We discovered `deploy_cloud.sh` was only pushing the compiled Rust binary to the cloud, *not* the `assets/maps/` directory. Without the `manifest.json`, the VPS server silently fell back to default `GameConfig` dimensions (Europe: 2904x1672), while the WASM client fetched the correct World map (2000x1000) from Nginx. The dimensions mismatched, the map crashed, and the real coordinate issue was obscured.
* **The Final Realization:** After fixing the deploy script to `rsync` assets to the server's `WorkingDirectory`, we traced the coordinate chain. The realization hit: **Bevy's `wgpu` layer flawlessly abstracts the WebGL/Vulkan differences**. The `Image` buffer layout is already completely consistent across all backends. Our `flip_y` WASM hack was actually *causing* the map to render upside down on the web, displacing the nations to the bottom while the camera panned to the top. By removing all platform-specific hacks, the raw byte array perfectly mapped to the exact same visual coordinates natively and on the web.

### 11. The GPU Texture Re-Upload Trap (OpenFront's Secret: Deferred Dirty-Rect Rendering)
With 400 bots fighting, the WASM client froze at 5 FPS. The culprit was invisible: every time a tile changed color, our code called Bevy's `images.get_mut(&handle)`, which marked the **entire** 8-19 MB map texture as "dirty". Bevy then re-uploaded the full texture to the GPU via `texImage2D` — **twice per frame** (once for tile changes, once for glow animations). On WebGL, this meant pushing 16-38 MB/frame through the JavaScript-to-GPU bridge at 60 FPS (over 1 GB/s).
* **OpenFront's Secret:** After auditing OpenFront's `TerritoryLayer.ts`, we discovered three hidden weapons: (1) CPU-side `ImageData` buffer with `putImageData(dirty_rect)` for partial uploads, (2) a `PriorityQueue` that drains only **10% of changed tiles per frame** (amortized across ~10 frames), and (3) a 100 Hz refresh throttle. OpenFront never uploads the full map per frame — only the visible dirty rectangle.
* **The Fix:** We replicated OpenFront's architecture in Bevy: a `MapTextureCPU` resource holds the pixel buffer outside Bevy's change detection, a `TileRenderQueue` drains 10% per frame, and a single `flush_map_to_gpu` system copies only the dirty rows once per frame. On idle frames (no attacks), **zero bytes** are uploaded to the GPU. During heavy combat, only the bounding box of active warfare is transferred.
* **The Result:** GPU uploads dropped from 19-38 MB/frame to ~100 KB/frame. Idle frames trigger no GPU work at all. The 10% drain also creates the same organic "wave" expansion visual that makes OpenFront feel alive.

---
### 🤖 OpenClaw Integration Test
This repository is currently being orchestrated and optimized via OpenClaw AI. Automated deployments, config management, and stress-testing logic have been validated.

## WASM local build (Fase 1)

One-time toolchain:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli
```

If `wasm-bindgen` fails with a version mismatch, install the CLI version it prints (for example `cargo install wasm-bindgen-cli --version 0.2.118` to match this workspace’s `wasm-bindgen` crate).

From the `dark-rift/` directory:

```bash
./build_wasm_local.sh
python3 -m http.server 8000 -d dist/
```

Open `http://localhost:8000/` to run the client in the browser.

**Multiplayer from the browser:** the WASM client uses **`wss://darkrift.ai/ws/`** for the WebSocket transport (TLS in the browser). Netcode still embeds a numeric **`IP:25565`** in the connect token; on `wasm32-unknown-unknown` Rust cannot resolve hostnames, so before production deploy set `WASM_NETCODE_SOCKET` in `crates/client/src/main.rs` to your VPS public IPv4 (same port Nginx forwards to the game server).

## Deployment & Orchestration

The `run_cluster.py` script has been supercharged to act as an all-in-one orchestrator for local development, cloud deployment, web (WASM) building, and stress testing.

### Usage Modes

**1. Default (Local Dev)**
```bash
./run_cluster.py
```
*Builds and runs 1 local server and 2 local clients connecting to it (127.0.0.1:25565).*

**2. Cloud Deploy (Server)**
```bash
./run_cluster.py --deploy <VPS_IP>
```
*Compiles the server locally in release mode, rsyncs the binary and assets to the target VPS, configures a `darkrift-server` systemd service, and restarts it automatically.*

**3. Web Deploy (WASM)**
```bash
./run_cluster.py --deploy <VPS_IP> --web
```
*After deploying the server, compiles the client to WebAssembly, binds it, and rsyncs the bundled web version directly to the VPS Nginx directory (`/var/www/worldofunreal.com/darkrift`).*

**4. Stress Test**
```bash
./run_cluster.py --stress 120 --server <VPS_IP:25565>
```
*Spawns 120 local instances of the client pointing to the given cloud server IP to test its limits.*
