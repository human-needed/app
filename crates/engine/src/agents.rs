//! Read-only agent status (D-054): the user installs each plugin themselves; the app only says
//! whether it has heard from that agent lately. "Connected" = an event from it arrived in the last
//! [`RECENT_MS`]. Last-seen times survive restarts in `data_dir/agents.json` (`{"<id>": ms}`).
//!
//! D-055: each agent has a switch ("lights the puck"). An agent switched off is still seen (its
//! status stays honest) but its events don't reach the session table. Kept in
//! `agent-switches.json` next to `agents.json` (`{"<id>": false}`; missing = on).

use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Where the per-agent setup guides live (WS-WEB).
pub const SETUP_URL: &str = "https://human-needed.com/start/";

/// How long an agent stays "connected" after its last event: 7 days.
pub const RECENT_MS: u64 = 7 * 24 * 3600 * 1000;

/// Don't rewrite the file for every event: only when the stored time is this much older.
const SAVE_EVERY_MS: u64 = 60 * 1000;

/// The agents the window lists, in order: (event agent id, name).
pub const AGENTS: &[(&str, &str)] = &[
    ("claude", "Claude"),
    ("codex", "Codex"),
    ("cursor", "Cursor"),
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentStatus {
    pub id: String,
    pub name: String,
    /// `connected` | `not_set_up`.
    pub state: &'static str,
    /// Last event (ms since epoch), if any.
    pub last_seen: Option<u64>,
    /// Lights the puck (switch, D-055).
    pub enabled: bool,
}

pub struct Seen {
    file: Option<PathBuf>,
    last: BTreeMap<String, u64>,
    saved: BTreeMap<String, u64>,
    off: BTreeSet<String>,
}

fn switches_file(file: &Option<PathBuf>) -> Option<PathBuf> {
    Some(file.as_ref()?.with_file_name("agent-switches.json"))
}

impl Seen {
    /// Loads `file` (missing or bad = nothing seen yet). `None` = memory only.
    pub fn load(file: Option<PathBuf>) -> Self {
        let last: BTreeMap<String, u64> = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str::<Map<String, Value>>(&s).ok())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| Some((k, v.as_u64()?)))
                    .collect()
            })
            .unwrap_or_default();
        let off = switches_file(&file)
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str::<Map<String, Value>>(&s).ok())
            .map(|m| {
                m.into_iter()
                    .filter(|(_, v)| v == &Value::Bool(false))
                    .map(|(k, _)| k)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            file,
            saved: last.clone(),
            last,
            off,
        }
    }

    /// Whether events from `agent` light the puck. Unknown agents (e.g. `claude-cloud`, which has
    /// its own switch: the cloud link) always do.
    pub fn enabled(&self, agent: &str) -> bool {
        !self.off.contains(agent)
    }

    /// Switches an agent on or off (saved at once).
    pub fn set_enabled(&mut self, agent: &str, on: bool) -> Result<(), String> {
        if !AGENTS.iter().any(|(id, _)| *id == agent) {
            return Err(format!("unknown agent {agent}"));
        }
        if on {
            self.off.remove(agent);
        } else {
            self.off.insert(agent.to_string());
        }
        let Some(f) = switches_file(&self.file) else {
            return Ok(());
        };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let m: Map<String, Value> = self
            .off
            .iter()
            .map(|k| (k.clone(), Value::Bool(false)))
            .collect();
        std::fs::write(f, Value::Object(m).to_string()).map_err(|e| e.to_string())
    }

    /// An event from `agent` arrived at `now`.
    pub fn saw(&mut self, agent: &str, now: u64) {
        if !AGENTS.iter().any(|(id, _)| *id == agent) {
            return;
        }
        self.last.insert(agent.to_string(), now);
        let stale = self
            .saved
            .get(agent)
            .is_none_or(|t| now.saturating_sub(*t) >= SAVE_EVERY_MS);
        if stale {
            self.save();
        }
    }

    fn save(&mut self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("json.tmp");
        let body = serde_json::to_string(&self.last).unwrap_or_default();
        if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, file).is_ok() {
            self.saved = self.last.clone();
        }
    }

    pub fn list(&self, now: u64) -> Vec<AgentStatus> {
        AGENTS
            .iter()
            .map(|(id, name)| {
                let last_seen = self.last.get(*id).copied();
                let recent = last_seen.is_some_and(|t| now.saturating_sub(t) < RECENT_MS);
                AgentStatus {
                    id: id.to_string(),
                    name: name.to_string(),
                    state: if recent { "connected" } else { "not_set_up" },
                    last_seen,
                    enabled: self.enabled(id),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(s: &Seen, id: &str, now: u64) -> &'static str {
        s.list(now).into_iter().find(|a| a.id == id).unwrap().state
    }

    #[test]
    fn connected_only_after_a_recent_event() {
        let mut s = Seen::load(None);
        let ids: Vec<_> = s.list(0).into_iter().map(|a| a.id).collect();
        assert_eq!(ids, ["claude", "codex", "cursor"]);
        assert!(s.list(0).iter().all(|a| a.state == "not_set_up"));
        s.saw("codex", 1_000);
        s.saw("someone-else", 1_000);
        assert_eq!(state(&s, "codex", 2_000), "connected");
        assert_eq!(state(&s, "claude", 2_000), "not_set_up");
        assert_eq!(state(&s, "codex", 1_000 + RECENT_MS), "not_set_up");
    }

    #[test]
    fn last_seen_survives_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("agents.json");
        let mut s = Seen::load(Some(f.clone()));
        s.saw("claude", 5_000);
        // Within a minute: not rewritten (still the first time on disk).
        s.saw("claude", 6_000);
        let back = Seen::load(Some(f.clone()));
        assert_eq!(back.list(7_000)[0].last_seen, Some(5_000));
        assert_eq!(state(&back, "claude", 7_000), "connected");
        s.saw("claude", 5_000 + SAVE_EVERY_MS);
        assert_eq!(
            Seen::load(Some(f)).list(0)[0].last_seen,
            Some(5_000 + SAVE_EVERY_MS)
        );
    }

    #[test]
    fn switches_persist_and_default_on() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("agents.json");
        let mut s = Seen::load(Some(f.clone()));
        assert!(s.enabled("codex") && s.enabled("claude-cloud"));
        s.set_enabled("codex", false).unwrap();
        assert!(s.set_enabled("nobody", false).is_err());
        let back = Seen::load(Some(f.clone()));
        assert!(!back.enabled("codex") && back.enabled("claude"));
        assert!(!back.list(0)[1].enabled);
        let mut back = back;
        back.set_enabled("codex", true).unwrap();
        assert!(Seen::load(Some(f)).enabled("codex"));
    }

    #[test]
    fn bad_file_means_nothing_seen() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("agents.json");
        std::fs::write(&f, "not json").unwrap();
        assert!(Seen::load(Some(f))
            .list(0)
            .iter()
            .all(|a| a.last_seen.is_none()));
    }
}
