# Dark Rift - Developer Journal

> *"Bringing the organic chaos of OpenFront into the deterministic rigor of Rust"*

Welcome to **Dark Rift**, the ECS-driven (Bevy + Lightyear) child of OpenFront.

## Current Architecture Snapshot

- Shared deterministic simulation is split across module trees:
  - `crates/shared/src/execution/mod.rs` (`combat.rs`, `income.rs`)
  - `crates/shared/src/intent/mod.rs` (`combat.rs`, `fleet.rs`, `buildings.rs`, `bot.rs`)
  - `crates/shared/src/building/mod.rs` (`core.rs`, `placement.rs`, `cost.rs`, `construction.rs`, `upgrade.rs`, `hud.rs`)
- Client lockstep schedule is chained in `crates/client/src/main.rs` under `SimulationSchedule`:
  `pop_turn` (applies queued `Turn`, advances `PendingTicks`) → optional `system_headless_bot_engine` when `--bot`.
- Server remains turn-relay only; lobby lifecycle and bot roster injection are handled in
  `crates/server/src/lobby/mod.rs` when a lobby transitions to `Active`.

## OpenFront Parity Matrix (Current)

### Implemented
- Attack execution (frontier expansion, merge, retreat refund path).
- Deterministic local bot think (`system_bot_think`) and headless bot sender.
- Turn relay architecture (`StampedIntent`/`Turn`) with replay-friendly ordering.
- Player troop/gold income and building aggregate effects.
- Structure build + upgrade intents with placement and cost validation.
- Construction tick system with `StructureReady` events.
- Fleet launch/recall/landing flow plus merge into attack execution.

### Partial
- Spawn lifecycle (server-selected spawn points; no full TS spawn phase execution).
- Retreat behavior (immediate retreat flag; no delayed `RetreatExecution` window yet).
- Defense posts (frontier priority pressure implemented; active turret projectile loop not yet).
- City/factory/port economics are present, but no full trade/rail execution family.
- Game-over experience is currently client-evaluated by alive-count, not full shared win-check logic.

### Missing
- WinCheckExecution parity using tile percentage threshold as authoritative shared sim logic.
- Pause/disconnect execution family (`PauseExecution`, `MarkDisconnectedExecution` equivalents).
- Nuclear and projectile stack (`Nuke`, `MIRV`, `SAMLauncher/SAMMissile`, `ShellExecution`).
- Naval combat stack (`Warship`, `TradeShip`, richer transport edge cases).
- Rail stack (`Train`, `TrainStation`, rail cluster recompute).
- Diplomacy/social/economy transfer stack (alliances, emoji/chat, donations, embargo).
- Advanced Nation AI orchestration.


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

### 12. The WebGL2 Mobile Texture Death Trap (Silent Shader Crashes)
When we finally ported the new GPU map renderer (`border.wgsl`) to WASM, it worked perfectly on desktop Chrome/Firefox, but **silently and fatally crashed on iOS Safari and Android Chrome**. The wgpu pipeline builder rejected the shader bindings entirely.
* **The Culprit:** Desktop OpenGL 4.x natively supports massive 32-bit texture formats (`R32Uint`, `Rgba32Float`), so our initial GPU architecture used them to store player IDs and colors. However, mobile WebGL2 is bound by **OpenGL ES 3.0** constraints. Mobile GPUs physically lack hardware support for filtering 32-bit floats without obscure extensions (`OES_texture_float_linear`), and many Adreno/Mali chips outright reject `R32Uint` framebuffers or storage textures.
* **The Poka-Yoke (Mistake-Proofing):** When targeting WASM/Mobile with `wgpu`, never use 32-bit texture formats unless mathematically unavoidable. We downgraded the GPU data pipeline to strictly mobile-safe formats: `R32Uint` -> `R16Uint` (since player IDs only go up to 4095, fitting in 12 bits), and `Rgba32Float` -> `Rgba8Unorm` (since colors are 8-bit sRGB anyway). The shader `textureLoad` syntax `texture_2d<u32>` handled the WGSL-side type promotion automatically. The crash instantly vanished across all mobile devices with zero loss in visual precision.

