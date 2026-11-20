// SPDX-License-Identifier: FSL-1.1-ALv2
//! Start at sign-in (D-027: on after the first run), for this user only.
//!
//! - Windows: a `HumanNeeded` value under `HKCU\…\Run` (and "enabled" under Task Manager's
//!   `StartupApproved`). Never the machine-wide key, even when the app runs as an administrator.
//! - macOS: `~/Library/LaunchAgents/HumanNeeded.plist`. It starts the app only if it is still
//!   there; if the app was dragged to the Trash, it deletes itself at the next sign-in.
//! - Linux: `~/.config/autostart/humanneeded.desktop`.
//!
//! The entry always points at the running copy: [`refresh`] rewrites it at start if it is on.

use std::path::{Path, PathBuf};

/// The entry's name (Run value, LaunchAgent label): the same name earlier builds used, so they
/// are found, updated and removed.
pub const NAME: &str = "HumanNeeded";
const ARG: &str = "--background";

pub fn is_enabled() -> bool {
    imp::is_enabled()
}

pub fn set(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if on {
        imp::enable(&exe)
    } else {
        imp::disable()
    }
}

/// At start: if it's on, point it at this copy of the app (moved, updated, or written by an
/// older build in an older format).
pub fn refresh() {
    if is_enabled() {
        let _ = set(true);
    }
}

/// Removes the entry, whatever wrote it (uninstall). Returns what it removed.
pub fn remove() -> Vec<String> {
    imp::remove_all()
}

/// The LaunchAgent: starts the app if it's still there, otherwise removes itself.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn launch_agent_plist(exe: &Path, plist: &Path) -> String {
    let q = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', r"'\''"));
    let script = format!(
        "if [ -x {exe} ]; then exec {exe} {ARG}; else rm -f {plist}; fi",
        exe = q(exe),
        plist = q(plist)
    );
    let x = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{NAME}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string>
    <string>-c</string>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#,
        x(&script)
    )
}

/// The Run value: the quoted path, then the argument.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn run_command(exe: &Path) -> String {
    format!("\"{}\" {ARG}", exe.display())
}

#[allow(dead_code)]
fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::{enums::*, RegKey};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    /// Task Manager's "enabled" (a first byte of 3 means the user disabled it there).
    const ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    pub fn is_enabled() -> bool {
        let Ok(run) = hkcu().open_subkey(RUN) else {
            return false;
        };
        if run.get_value::<String, _>(NAME).is_err() {
            return false;
        }
        // Turned off in Task Manager → Startup apps.
        let off = hkcu()
            .open_subkey(APPROVED)
            .and_then(|k| k.get_raw_value(NAME))
            .is_ok_and(|v| v.bytes.first().is_some_and(|b| b % 2 == 1));
        !off
    }

    pub fn enable(exe: &Path) -> Result<(), String> {
        let (run, _) = hkcu().create_subkey(RUN).map_err(|e| e.to_string())?;
        run.set_value(NAME, &run_command(exe))
            .map_err(|e| e.to_string())?;
        if let Ok((k, _)) = hkcu().create_subkey(APPROVED) {
            let _ = k.set_raw_value(
                NAME,
                &winreg::RegValue {
                    bytes: ENABLED.to_vec(),
                    vtype: REG_BINARY,
                },
            );
        }
        Ok(())
    }

    pub fn disable() -> Result<(), String> {
        remove_all();
        Ok(())
    }

    pub fn remove_all() -> Vec<String> {
        let mut done = Vec::new();
        for key in [RUN, APPROVED] {
            if let Ok(k) = hkcu().open_subkey_with_flags(key, KEY_READ | KEY_WRITE) {
                for name in [NAME, "humanneeded"] {
                    if k.delete_value(name).is_ok() {
                        done.push(format!(r"HKCU\{key}\{name}"));
                    }
                }
            }
        }
        // Builds before 1.0 could write the machine-wide key when run as an administrator.
        if let Ok(k) =
            RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(RUN, KEY_READ | KEY_WRITE)
        {
            if k.delete_value(NAME).is_ok() {
                done.push(format!(r"HKLM\{RUN}\{NAME}"));
            }
        }
        done
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    fn plist() -> Option<PathBuf> {
        Some(
            home()?
                .join("Library/LaunchAgents")
                .join(format!("{NAME}.plist")),
        )
    }

    pub fn is_enabled() -> bool {
        plist().is_some_and(|p| p.exists())
    }

    pub fn enable(exe: &Path) -> Result<(), String> {
        let p = plist().ok_or("no home folder")?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, launch_agent_plist(exe, &p)).map_err(|e| e.to_string())
    }

    pub fn disable() -> Result<(), String> {
        remove_all();
        Ok(())
    }

    pub fn remove_all() -> Vec<String> {
        let mut done = Vec::new();
        let Some(dir) = home().map(|h| h.join("Library/LaunchAgents")) else {
            return done;
        };
        for name in [NAME, "humanneeded"] {
            let p = dir.join(format!("{name}.plist"));
            if std::fs::remove_file(&p).is_ok() {
                done.push(p.display().to_string());
            }
        }
        done
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::*;

    fn file() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".config")))?;
        Some(config.join("autostart/humanneeded.desktop"))
    }

    pub fn is_enabled() -> bool {
        file().is_some_and(|p| p.exists())
    }

    pub fn enable(exe: &Path) -> Result<(), String> {
        let p = file().ok_or("no home folder")?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let body = format!(
            "[Desktop Entry]\nType=Application\nName={NAME}\nExec=\"{}\" {ARG}\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(p, body).map_err(|e| e.to_string())
    }

    pub fn disable() -> Result<(), String> {
        remove_all();
        Ok(())
    }

    pub fn remove_all() -> Vec<String> {
        file()
            .filter(|p| std::fs::remove_file(p).is_ok())
            .map(|p| vec![p.display().to_string()])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_agent_runs_the_app_or_removes_itself() {
        let p = launch_agent_plist(
            Path::new("/Applications/HumanNeeded.app/Contents/MacOS/humanneeded"),
            Path::new("/Users/pat/Library/LaunchAgents/HumanNeeded.plist"),
        );
        assert!(p.contains("<string>HumanNeeded</string>"));
        assert!(p.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(p.contains(
            "if [ -x '/Applications/HumanNeeded.app/Contents/MacOS/humanneeded' ]; then exec \
             '/Applications/HumanNeeded.app/Contents/MacOS/humanneeded' --background; \
             else rm -f '/Users/pat/Library/LaunchAgents/HumanNeeded.plist'; fi"
        ));
        // Quotes in a path stay inside the shell's quoting, and the XML stays valid.
        let p = launch_agent_plist(Path::new("/Users/pat/Pat's Apps/x"), Path::new("/p & q"));
        assert!(p.contains("'/Users/pat/Pat'\\''s Apps/x'") && p.contains("'/p &amp; q'"));
    }

    #[test]
    fn the_run_value_quotes_the_path() {
        assert_eq!(
            run_command(Path::new(
                r"C:\Users\pat\AppData\Local\HumanNeeded\humanneeded.exe"
            )),
            r#""C:\Users\pat\AppData\Local\HumanNeeded\humanneeded.exe" --background"#
        );
    }
}
