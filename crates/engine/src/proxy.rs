// SPDX-License-Identifier: FSL-1.1-ALv2
//! D-049 (app-events 2.0 §5): Claude sends its HTTP hooks through `HTTP(S)_PROXY` unless
//! `NO_PROXY` contains `127.0.0.1` (`localhost` alone is not enough). Then the hooks never reach
//! the app and the puck stays dark, so the app warns, with a one-line fix.
//!
//! Where Claude gets these variables, highest first: the `env` block of
//! `~/.claude/settings.json`, then the user's environment (Windows: the registry, so a change
//! shows without restarting the app; user over machine; then this process's own environment).

use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProxyWarning {
    /// What's wrong, in plain words.
    pub problem: String,
    /// One line the user can run or apply.
    pub fix: String,
}

/// Where a variable's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    ClaudeSettings,
    Os,
}

/// The variables as Claude would see them.
#[derive(Debug, Default)]
pub struct Vars {
    /// `env` from `~/.claude/settings.json`.
    pub claude: HashMap<String, String>,
    /// The user's environment.
    pub os: HashMap<String, String>,
    pub windows: bool,
}

impl Vars {
    /// Looks up `NAME` (and `name` too, as proxies honour both).
    fn get(&self, name: &str) -> Option<(String, Source)> {
        let lower = name.to_ascii_lowercase();
        for (map, src) in [
            (&self.claude, Source::ClaudeSettings),
            (&self.os, Source::Os),
        ] {
            for key in [name, lower.as_str()] {
                let hit = if self.windows {
                    // Windows environment names are case-insensitive.
                    map.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))
                } else {
                    map.get_key_value(key)
                };
                if let Some((_, v)) = hit {
                    if !v.trim().is_empty() {
                        return Some((v.trim().to_string(), src));
                    }
                }
            }
        }
        None
    }
}

/// `NO_PROXY` lets 127.0.0.1 through (an entry `127.0.0.1` or `*`).
pub fn bypasses_loopback(no_proxy: &str) -> bool {
    no_proxy
        .split([',', ' ', ';'])
        .map(str::trim)
        .any(|e| e == "127.0.0.1" || e == "*")
}

pub fn check(v: &Vars) -> Option<ProxyWarning> {
    let (name, (_, proxy_src)) = ["HTTPS_PROXY", "HTTP_PROXY"]
        .into_iter()
        .find_map(|n| v.get(n).map(|x| (n, x)))?;
    let no_proxy = v.get("NO_PROXY");
    if no_proxy
        .as_ref()
        .is_some_and(|(np, _)| bypasses_loopback(np))
    {
        return None;
    }
    let new_value = match &no_proxy {
        Some((np, _)) => format!("{},127.0.0.1", np.trim_end_matches(',')),
        None => "localhost,127.0.0.1".to_string(),
    };
    let in_settings = proxy_src == Source::ClaudeSettings
        || no_proxy
            .as_ref()
            .is_some_and(|(_, s)| *s == Source::ClaudeSettings);
    let fix = if in_settings {
        format!(
            r#"In ~/.claude/settings.json, under "env", set "NO_PROXY": "{new_value}", then start a new Claude session."#
        )
    } else if v.windows {
        format!(r#"Run: setx NO_PROXY "{new_value}" — then restart Claude."#)
    } else {
        format!(
            r#"Add to your shell profile: export NO_PROXY="{new_value}" — then restart Claude."#
        )
    };
    Some(ProxyWarning {
        problem: format!(
            "{name} is set, so Claude sends its puck updates to your proxy and they never arrive."
        ),
        fix,
    })
}

/// Reads the variables now and checks them.
pub fn current(claude_dir: Option<&Path>) -> Option<ProxyWarning> {
    let mut v = Vars {
        windows: cfg!(windows),
        ..Default::default()
    };
    if let Some(dir) = claude_dir {
        if let Ok(text) = std::fs::read_to_string(dir.join("settings.json")) {
            if let Ok(s) = serde_json::from_str::<serde_json::Value>(&text) {
                for (k, val) in s["env"].as_object().into_iter().flatten() {
                    if let Some(val) = val.as_str() {
                        v.claude.insert(k.clone(), val.to_string());
                    }
                }
            }
        }
    }
    v.os = os_env();
    check(&v)
}

const NAMES: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
];