### 13. The B0001 Query Conflict Trap (Disjoint Queries)
When querying the same components mutably (like `&mut Node`) across multiple `Query` parameters in the same system, Bevy will statically panic at startup with `error[B0001]` if it detects a potential intersection.
* **The Poka-Yoke (Mistake-Proofing):** You MUST create perfectly disjoint sets using exhaustive `Without<T>` filters. If you have three UI queries `Query<&mut Node, With<A>>`, `Query<&mut Node, With<B>>`, and `Query<&mut Node, With<C>>`, the first query MUST explicitly define `(Without<B>, Without<C>)`, the second `(Without<A>, Without<C>)`, and so on. Do not assume Bevy knows `A`, `B`, and `C` are mutually exclusive; you must mathematically prove it to the ECS type system.

### 14. The Unsorted Query Iteration Trap (Lockstep Divergence)
When iterating over Bevy `Query` results in simulation code, the default iteration order is **not deterministic** across platforms and entity generations. This caused lockstep desyncs where clients would process buildings/fleets in different orders, consuming the RNG state differently and diverging immediately.
* **The Symptom:** Game worked perfectly in single-player, but multiplayer desynced within seconds when multiple buildings or fleets existed. Clients stayed in sync during idle moments but diverged during fleet launches or building construction.
* **The Root Cause:** `q_building.iter().map(|(_, b)| *b).collect()` collects buildings in memory-layout order, which varies by platform architecture (x86_64 vs ARM), Bevy's internal entity allocation, and entity spawn order.
* **The Fix:** ALL Query iterations in simulation code MUST be sorted by a stable, monotonic ID before any state mutation or RNG consumption:
```rust
// BEFORE (WRONG - diverges across clients):
let buildings: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();

// AFTER (CORRECT - deterministic):
let mut buildings: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();
buildings.sort_by_key(|b| b.id);
```
* **Files Fixed:** `crates/shared/src/intent/mod.rs` (line 66-67) - Added sorting before passing buildings to `apply_launch_fleet_intent`.
* **Verification:** All 38 existing tests pass, including `bot_think_is_stable_across_player_insertion_order` and `bot_think_is_repeatable_same_seed_and_order`.

### 15. The Window Focus Input Leak (winit `WindowFocused` Events)
When a player alt-tabbed, minimized the browser, or clicked outside the game window, input events (keyboard/mouse) were NOT being cleared. This caused phantom inputs to accumulate and fire when the player returned focus, triggering unintended fleet launches or building placements that desynced all clients.
* **The Symptom:** Player alt-tabs to check Discord, returns to game, and suddenly 5 fleets launch at random tiles. All clients desync because only ONE client sent the intents.
* **The Root Cause:** Bevy's `ButtonInput<KeyCode>` and `ButtonInput<MouseButton>` resources persist their state across window focus changes. Without explicit clearing, a held key (like 'B' for force-boat) would trigger again when focus returned.
* **The Fix:** Added a `clear_inputs_on_focus_loss` system that listens to `WindowFocused` events and clears both keyboard and mouse input buffers when `!ev.focused`:
```rust
fn clear_inputs_on_focus_loss(
    mut events: MessageReader<bevy::window::WindowFocused>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut kb: ResMut<ButtonInput<KeyCode>>,
) {
    for ev in events.read() {
        if !ev.focused {
            mouse.clear();
            kb.clear();
        }
    }
}
```
* **Commit:** `fdb15da` (April 13, 2026)
* **Files Modified:** `crates/client/src/main.rs`, `crates/client/src/gameplay/input.rs`

### 16. The Stateless Entropy Trap (Tick-Based PRNG Hash Collision)
When porting the bots, we initially seeded a *new* PRNG every tick by hashing `game.seed + game.tick + bot_id`. This seemed deterministic, but it suffered from fatal network desync: if a local client executed a bot intent one tick earlier or later due to network latency interpolation, the `game.tick` hash changed completely, yielding a totally different random outcome and irreversibly splitting the timeline.
* **The Poka-Yoke (Mistake-Proofing):** Never use ephemeral "tick hashes" to drive continuous random logic. Entropy must be **stateful and persistent**. We migrated the entire engine to `bevy_rand` with the `WyRand` algorithm. Each Bot (`Player`) and `AttackExecution` now owns its own strictly serializable `WyRand` property. The entropy stream flows continuously and is natively synchronized over the network by Bevy, making bot decisions completely immune to tick-jitter.

