// SPDX-License-Identifier: FSL-1.1-ALv2
//! Clean uninstall (R-05): nothing of the app stays behind, except the user's settings if they
//! choose to keep them.
//!
//! - Windows: the uninstaller (Apps → Installed apps, or Settings → Advanced → Uninstall) runs
//!   `humanneeded.exe --uninstall [--purge]` before it deletes the program (`windows/hooks.nsh`).
//! - macOS: Settings → Advanced → Uninstall cleans up, moves the app to the Trash and quits.
//!
//! Removed: the start-at-login entry, the cloud link's credential, `hn` on PATH (1.0 builds),
//! the log, the web view's cache, and the settings unless kept. Your agents' plugins and hooks
//! are theirs: remove them in each agent (README).

use hn_engine::paths;
use std::path::{Path, PathBuf};

/// The user's settings in the data folder (kept when they ask).
const SETTINGS: [&str; 6] = [
    "alerts.json",
    "agents.json",
    "app-settings.json",
    "cloud.json",
    "glow.json",
    "seen",
];

/// The app's identifier (tauri.conf.json): the web view keeps its data under it.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
const BUNDLE_ID: &str = "com.human-needed.app";

/// Removes what the app left on this computer. Returns what it did, for the log.
pub fn clean(keep_settings: bool) -> Vec<String> {
    let mut done = Vec::new();
    done.extend(remove_login_item());
    if let Some(m) = crate::hn_install::retire() {
        done.push(format!("hn: {m}"));
    }
    if remove_credential() {
        done.push("cloud link credential".into());
    }
    let data = paths::data_dir();
    done.extend(clear_data(&data, keep_settings));
    for dir in webview_dirs() {
        if dir.exists() && std::fs::remove_dir_all(&dir).is_ok() {
            done.push(dir.display().to_string());
        }
    }
    done
}

/// Empties the data folder, keeping the settings if asked and never touching the program
/// itself (on Windows the app installs into the same folder). Removes the folder if empty.
fn clear_data(dir: &Path, keep_settings: bool) -> Vec<String> {
    let mut done = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return done;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let program = name.eq_ignore_ascii_case("humanneeded.exe")
            || name.eq_ignore_ascii_case("uninstall.exe");
        if program || (keep_settings && SETTINGS.contains(&name.as_str())) {
            continue;
        }
        let p = e.path();
        let ok = if p.is_dir() {
            std::fs::remove_dir_all(&p).is_ok()
        } else {
            std::fs::remove_file(&p).is_ok()
        };
        if ok {
            done.push(name);
        }
    }
    if std::fs::remove_dir(dir).is_ok() {
        done.push(dir.display().to_string());
    }
    done
}

/// The cloud link's identity, in Credential Manager / the Keychain (engine `cloud::OsCreds`).
fn remove_credential() -> bool {
    use hn_engine::cloud::CredStore;
    let creds = hn_engine::cloud::OsCreds {
        service: "HumanNeeded".into(),
    };
    let had = creds.load().is_some();
    creds.delete();
    had
}

/// The web view's own data (cache, storage), named after the identifier.
#[allow(unused_mut)]
fn webview_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        v.extend(dirs::data_local_dir().map(|d| d.join(BUNDLE_ID)));
        v.extend(dirs::data_dir().map(|d| d.join(BUNDLE_ID)));
    }
    #[cfg(target_os = "macos")]
    if let Some(h) = dirs::home_dir() {
        for d in [
            "Library/Caches",
            "Library/WebKit",
            "Library/Application Support",
            "Library/HTTPStorages",
        ] {
            v.push(h.join(d).join(BUNDLE_ID));
        }
        v.push(
            h.join("Library/Saved Application State")
                .join(format!("{BUNDLE_ID}.savedState")),
        );
    }
    v
}

