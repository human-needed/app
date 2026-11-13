//! Integration manifests (app-events §6, D-037): per agent app, how to detect it, whether the
//! HumanNeeded integration is installed, how to install/remove it, and manual instructions.
//! v1 installs Claude Code only; Codex and Cursor are detected and shown as "coming".

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const CLAUDE_MARKETPLACE: &str = "human-needed/plugins";
pub const CLAUDE_PLUGIN: &str = "humanneeded@humanneeded";

#[derive(Debug, Clone, Serialize)]
pub struct Integration {
    pub id: &'static str,
    pub name: &'static str,
    /// The agent app is on this computer.
    pub detected: bool,
    /// The HumanNeeded integration is installed and enabled.
    pub installed: bool,
    /// The app can install it with one click.
    pub can_install: bool,
    pub manual: &'static str,
}

pub struct Env {
    pub home: Option<PathBuf>,
    pub claude_dir: Option<PathBuf>,
    pub path: Option<std::ffi::OsString>,
}

impl Env {
    pub fn current() -> Self {
        Self { home: dirs::home_dir(), claude_dir: crate::paths::claude_dir(), path: std::env::var_os("PATH") }
    }
}

/// Finds an executable on PATH (Windows: .exe, .cmd, .bat too).
pub fn which(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    for dir in std::env::split_paths(path?) {
        for ext in exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn claude_installed(claude_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(claude_dir.join("settings.json")) else { return false };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    v.get("enabledPlugins")
        .and_then(|p| p.as_object())
        .is_some_and(|m| m.iter().any(|(k, on)| k.starts_with("humanneeded@") && on.as_bool() == Some(true)))
}

pub fn list(env: &Env) -> Vec<Integration> {
    let has = |n: &str| which(n, env.path.as_deref()).is_some();
    let home_has = |d: &str| env.home.as_ref().is_some_and(|h| h.join(d).is_dir());
    let claude_dir_exists = env.claude_dir.as_ref().is_some_and(|d| d.is_dir());
    vec![
        Integration {
            id: "claude",
            name: "Claude Code",
            detected: has("claude") || claude_dir_exists,
            installed: env.claude_dir.as_deref().is_some_and(claude_installed),
            can_install: has("claude"),
            manual: "In Claude Code run: /plugin marketplace add human-needed/plugins, then /plugin install humanneeded@humanneeded, then restart Claude Code.",
        },
        Integration {
            id: "codex",
            name: "Codex",
            detected: has("codex") || home_has(".codex"),
            installed: false,
            can_install: false,
            manual: "Coming soon.",
        },
        Integration {
            id: "cursor",
            name: "Cursor",
            detected: has("cursor") || has("cursor-agent") || home_has(".cursor"),
            installed: false,
            can_install: false,
            manual: "Coming soon.",
        },
    ]
}

fn run(exe: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("{}: {e}", exe.display()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!("`claude {}` failed: {}", args.join(" "), text.trim()))
    }
}

/// One-click install (Claude Code only in v1).
pub fn install(id: &str, env: &Env) -> Result<String, String> {
    match id {
        "claude" => {
            let exe = which("claude", env.path.as_deref()).ok_or("Claude Code's `claude` command wasn't found")?;
            // Adding an already-known marketplace fails harmlessly; install is what matters.
            let _ = run(&exe, &["plugin", "marketplace", "add", CLAUDE_MARKETPLACE]);
            run(&exe, &["plugin", "install", CLAUDE_PLUGIN, "--scope", "user"])?;
            Ok("Installed. Restart Claude Code sessions to load it.".into())
        }
        _ => Err("not available yet".into()),
    }
}

pub fn uninstall(id: &str, env: &Env) -> Result<String, String> {
    match id {
        "claude" => {
            let exe = which("claude", env.path.as_deref()).ok_or("Claude Code's `claude` command wasn't found")?;
            run(&exe, &["plugin", "uninstall", CLAUDE_PLUGIN])?;
            Ok("Removed.".into())
        }
        _ => Err("not available yet".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_claude_plugin_state() {
        let home = tempfile::tempdir().unwrap();
        let cdir = home.path().join(".claude");
        std::fs::create_dir(&cdir).unwrap();
        let env = Env { home: Some(home.path().into()), claude_dir: Some(cdir.clone()), path: None };
        let c = &list(&env)[0];
        assert!(c.detected && !c.installed && !c.can_install);

        std::fs::write(cdir.join("settings.json"), r#"{"enabledPlugins":{"humanneeded@humanneeded":false}}"#).unwrap();
        assert!(!list(&env)[0].installed);
        std::fs::write(cdir.join("settings.json"), r#"{"enabledPlugins":{"humanneeded@humanneeded":true}}"#).unwrap();
        assert!(list(&env)[0].installed);
    }

    #[test]
    fn nothing_detected_in_empty_home() {
        let home = tempfile::tempdir().unwrap();
        let env = Env { home: Some(home.path().into()), claude_dir: Some(home.path().join(".claude")), path: None };
        assert!(list(&env).iter().all(|i| !i.detected && !i.installed));
    }

    #[test]
    fn which_finds_executables() {
        let d = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "claude.cmd" } else { "claude" };
        std::fs::write(d.path().join(name), "").unwrap();
        let path = std::env::join_paths([d.path()]).unwrap();
        assert!(which("claude", Some(&path)).is_some());
        assert!(which("codex", Some(&path)).is_none());
    }
}
