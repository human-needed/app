// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;

/// Per-user data folder (logs, config). `HN_DATA_DIR` overrides it (tests).
pub fn data_dir() -> PathBuf {
    if let Some(v) = std::env::var_os("HN_DATA_DIR") {
        return PathBuf::from(v);
    }
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    if cfg!(any(windows, target_os = "macos")) {
        base.join("HumanNeeded")
    } else {
        base.join("humanneeded")
    }
}

/// Claude Code's config folder (`CLAUDE_CONFIG_DIR`, else `~/.claude`).
pub fn claude_dir() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(v));
    }
    dirs::home_dir().map(|h| h.join(".claude"))
}
