# Dark Rift // Zero-Overhead Singularity Engine

A high-performance incremental simulation game powered by Vulkan/Metal hardware acceleration, real-time general relativity black hole lensing shaders, and a zero-allocation game state loop.

---

## 🌌 Overview

In **Dark Rift**, you nurture a nascent micro-singularity from $0.0\text{ kg}$ into a supermassive monster capable of consuming planets, stars, and eventually collapsing the entire universe in **The Big Crunch**.

### Core Pillars
- **Zero-Allocation Hot Path:** Simulation tick and GPU buffer updates run with zero heap allocations per frame.
- **Relativistic Raymarched Shader:** General-relativistic gravitational lensing, Kerr metric spin parameter ($a^*$), photon sphere, and accretion disk rendering in real-time WGSL.
- **Progressive Accretion Loop:**
  1. **Phase 1 (0 to 10 kg):** Manual gravitational pulses on the black hole.
  2. **Phase 2 (10 to 150 kg):** Automated quantum gas siphons & matter streams (+kg/s).
  3. **Phase 3 (150 kg to 1,000,000 t):** Relativistic technologies (Frame Dragging, Relativistic Jets, Penrose Extraction).
  4. **Phase 4 (The Big Crunch):** Primordial collapse into Dark Matter Shards for permanent cosmic perks.

---

## 🚀 Getting Started

### Prerequisites
- **Rust Toolchain:** Stable Rust (2021 edition) + Cargo.
- **Vulkan Drivers:** A Vulkan 1.2+ capable GPU (NVIDIA, AMD, Intel, or Apple Silicon via Metal).

### Running Native Desktop Client
```bash
./dr
```
*Or via cargo directly:*
```bash
cargo run -p dr-client
```

### Running Workspace Unit Tests
```bash
cargo test --workspace
```

---

## 🛠️ Architecture

```
dark-rift/
├── crates/
│   ├── dr-core/      # Pure simulation engine: Black hole physics, tiers, tech tree, prestige, Big Number arithmetic
│   ├── dr-render/    # Blade Graphics (Vulkan/Metal) renderer & WGSL shaders (black_hole.wgsl, particles.wgsl)
│   └── dr-client/    # Native Winit application runner, Blade integration & Egui HUD
├── AGENTS.md         # Strict rules and prohibited anti-patterns for AI assistants
└── Cargo.toml        # Workspace manifest
```

---

## 📜 License
MIT / Apache 2.0.
