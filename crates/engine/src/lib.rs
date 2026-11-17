// SPDX-License-Identifier: GPL-3.0-or-later
//! HumanNeeded app engine. Everything the tray app does, without the UI, so it can run headless
//! (`hnd`) in CI and integration tests.
//!
//! - [`cloud`]: cloud link to the relay, so cloud chats light the puck (relay-api §3).
//! - [`http`]: receives agent hook events on 127.0.0.1 (D-047, the main path).
//! - [`ipc`]: named pipe / Unix socket for the `hn` dev tool (app-events 1.0, retired for agents).
//! - [`link`]: the only owner of the puck's serial port; `hn-usb/2` client with reconnect.
//! - [`liveness`]: reads Claude's session files and checks processes.
//! - [`procs`]: finds the agent process behind a hook's connection (Codex/Cursor liveness).
//! - [`proxy`]: D-049 warning when Claude would send its hooks to a proxy.
//! - [`quiet`]: quiet hours (D-055): sounds muted and the light dimmed in a daily window.
//! - [`engine`]: the loop tying them together (session table → SNAP every 10 s / on change).
//! - [`firmware`]: `latest.json` → SHA-256 → `BOOTSEL` → copy .uf2 to RPI-RP2.
//! - [`agents`]: read-only per-agent status (D-054: the user installs the plugins).
//! - [`alerts`]: the app's alert settings per state, for this computer and the puck (D-059).
//! - [`here`]: "this computer" (D-059): screen edges and sound, driven like a puck.
//! - [`tones`]: the puck's sounds rendered for the computer's speakers.

pub mod agents;
pub mod alerts;
pub mod cloud;
pub mod engine;
pub mod firmware;
pub mod here;
pub mod http;
pub mod ipc;
pub mod link;
pub mod liveness;
pub mod log;
pub mod paths;
pub mod procs;
pub mod proxy;
pub mod quiet;
pub mod tones;

pub use engine::{Config, Engine};
pub use link::PuckTarget;
