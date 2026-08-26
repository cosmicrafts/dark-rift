# Dark Rift // Agent Guidelines & Strict Rules

## Core Principles

1. **Zero Overhead & Zero VRAM Leaks:**
   - Every GPU resource created with `blade_graphics` must be explicitly deallocated in `cleanup()` and `Drop`.
   - Never let a `CommandEncoder`, `Surface`, or buffer drop without explicit destruction.
   - Hot execution paths must be zero-allocation (preallocated static arrays, flat memory layout).

2. **Onboarding Tempo is Sacred:**
   - The onboarding progression ($0 \to 10\text{ kg}$) must be clean, linear, and gradual ($1\text{ kg per click}$).
   - **PROHIBITED:** Injecting bulk quest mass rewards (e.g. $+5\text{ kg}$, $+10\text{ kg}$) or multipliers during the tutorial that skip steps.
   - Let the player internalize each step before introducing the next concept.

3. **No Unlocked Mechanics at Level 0:**
   - **PROHIBITED:** Triggering active abilities, temporary multipliers, or screen banners for technologies/mechanics whose upgrade level is 0.
   - Always gate mechanic triggers with `if upgrade_level > 0`.

4. **Coordinate Systems & Directional Math:**
   - When drawing vector arrows in `egui`, remember `arrow(origin, vector)` points towards `origin + vector`.
   - If pointing to an element on the **right** (such as the sidebar), `vector.x` MUST be **positive** (`vec2(+55.0, 0.0)`).
   - If pointing to the left, `vector.x` is negative.
   - Always verify coordinate transformations between screen space and normalized shader UVs.

5. **Visual Hierarchy & Zero Clutter:**
   - The central entity (the black hole) is the primary focus.
   - Never render floating text cards over dozens of passive background particles.
   - One clear action indicator per tutorial step.

6. **Validation Gate:**
   - Verify all workspace tests with `cargo test --workspace`.
   - Verify zero compiler warnings with `cargo check --workspace`.
   - Launch and verify live on `DISPLAY=:0.0` with screenshot audits.
