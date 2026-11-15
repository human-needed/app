//! app-events 2.0 retired `hn` (agents post to the local HTTP hub instead). 1.0 builds copied it
//! to `<data>/bin` and put that on PATH (Windows: user PATH; macOS `/usr/local/bin/hn`, Linux
//! `~/.local/bin/hn`, symlinks to our copy). This removes exactly those, nothing else.

use hn_engine::paths;
use std::path::PathBuf;

const EXE: &str = if cfg!(windows) { "hn.exe" } else { "hn" };

fn bin_dir() -> PathBuf {
    paths::data_dir().join("bin")
}

/// Undoes the 1.0 `hn` install. Returns what it did, if anything.
pub fn retire() -> Option<String> {
    let dir = bin_dir();
    let mut done = Vec::new();
    match remove_from_user_path(&dir) {
        Ok(true) => done.push("removed from PATH".to_string()),
        Ok(false) => {}
        Err(e) => done.push(format!("PATH: {e}")),
    }
    if dir.exists() {
        // A hook may still be running the old copy; whatever can't go now goes next start.
        for f in [EXE.to_string(), format!("{EXE}.old")] {
            let _ = std::fs::remove_file(dir.join(f));
        }
        if std::fs::remove_dir(&dir).is_ok() {
            done.push(format!("deleted {}", dir.display()));
        }
    }
    (!done.is_empty()).then(|| done.join("; "))
}

#[cfg(windows)]
fn remove_from_user_path(dir: &std::path::Path) -> Result<bool, String> {
    use winreg::{enums::*, RegKey};
    let env = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|e| e.to_string())?;
    let Ok(raw) = env.get_raw_value("Path") else {
        return Ok(false);
    };
    let current: String = env.get_value("Path").unwrap_or_default();
    let want = dir.to_string_lossy().to_lowercase();
    let parts: Vec<&str> = current.split(';').collect();
    let kept: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|p| p.trim().trim_end_matches('\\').to_lowercase() != want)
        .collect();
    if kept.len() == parts.len() {
        return Ok(false);
    }
    // Keep the value type (REG_EXPAND_SZ keeps %USERPROFILE% etc. working).
    let bytes: Vec<u8> = kept
        .join(";")
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|u| u.to_le_bytes())
        .collect();
    env.set_raw_value(
        "Path",
        &winreg::RegValue {
            bytes,
            vtype: raw.vtype,
        },
    )
    .map_err(|e| e.to_string())?;
    broadcast_env_change();
    Ok(true)
}

#[cfg(windows)]
fn broadcast_env_change() {
    // Tell Explorer so newly started programs see the new PATH.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let env: Vec<u16> = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            env.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            2000,
            &mut result,
        );
    }
}

/// Removes the 1.0 symlink, only if it still points at our copy.
#[cfg(unix)]
fn remove_from_user_path(dir: &std::path::Path) -> Result<bool, String> {
    let link_dir = if cfg!(target_os = "macos") {
        PathBuf::from("/usr/local/bin")
    } else {
        match std::env::var_os("HOME") {
            Some(h) => PathBuf::from(h).join(".local/bin"),
            None => return Ok(false),
        }
    };
    let link = link_dir.join(EXE);
    if std::fs::read_link(&link).ok().as_deref() != Some(&dir.join(EXE)) {
        return Ok(false);
    }
    std::fs::remove_file(&link).map_err(|e| format!("{}: {e}", link.display()))?;
    Ok(true)
}
