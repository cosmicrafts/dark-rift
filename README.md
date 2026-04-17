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
