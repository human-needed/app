//! HumanNeeded core: everything that can be tested without I/O.
//!
//! - [`event`]: the `hn-events/1` wire format (agent -> app).
//! - [`proto`]: host side of the puck protocol `hn-usb/2`.
//! - [`sessions`]: the session table, merge and liveness rules.
//! - [`claude`]: Claude Code's `~/.claude/sessions/<pid>.json` files (liveness source).
//! - [`ids`]: stable 8-hex puck ids for sessions.
//! - [`version`]: version comparison for firmware updates.

pub mod claude;
pub mod endpoint;
pub mod event;
pub mod ids;
pub mod proto;
pub mod sessions;
pub mod version;