### 17. The Git Master Dependency Trap (WASM Compilation Death)
During the RNG migration, we cloned a repository that referenced Bevy's `main` branch directly (`git = "https://github.com/bevyengine/bevy"`). While it worked locally, mixing a git dependency with our `bevy = "0.18"` from crates.io caused `cargo` to compile two completely different versions of the engine (0.19-dev and 0.18). This exploded the WebAssembly build pipeline because sub-dependencies like `getrandom` misaligned their feature flags (`wasm_js`).
* **The Poka-Yoke (Mistake-Proofing):** Never use `git = ...` dependencies for core architectural components (like ECS or math engines) unless you are ready to fork the entire ecosystem. Always use strict `crates.io` versioning (e.g., `version = "0.14"`) to guarantee feature-flag alignment across complex compile targets like WASM.

---
### 🤖 OpenClaw Integration Test
This repository is currently being orchestrated and optimized via OpenClaw AI. Automated deployments, config management, and stress-testing logic have been validated.

## 🧭 Determinism Contract (read this before touching simulation code)

Dark Rift runs a **lockstep** simulation: the server is a pure turn relay and
every client (native and WASM) must produce a byte-identical `GameState` each
tick from the same seed + same turns. Any divergence → instant desync between
PC and Mac/iPhone/Android. These are the *hard rules*; break them and things
will appear to "work on my machine" but silently diverge on ARM/WASM.

### 1. Zero `HashMap` / `HashSet` iteration in simulation code
`std::collections::HashMap` randomizes iteration order per process (SipHash
DoS mitigation). Forbidden inside `SimulationSchedule` systems,
`apply_stamped_intent`, `system_attack_execution`, `system_bot_think`, or
anything reachable from them.
- OK: use `HashMap` as a **dedup scratch buffer** only if you immediately
  collect into a `Vec` and `sort_by(|a, b| a.cmp(b))` with a *total* order
  (see `merge_frontiers` in `crates/shared/src/intent/combat.rs`).
- Prefer: `Vec`, `BinaryHeap<PrioritizedTile>`, linear scans over
  `game.map.state` / `game.players`.

### 2. Every `Ord` must end with an absolute spatial tie-breaker
Never terminate an `Ord::cmp` with `Ordering::Equal`. The tie-break chain for
any tile struct must end with `(y, x)` — map coordinates are immutable and
unique per tile. See `PrioritizedTile` in `crates/shared/src/execution/mod.rs`:
`priority → insert_seq → y → x`.

### 3. Stateful Caches MUST use absolute spatial ordering
If you maintain a stateful per-player cache (like `Player.border_tiles`), you **must** guarantee its iteration order is perfectly deterministic across all platforms. 
- You may use a `Vec` for incremental caching *only* if it is maintained in strictly sorted order (e.g. by sorting linear indices: `(y * width) + x`).
- Standard `Vec::push` + `swap_remove` inside `set_tile_owner` is **banned**. It creates insertion-order artifacts that cascade into different bot behaviors based on network arrival times.
- If in doubt, derive lazily. If optimizing for speed, use `binary_search` and `insert`/`remove` to guarantee perfect absolute spatial sorting.

### 4. No early-return in `SimulationSchedule` systems based on async resources
Bevy flushes `Commands::insert_resource` at `apply_deferred`. WASM on
ProMotion Safari ticks `FixedUpdate` before the flush. If your system does
`let Some(cfg) = res else { return; }`, one platform will skip a tick and the
PRNG stream desyncs forever. Use `unwrap_or_default()` and execute the same
mutations every tick on every platform.

### 5. Bots run on every client, never broadcast
`system_bot_think` runs inside `SimulationSchedule` locally on every client;
the server only relays **human** intents (`handle_intent_messages`). Bot code
gets the determinism rules *doubled*: no `HashMap`, no Vec-order reuse across
ticks, seed the RNG deterministically from `(game.seed, game.tick, bot_id)`.

