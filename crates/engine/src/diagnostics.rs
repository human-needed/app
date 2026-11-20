// SPDX-License-Identifier: FSL-1.1-ALv2
//! "Copy diagnostics" (Settings → Advanced): a plain-text summary for a bug report. Versions,
//! agents, the agent port, the cloud link, the puck, what's shown and the last log lines. No
//! keys, no session ids, no serial numbers; the user's home folder is shown as `~`.

use serde_json::Value;

/// Log lines at the end of the report.
pub const LOG_LINES: usize = 40;

/// The report, from the app's status (`Engine::status` plus the app's `update`), the end of the
/// log and the home folder to hide.
pub fn report(st: &Value, os: &str, log: &str, home: Option<&str>) -> String {
    let now = st["now"].as_u64().unwrap_or(0);
    let mut out = vec![
        "HumanNeeded diagnostics".to_string(),
        format!("App: {} ({os})", text(&st["app"]["version"])),
    ];

    let agents: Vec<String> = st["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let state = if a["enabled"] == false {
                "off".into()
            } else {
                text(&a["state"]).replace('_', " ")
            };
            match a["last_seen"].as_u64() {
                Some(t) => format!("{} {state}, last signal {}", text(&a["id"]), ago(now, t)),
                None => format!("{} {state}", text(&a["id"])),
            }
        })
        .collect();
    out.push(format!("Agents: {}", agents.join("; ")));

    let h = &st["http"];
    out.push(format!(
        "Agent port: {}{}",
        text(&h["port"]),
        match (h["listening"].as_bool(), h["error"].as_str()) {
            (_, Some(e)) => format!(", error: {e}"),
            (Some(true), _) => ", listening".into(),
            _ => ", not listening".into(),
        }
    ));
    if !st["proxy"].is_null() {
        out.push(format!("Claude hook proxy: {}", st["proxy"]));
    }

    let c = &st["cloud"];
    out.push(match c["error"].as_str() {
        Some(e) => format!("Cloud link: {}, error: {e}", text(&c["state"])),
        None => format!("Cloud link: {}", text(&c["state"])),
    });

    let p = &st["puck"];
    out.push(if p["connected"] == true {
        format!(
            "Puck: connected, firmware {}, board {}, protocol {}{}{}",
            text(&p["fw"]),
            text(&p["board"]),
            text(&p["proto"]),
            if p["compatible"] == false {
                ", NOT compatible"
            } else {
                ""
            },
            if p["updating"].is_null() {
                String::new()
            } else {
                format!(", updating {}", p["updating"])
            },
        )
    } else {
        "Puck: not connected".into()
    });

    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for e in st["snapshot"].as_array().into_iter().flatten() {
        *counts.entry(text(&e["state"])).or_default() += 1;
    }
    let counts: Vec<String> = counts.iter().map(|(s, n)| format!("{n} {s}")).collect();
    out.push(format!(
        "Sessions: {}",
        if counts.is_empty() {
            "none".into()
        } else {
            counts.join(", ")
        }
    ));
    out.push(format!(
        "Showing: computer {}, puck {}",
        or_idle(&st["output"]["shown"]),
        if p["connected"] == true {
            or_idle(&p["shown"])
        } else {
            "-".into()
        }
    ));

    let q = &st["quiet"];
    out.push(format!(
        "Quiet hours: {}",
        if q["enabled"] == true {
            format!(
                "{}-{}{}",
                text(&q["from"]),
                text(&q["to"]),
                if q["active"] == true { ", on now" } else { "" }
            )
        } else {
            "off".into()
        }
    ));
    let g = &st["glow"];
    if g.is_object() {
        out.push(format!(
            "Screen edges: {}, screens {}, thickness {}",
            if g["enabled"] == false {
                "paused"
            } else {
                "on"
            },
            text(&g["screens"]),
            text(&g["thickness"]),
        ));
    }
    let u = &st["update"];
    if u.is_object() {
        out.push(format!(
            "Updates: {}{}{}",
            if u["auto"] == true {
                "automatic"
            } else {
                "manual"
            },
            u["last"]
                .as_str()
                .map(|l| format!(", last check: {l}"))
                .unwrap_or_default(),
            u["ready"]
                .as_str()
                .map(|r| format!(", {r} ready"))
                .unwrap_or_default(),
        ));
    }

    out.push(String::new());
    out.push(format!("Last {LOG_LINES} log lines:"));
    let lines: Vec<&str> = log.lines().collect();
    out.extend(
        lines[lines.len().saturating_sub(LOG_LINES)..]
            .iter()
            .map(|l| l.to_string()),
    );
    let mut s = out.join("\n");
    if let Some(home) = home.filter(|h| h.len() > 3) {
        s = s.replace(home, "~");
        // Logs and JSON may spell it with escaped or forward slashes.
        s = s
            .replace(&home.replace('\\', "\\\\"), "~")
            .replace(&home.replace('\\', "/"), "~");
    }
    hide_session_ids(&s) + "\n"
}

