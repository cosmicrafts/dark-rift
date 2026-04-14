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