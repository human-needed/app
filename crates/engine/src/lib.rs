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
//! - [`engine`]: the loop tying them together (session table → SNAP every 10 s / on change).
//! - [`firmware`]: `latest.json` → SHA-256 → `BOOTSEL` → copy .uf2 to RPI-RP2.
//! - [`agents`]: read-only per-agent status (D-054: the user installs the plugins).

pub mod agents;
pub mod cloud;
pub mod engine;
pub mod firmware;
pub mod http;
pub mod ipc;
pub mod link;
pub mod liveness;
pub mod log;
pub mod paths;
pub mod procs;
pub mod proxy;

pub use engine::{Config, Engine};
pub use link::PuckTarget;
