//! Puts the `hn` command on the user's PATH (app-events §1): copies `hn` from next to the app
//! into `<data>/bin` and adds that folder to the user's PATH. The installer will do this later;
//! the app keeps it right (e.g. after an app update ships a newer `hn`).

use hn_engine::paths;
use serde_json::{json, Value};
use std::path::PathBuf;

const EXE: &str = if cfg!(windows) { "hn.exe" } else { "hn" };

fn bin_dir() -> PathBuf {
    paths::data_dir().join("bin")
}

fn bundled() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join(EXE);
    p.is_file().then_some(p)
}

pub fn status() -> Value {
    let on_path = hn_engine::integrations::which("hn", std::env::var_os("PATH").as_deref());
    json!({
        "installed": bin_dir().join(EXE).is_file(),
        "dir": bin_dir(),
        "on_path": on_path.is_some() || user_path_has(&bin_dir()),
    })
}

pub fn install() -> Result<String, String> {
    let src = bundled().ok_or("hn isn't next to the app")?;
    let dir = bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dst = dir.join(EXE);
    let same = std::fs::read(&src).ok() == std::fs::read(&dst).ok();
    if !same {
        // A hook may be running the old copy: move it aside first (works on Windows too).
        let _ = std::fs::remove_file(dir.join(format!("{EXE}.old")));
        let _ = std::fs::rename(&dst, dir.join(format!("{EXE}.old")));
        std::fs::copy(&src, &dst).map_err(|e| format!("copy hn: {e}"))?;
    }
    add_to_user_path(&dir)?;
    Ok(format!("{} ready", dst.display()))
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

#[cfg(unix)]
fn add_to_user_path(dir: &std::path::Path) -> Result<(), String> {
    // macOS/Linux: link into ~/.local/bin (on PATH for most shells). OPEN: installer's job.
    let home = dirs_home().ok_or("no home")?;
    let local = home.join(".local/bin");
    std::fs::create_dir_all(&local).map_err(|e| e.to_string())?;
    let link = local.join(EXE);
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(dir.join(EXE), &link).map_err(|e| e.to_string())
}

#[cfg(unix)]
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