/// The start-at-login entry (tauri-plugin-autostart): a value under the Run key on Windows, a
/// LaunchAgent on macOS. Named after the app.
#[allow(unused_mut)]
fn remove_login_item() -> Vec<String> {
    let mut done = Vec::new();
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for key in [
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
        ] {
            if let Ok(k) = hkcu.open_subkey_with_flags(key, KEY_READ | KEY_WRITE) {
                for name in ["HumanNeeded", "humanneeded"] {
                    if k.delete_value(name).is_ok() {
                        done.push(format!("login item ({key})"));
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(h) = dirs::home_dir() {
        for name in ["HumanNeeded", "humanneeded"] {
            let p = h.join("Library/LaunchAgents").join(format!("{name}.plist"));
            if std::fs::remove_file(&p).is_ok() {
                done.push(format!("login item ({})", p.display()));
            }
        }
    }
    done
}

/// `humanneeded --uninstall [--purge]`: called by the Windows uninstaller before it removes the
/// program. Runs before the tray app starts (and before single-instance), then exits.
pub fn from_args() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|a| a == "--uninstall") {
        return false;
    }
    // Nothing is logged: the log is one of the things removed.
    clean(!args.iter().any(|a| a == "--purge"));
    true
}

/// Settings → Advanced → Uninstall.
/// - Windows: opens the uninstaller (it asks, then runs `--uninstall`), and the app quits.
/// - macOS: cleans up, moves the app to the Trash, and quits.
pub fn start(app: &tauri::AppHandle, keep_settings: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let un = exe.with_file_name("uninstall.exe");
        if !un.exists() {
            return Err("No uninstaller here: remove the app from Settings → Apps".into());
        }
        let mut cmd = std::process::Command::new(un);
        if !keep_settings {
            cmd.arg("/DELETEAPPDATA"); // ticks "Delete the application data" for the user
        }
        cmd.spawn().map_err(|e| e.to_string())?;
        hn_engine::log!("uninstall: opened the uninstaller");
        app.exit(0);
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        clean(keep_settings);
        // The .app bundle: …/HumanNeeded.app/Contents/MacOS/humanneeded.
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if let Some(bundle) = exe
            .ancestors()
            .find(|p| p.extension().is_some_and(|x| x == "app"))
        {
            if trash(bundle).is_err() {
                // Couldn't move it: show it, so the user can drag it to the Trash.
                let _ = std::process::Command::new("open")
                    .arg("-R")
                    .arg(bundle)
                    .spawn();
            }
        }
        app.exit(0);
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (app, keep_settings);
        Err("Uninstall from your package manager".into())
    }
}

/// Moves a path to the user's Trash (Finder, so it can be put back).
#[cfg(target_os = "macos")]
fn trash(p: &Path) -> Result<(), String> {
    let script = format!(
        "tell application \"Finder\" to delete POSIX file \"{}\"",
        p.display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    );
    let ok = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if ok {
        Ok(())
    } else {
        Err("Finder refused".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(d: &Path) {
        for f in [
            "alerts.json",
            "agents.json",
            "glow.json",
            "seen",
            "humanneeded.exe",
            "uninstall.exe",
            "x.tmp",
        ] {
            std::fs::write(d.join(f), "1").unwrap();
        }
        std::fs::create_dir_all(d.join("logs")).unwrap();
        std::fs::write(d.join("logs/app.log"), "1").unwrap();
        std::fs::create_dir_all(d.join("bin")).unwrap();
    }

    fn names(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn keeps_only_settings_and_the_program() {
        let t = tempfile::tempdir().unwrap();
        fill(t.path());
        clear_data(t.path(), true);
        assert_eq!(
            names(t.path()),
            [
                "agents.json",
                "alerts.json",
                "glow.json",
                "humanneeded.exe",
                "seen",
                "uninstall.exe"
            ]
        );
    }

    #[test]
    fn purge_leaves_only_the_program_or_nothing() {
        let t = tempfile::tempdir().unwrap();
        fill(t.path());
        clear_data(t.path(), false);
        assert_eq!(names(t.path()), ["humanneeded.exe", "uninstall.exe"]);
        // Without the program in it (macOS, or another install folder), the folder goes too.
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("HumanNeeded");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("alerts.json"), "1").unwrap();
        clear_data(&d, false);
        assert!(!d.exists());
    }
}