/// Session ids (UUIDs, as the log writes them) become `<session>`.
fn hide_session_ids(s: &str) -> String {
    let b = s.as_bytes();
    let is_id = |w: &[u8]| {
        w.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
    };
    let (mut out, mut i, mut from) = (String::with_capacity(s.len()), 0, 0);
    while i + 36 <= b.len() {
        if is_id(&b[i..i + 36]) {
            out.push_str(&s[from..i]);
            out.push_str("<session>");
            i += 36;
            from = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[from..]);
    out
}

/// The end of the log file (the current one; empty if there is none).
pub fn log_tail() -> String {
    let Some(path) = crate::log::path() else {
        return String::new();
    };
    let s = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(LOG_LINES)..].join("\n")
}

/// The operating system and its version, e.g. `windows 10.0.26200 x86_64`, `macos 15.1 aarch64`.
pub fn os() -> String {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    match os_version() {
        Some(v) => format!("{os} {v} {arch}"),
        None => format!("{os} {arch}"),
    }
}

#[cfg(windows)]
fn os_version() -> Option<String> {
    let k = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?;
    let build: String = k.get_value("CurrentBuild").ok()?;
    let major: u32 = k.get_value("CurrentMajorVersionNumber").unwrap_or(10);
    let minor: u32 = k.get_value("CurrentMinorVersionNumber").unwrap_or(0);
    Some(format!("{major}.{minor}.{build}"))
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    let out = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|v| !v.is_empty())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn os_version() -> Option<String> {
    None
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        v => v.to_string(),
    }
}

fn or_idle(v: &Value) -> String {
    v.as_str().unwrap_or("idle").to_string()
}

fn ago(now: u64, t: u64) -> String {
    let s = now.saturating_sub(t) / 1000;
    match s {
        0..=89 => format!("{s} s ago"),
        90..=5399 => format!("{} min ago", s / 60),
        5400..=172_799 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status() -> Value {
        json!({
            "now": 1_000_000,
            "app": {"version": "1.0.0", "log": "C:\\Users\\pat\\AppData\\Local\\HumanNeeded\\logs\\app.log"},
            "agents": [
                {"id": "claude", "enabled": true, "state": "connected", "last_seen": 988_000},
                {"id": "codex", "enabled": true, "state": "not_set_up", "last_seen": null},
                {"id": "cursor", "enabled": false, "state": "not_set_up", "last_seen": null}
            ],
            "http": {"port": 24816, "listening": true, "error": null},
            "proxy": null,
            "cloud": {"state": "on", "error": null, "key": "hn_secret_key", "keys": ["hn_secret_key"]},
            "puck": {"connected": true, "fw": "2.0.7", "board": "hn-v1", "proto": 2, "compatible": true,
                     "serial": "HN505559851846571C", "shown": "needs", "updating": null},
            "snapshot": [{"id": "95c5bd66", "state": "working"}, {"id": "0000000a", "state": "needs"},
                         {"id": "0000000b", "state": "working"}],
            "sessions": [{"session": "a54e2709-0d74-46e2-bcb0-5e2cad1d0348"}],
            "output": {"shown": "needs"},
            "quiet": {"enabled": true, "from": "22:00", "to": "08:00", "active": false},
            "glow": {"enabled": true, "screens": "all", "thickness": "mid"},
            "update": {"auto": true, "checking": false, "ready": null, "last": "up to date"}
        })
    }

    #[test]
    fn summarises_everything_a_bug_report_needs() {
        let r = report(&status(), "windows 10.0.26200 x86_64", "", None);
        for want in [
            "App: 1.0.0 (windows 10.0.26200 x86_64)",
            "Agents: claude connected, last signal 12 s ago; codex not set up; cursor off",
            "Agent port: 24816, listening",
            "Cloud link: on",
            "Puck: connected, firmware 2.0.7, board hn-v1, protocol 2",
            "Sessions: 1 needs, 2 working",
            "Showing: computer needs, puck needs",
            "Quiet hours: 22:00-08:00",
            "Screen edges: on, screens all, thickness mid",
            "Updates: automatic, last check: up to date",
        ] {
            assert!(r.contains(want), "missing {want:?} in\n{r}");
        }
    }

    #[test]
    fn leaves_out_keys_ids_and_the_home_folder() {
        let log = "08:00:00.000Z event claude a54e2709-0d74-46e2-bcb0-5e2cad1d0348 Working\n08:00:00.500Z puck: connected\n08:00:01.000Z read C:\\Users\\pat\\.claude\\settings.json";
        let r = report(&status(), "windows", log, Some("C:\\Users\\pat"));
        for secret in [
            "hn_secret_key",
            "HN505559851846571C",
            "a54e2709",
            "95c5bd66",
            "pat",
        ] {
            assert!(!r.contains(secret), "{secret} leaked in\n{r}");
        }
        assert!(r.contains("read ~\\.claude\\settings.json"));
        assert!(r.contains("event claude <session> Working"));
    }

    #[test]
    fn keeps_only_the_last_log_lines() {
        let log: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let r = report(&status(), "macos", &log.join("\n"), None);
        assert!(r.contains("line 99") && r.contains("line 60") && !r.contains("line 59\n"));
        let r = report(&json!({}), "macos", "", None);
        assert!(r.contains("Puck: not connected") && r.contains("Sessions: none"));
    }
}