#### Current simulation determinism invariants (Bots & Combat)
The current implementation is strictly deterministic across Native and WASM architectures due to these architectural invariants:

1. **Immutable Ordered Snapshots**: Both `system_bot_think` and `system_attack_execution` snapshot simulation inputs (`q_building` -> `Vec<Building>`) and rigidly sort them by `Building.id` before any calculations or state mutations occur (e.g., `defense_post_priority_bonus`). `q_building.iter()` yields an unstable memory-layout order across clients and entity generations, making the sorting step a critical Poka-Yoke.
2. **Deterministic Bot Scheduling**: The bot schedule is built from `game.players` and then sorted by `bot_id`. Insertion order in `GameState.players` must not affect behavior.
3. **Isolated Bot Entropy**: Each bot carries a persistent `WyRand` stream (`Player.bot_rng`) and each `AttackExecution` has its own `WyRand`. There is no shared global bot RNG stream; one bot taking a different branch cannot shift random draws for later bots.
4. **Stable Target Sets**: Enemy target candidates are `sort_unstable()` + `dedup()` before selection, so duplicate border contacts and scan order do not affect the target set.
5. **Two-Phase Application**: Structure and attack decisions are collected into `BotDecision`, sorted by `(bot_id, BotDecisionKind)`, and only then applied through `apply_stamped_intent`.
6. **Integer Escalation**: OpenFront-style exponential structure costs use integer doubling in `scaled_pow2_cost`, strictly avoiding `powf`/`powi` or other platform-sensitive math.
7. **Fixed-Point Combat Math**: Troops/gold and combat rates now use `i64` fixed-point-style units (with milli-scaling for ratios where needed), removing prior floating-point drift concerns from the shared simulation hot path.
8. **HashMap-Free Frontier Merge**: `merge_frontiers` must **never** use `HashMap` for dedup. When two `BinaryHeap` frontiers are merged and both contain a tile at the same `(x,y)` with identical `priority` values (rare but happens when RNG draws collide), `HashMap` insertion order is randomized by SipHash, so different clients would keep different `insert_seq` values — silently diverging the expansion wavefront. The fix uses `Vec` sort by `(x, y, priority, insert_seq)` + `dedup_by_key((x, y))`, which is fully deterministic. This was the root cause of the "starts light then completely diverges during PvP" desync pattern.
9. **Negative Troop Clamping in PvP**: Defender `troops` can go negative when multiple attacks drain simultaneously before `system_player_income` restores them. All PvP reads of defender troops must clamp with `.max(0.0)` **before** computing `ratio` or `def_loss`. Without the clamp, negative troops produce negative `def_loss` — meaning the defender **gains** troops when tiles are captured — compounding each tick and diverging exponentially. The specific lines are: `defender_troops = p.troops.max(0.0)` (expansion speed) and `def_loss = target_player.troops.max(0.0) / tile_count` (attrition).

If simulation determinism breaks again, restore those invariants before tuning strategy or logic. The regression tests that protect this are:

```bash
cargo test -p dark-rift-shared bot_think_is_stable_across_player_insertion_order
cargo test -p dark-rift-shared bot_think_is_repeatable_same_seed_and_order
```

### 6. No float transcendentals in shared simulation
The current shared sim is intentionally integer-driven (`i64` + scaled ratios).
Keep `powf`/`exp`/`ln` out of deterministic gameplay state transitions unless
there is strict proof of cross-target bit-identical behavior and regression
coverage.

### 7. No hidden allocations in simulation hot paths
Per-tile allocations (e.g. `Vec::with_capacity(5)` in `set_tile_owner`,
`neighbors() -> Vec`) are banned on simulation hot paths. Use
`GameMap::for_each_neighbor` (alloc-free closure). Allocator pressure is the
#1 reason WASM/ARM choke — see the perf section in the developer journal
above.

### 8. Stable-sort entities by monotonic id before mutating them
`query.iter()` order is NOT stable across `Entity` generations. Always
collect `(Entity, id)` and `sort_by_key(|&(_, id)| id)` before mutating — see
`system_attack_execution` and `apply_attack_intent`.

