//! HumanNeeded app engine. Everything the tray app does, without the UI, so it can run headless
//! (`hnd`) in CI and integration tests.
//!
//! - [`http`]: receives agent hook events on 127.0.0.1 (D-047, the main path).
//! - [`ipc`]: receives `hn` messages (named pipe / Unix socket; kept as a fallback).
//! - [`link`]: the only owner of the puck's serial port; `hn-usb/2` client with reconnect.
//! - [`liveness`]: reads Claude's session files and checks processes.
//! - [`engine`]: the loop tying them together (session table → SNAP every 10 s / on change).
//! - [`firmware`]: `latest.json` → SHA-256 → `BOOTSEL` → copy .uf2 to RPI-RP2.
//! - [`integrations`]: detect / install the HumanNeeded integration per agent app.

pub mod engine;
pub mod firmware;
pub mod http;
pub mod integrations;
pub mod ipc;
pub mod link;
pub mod liveness;
pub mod log;
pub mod paths;

pub use engine::{Config, Engine};
pub use link::PuckTarget;
