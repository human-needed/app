//! The app's own `hn` (app-events §1, §6b, D-046): a copy in `<data>/bin`, which hooks the app
//! writes call by absolute path. Putting it on PATH is only a convenience for people and skills
//! (D-046): nothing depends on it.

use hn_engine::paths;
use serde_json::{json, Value};
use std::path::PathBuf;

const EXE: &str = if cfg!(windows) { "hn.exe" } else { "hn" };

fn bin_dir() -> PathBuf {
    paths::data_dir().join("bin")
}

/// Same file as `hn_engine::integrations::hn_path()`.
fn dst() -> PathBuf {
    hn_engine::integrations::hn_path()
}

fn bundled() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join(EXE);
    p.is_file().then_some(p)
}

pub fn status() -> Value {
    let on_path = hn_engine::integrations::which("hn", std::env::var_os("PATH").as_deref());
    json!({
        "installed": dst().is_file(),
        "path": dst(),
        "on_path": on_path.is_some() || user_path_has(&bin_dir()),
    })
}

/// Copies `hn` (shipped next to the app) to `<data>/bin` when it differs.
pub fn ensure_copy() -> Result<PathBuf, String> {
    let src = bundled().ok_or("hn isn't next to the app")?;
    let dir = bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dst = dst();
    let same = std::fs::read(&src).ok() == std::fs::read(&dst).ok();
    if !same {
        // A hook may be running the old copy: move it aside first (works on Windows too).
        let _ = std::fs::remove_file(dir.join(format!("{EXE}.old")));
        let _ = std::fs::rename(&dst, dir.join(format!("{EXE}.old")));
        std::fs::copy(&src, &dst).map_err(|e| format!("copy hn: {e}"))?;
    }
    Ok(dst)
}

/// Copy + PATH convenience.
pub fn install() -> Result<String, String> {
    let dst = ensure_copy()?;
    add_to_user_path(&bin_dir())?;
    Ok(format!("{} ready, on PATH", dst.display()))
}

#[cfg(windows)]
fn user_path_has(dir: &std::path::Path) -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let Ok(env) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment") else {
        return false;
    };
    let path: String = env.get_value("Path").unwrap_or_default();
    let want = dir.to_string_lossy().to_lowercase();
    path.split(';')
        .any(|p| p.trim().trim_end_matches('\\').to_lowercase() == want)
}

#[cfg(windows)]
fn add_to_user_path(dir: &std::path::Path) -> Result<(), String> {
    use winreg::{enums::*, RegKey};
    if user_path_has(dir) {
        return Ok(());
    }
    let env = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|e| e.to_string())?;
    // Keep REG_EXPAND_SZ values (e.g. %USERPROFILE%) intact.
    let raw = env.get_raw_value("Path").ok();
    let current: String = env.get_value("Path").unwrap_or_default();
    let new = if current.trim().is_empty() {
        dir.display().to_string()
    } else {
        format!("{};{}", current.trim_end_matches(';'), dir.display())
    };
    let vtype = raw.map(|r| r.vtype).unwrap_or(REG_EXPAND_SZ);
    let bytes: Vec<u8> = new
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|u| u.to_le_bytes())
        .collect();
    env.set_raw_value("Path", &winreg::RegValue { bytes, vtype })
        .map_err(|e| e.to_string())?;
    broadcast_env_change();
    Ok(())
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

#[cfg(unix)]
fn user_path_has(dir: &std::path::Path) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir))
}

/// D-045: macOS `/usr/local/bin` (GUI-started agents only get launchd's PATH), Linux
/// `~/.local/bin`. A symlink to our copy, so app updates refresh it.
#[cfg(unix)]
fn add_to_user_path(dir: &std::path::Path) -> Result<(), String> {
    let target_dir = if cfg!(target_os = "macos") {
        PathBuf::from("/usr/local/bin")
    } else {
        dirs_home().ok_or("no home")?.join(".local/bin")
    };
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("{}: {e}", target_dir.display()))?;
    let link = target_dir.join(EXE);
    if std::fs::read_link(&link).ok().as_deref() == Some(&dir.join(EXE)) {
        return Ok(());
    }
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(dir.join(EXE), &link)
        .map_err(|e| format!("{}: {e} (the installer must create it)", link.display()))
}

#[cfg(unix)]
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