**Critical:** This applies to ALL Query iterations in simulation code, not just
attack execution. Building queries, fleet queries, and any other dynamic entity
iteration MUST be sorted by ID before processing:
```rust
// Simulation code - MUST sort:
let mut buildings: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();
buildings.sort_by_key(|b| b.id);

// Bot code - MUST sort:
let mut buildings_snapshot: Vec<Building> = q_building.iter().map(|(_, b)| *b).collect();
buildings_snapshot.sort_unstable_by_key(|b| b.id);
```

### 9. No `std::time` / `Instant` / `SystemTime` in simulation
Wall-clock drifts per platform. All time inside the sim comes from
`game.tick` (u64). Real `Instant` is fine only for non-sim diagnostics like
`NetTiming`.

### 10. No parallelism in `SimulationSchedule`
Keep systems `.chain()`ed. Do not introduce `par_iter`, `rayon`, task pools,
or unordered Bevy system sets inside the sim. Parallel scheduling produces
non-deterministic read/write interleavings.

### Desync debugging checklist
Before blaming architecture, verify:
1. Same `game.seed` on both clients (`ServerStartMessage`).
2. Same `game.tick` when diverging (hash `map.state` + `players.tile_count`
   at tick N and compare).
3. Replay the turn log from `latest_replay.json` headlessly — divergence
   must be reproducible if the bug is in sim code.
4. `rg 'powf|HashMap::new|HashSet::new|Instant::now|thread_rng|rand::random' crates/shared`.

### Deterministic Integration Checklist (for new subsystems)
Before adding any new execution family, validate all items:

1. New player actions enter through `GameplayIntent` / `StampedIntent` (`crates/shared/src/protocol.rs`).
2. Authoritative state mutation runs only in chained simulation systems (`SimulationSchedule` on client).
3. Query iteration order is explicitly stabilized (collect and sort by monotonic domain IDs).
4. No wall-clock timing (`Instant`/`SystemTime`) in shared simulation logic.
5. No unordered `HashMap`/`HashSet` iteration in simulation mutation paths.
6. No parallelism (`par_iter`, unordered system sets) in shared simulation update order.
7. Any new long-lived execution entity has a deterministic ID source (e.g., `GameState` counters).
8. Tests include repeatability by seed and insertion-order stability.

### Economy: gold vs troops (OpenFront parity)
- **Troops**: territory expansion, ground attacks, and fleets (`Player.troops`, `system_player_income` troop branch).
- **Gold**: structure build/upgrade costs and passive gold income (`Player.gold`, `structure_build_cost_gold`, `GOLD_BASE_INCOME` + per ready City level in `config.rs`). Authoritative deduction happens in `apply_build_structure_intent` / `apply_upgrade_structure_intent` (`intent/buildings.rs`).

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

**Multiplayer from the browser:** the WASM client uses **`wss://darkrift.ai/ws/`** for the WebSocket transport (TLS in the browser). Netcode still embeds a numeric **`IP:25565`** in the connect token; on `wasm32-unknown-unknown` Rust cannot resolve hostnames, so before production deploy set `WASM_NETCODE_SOCKET` in `crates/client/src/network/network.rs` to your VPS public IPv4 (same port Nginx forwards to the game server).

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

## Cleanup Items (Tracked)

- Rail/train client rendering (`bak-client` mesh + sprites) stays deferred until `DarkRiftEngine` / shared expose the same rail snapshot the old `GameState`-centric client used.
- `warp_fleet_tests` (library integration tests) was removed: it targeted pre-`DarkRiftEngine` Bevy systems such as `system_warp_fleet_execution`. Re-add fleet/routing coverage as unit tests around `resolve_fleet_route` / `DarkRiftEngine` when a small harness exists.
- `crates/shared/src/test_bevy.rs` appears to be scratch code and should be removed in a cleanup pass.
- `scripts/deploy_cloud.sh` still rewrites `crates/client/src/main.rs` to patch IPs, while runtime networking constants now live in `crates/client/src/network/network.rs`; this should be migrated to avoid stale deploy behavior.
- `TURN_INTERVAL_MS` in `crates/server/src/core/mod.rs` is currently `10.0` while the nearby comment describes `100ms = 10 turns/sec`; align comment and value semantics.
