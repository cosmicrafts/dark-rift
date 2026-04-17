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
To organically tie-break priorities without resorting to Floating Point Math (`f32` destroys Lockstep determinism on mobile devices), we initially returned `Ordering::Equal` to the Rust `BinaryHeap`. Rust hated this. We broke the `Ord` Total Ordering contract, triggering arbitrary memory-based sorting that instantly desynchronized both clients (`desync`).
* **The Fix:** We injected an absolute internal clock into `AttackExecution` via `insert_seq`. Now, on an exact priority tie, the cell that queued *first* wins. This gives Rust the strict Total Ordering it mathematically demands, while perfectly simulating the stable-ish FIFO behavior that Typescript implicitly provided with its array.

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
