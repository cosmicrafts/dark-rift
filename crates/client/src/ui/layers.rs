//! Global UI stacking — **one `GlobalZIndex` per full-screen plane**.
//!
//! Use these values only on **root** UI nodes for each plane (siblings under the same UI target).
//! Children can use [`ZIndex`] for ordering *inside* a plane (e.g. modal card above its scrim).
//!
//! # Plane order (back → front)
//!
//! | Constant            | Value | What lives here |
//! |---------------------|------:|-------------------|
//! | `HUD_SHELL`         |   800 | In-game HUD: `HudShellRoot` — top bar, bottom sheet, action dock (HOME/EXIT, combat HUD). |
//! | `LOBBY_BROWSER`     |   900 | Pre-game lobby (`LobbyBrowserRoot`): queue / browser — **above** the shell so gameplay chrome is not visible “through” it. |
//! | `GAME_OVER`         |   950 | End-of-match overlay (`GameOverPanel`). |
//!
//! Reserve gaps of ~50 when inserting new planes (e.g. settings at 850, toast at 980).

/// Root HUD shell — gameplay chrome (`HudShellRoot` in `ui_shell.rs`).
pub const HUD_SHELL: i32 = 800;

/// Root lobby overlay (`LobbyBrowserRoot` in `ui_lobby_browser.rs`).
pub const LOBBY_BROWSER: i32 = 900;

/// Game-over full-screen overlay (`GameOverPanel` in `main.rs`).
pub const GAME_OVER: i32 = 950;