#[cfg(windows)]
fn os_env() -> HashMap<String, String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let mut out = HashMap::new();
    // Lowest first, so higher sources overwrite.
    for n in NAMES {
        if let Ok(val) = std::env::var(n) {
            out.insert(n.to_ascii_uppercase(), val);
        }
    }
    let keys = [
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
        RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment"),
    ];
    for key in keys.into_iter().flatten() {
        for n in ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY"] {
            if let Ok(val) = key.get_value::<String, _>(n) {
                out.insert(n.to_string(), val);
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn os_env() -> HashMap<String, String> {
    NAMES
        .iter()
        .filter_map(|n| std::env::var(n).ok().map(|v| (n.to_string(), v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(os: &[(&str, &str)], claude: &[(&str, &str)], windows: bool) -> Vars {
        let m = |kv: &[(&str, &str)]| {
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        Vars {
            claude: m(claude),
            os: m(os),
            windows,
        }
    }

    #[test]
    fn no_proxy_no_warning() {
        assert_eq!(check(&vars(&[], &[], true)), None);
        assert_eq!(check(&vars(&[("NO_PROXY", "x")], &[], true)), None);
    }

    #[test]
    fn proxy_without_loopback_bypass_warns_with_a_fix() {
        let w = check(&vars(&[("HTTPS_PROXY", "http://p:8080")], &[], true)).unwrap();
        assert!(w.problem.contains("HTTPS_PROXY"), "{}", w.problem);
        assert_eq!(
            w.fix,
            r#"Run: setx NO_PROXY "localhost,127.0.0.1" — then restart Claude."#
        );
        // localhost alone is not enough (Claude still proxies 127.0.0.1).
        let w = check(&vars(
            &[("http_proxy", "http://p"), ("no_proxy", "localhost,.corp")],
            &[],
            false,
        ))
        .unwrap();
        assert!(
            w.fix
                .contains(r#"export NO_PROXY="localhost,.corp,127.0.0.1""#),
            "{}",
            w.fix
        );
    }

    #[test]
    fn loopback_bypass_or_star_is_fine() {
        for np in ["127.0.0.1", "localhost, 127.0.0.1", "*", ".corp,127.0.0.1,"] {
            assert_eq!(
                check(&vars(
                    &[("HTTP_PROXY", "http://p"), ("NO_PROXY", np)],
                    &[],
                    true
                )),
                None,
                "{np}"
            );
        }
        assert!(!bypasses_loopback("127.0.0.10"));
    }

    #[test]
    fn claude_settings_win_and_get_the_fix() {
        // NO_PROXY fine in the OS, but settings.json overrides it.
        let w = check(&vars(
            &[("HTTPS_PROXY", "http://p"), ("NO_PROXY", "127.0.0.1")],
            &[("NO_PROXY", "localhost")],
            true,
        ))
        .unwrap();
        assert!(w.fix.contains("settings.json"), "{}", w.fix);
        assert!(
            w.fix.contains(r#""NO_PROXY": "localhost,127.0.0.1""#),
            "{}",
            w.fix
        );
        // Proxy only in settings.json.
        let w = check(&vars(&[], &[("HTTPS_PROXY", "http://p")], false)).unwrap();
        assert!(w.fix.contains("settings.json"), "{}", w.fix);
        // Settings fix it.
        assert_eq!(
            check(&vars(
                &[("HTTPS_PROXY", "http://p")],
                &[("NO_PROXY", "127.0.0.1")],
                true
            )),
            None
        );
    }

    #[test]
    fn windows_names_are_case_insensitive() {
        assert!(check(&vars(&[("Https_Proxy", "http://p")], &[], true)).is_some());
        assert!(check(&vars(&[("Https_Proxy", "http://p")], &[], false)).is_none());
    }

    #[test]
    fn reads_claude_settings_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("settings.json"),
            r#"{"env":{"HTTPS_PROXY":"http://p:1","NO_PROXY":"localhost"}}"#,
        )
        .unwrap();
        let w = current(Some(d.path())).unwrap();
        assert!(w.fix.contains("settings.json"), "{}", w.fix);
    }
}
