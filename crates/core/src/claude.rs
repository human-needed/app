// SPDX-License-Identifier: FSL-1.1-ALv2
//! Claude Code keeps `~/.claude/sessions/<pid>.json` per running local chat (undocumented, D-022):
//! `{"pid":…,"sessionId":"…","procStart":"…","status":"busy"|"idle","statusUpdatedAt":<ms>,…}`.
//! Pressing Stop flips `status` to `idle` with no hook; the file is removed when the chat ends.
//! Only the fields below are used; anything else may change without breaking us.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeStatus {
    Busy,
    Idle,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeSessionFile {
    pub pid: u32,
    pub session_id: String,
    /// Process start as Claude wrote it (Windows: FILETIME, 100 ns since 1601). Opaque elsewhere.
    pub proc_start: Option<String>,
    pub status: ClaudeStatus,
    /// Wall clock ms of the last status change.
    pub status_updated_at: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    pid: u32,
    session_id: String,
    #[serde(default)]
    proc_start: Option<serde_json::Value>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    status_updated_at: Option<u64>,
    #[serde(default)]
    updated_at: Option<u64>,
}

pub fn parse_session_file(text: &str) -> Option<ClaudeSessionFile> {
    let raw: Raw = serde_json::from_str(text).ok()?;
    let status = match raw.status.as_deref() {
        Some("busy") => ClaudeStatus::Busy,
        Some("idle") => ClaudeStatus::Idle,
        _ => ClaudeStatus::Unknown,
    };
    let proc_start = raw.proc_start.map(|v| match v {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    });
    Some(ClaudeSessionFile {
        pid: raw.pid,
        session_id: raw.session_id,
        proc_start,
        status,
        status_updated_at: raw.status_updated_at.or(raw.updated_at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_file() {
        let text = r#"{"pid":74768,"sessionId":"33ef1806-76de-4243-b013-e23e9f3d14c6","cwd":"C:\\x","startedAt":1794554906074,"procStart":"134390285039436844","version":"2.1.321","kind":"interactive","status":"busy","updatedAt":1794560016915,"statusUpdatedAt":1794560016915}"#;
        let f = parse_session_file(text).unwrap();
        assert_eq!(f.pid, 74768);
        assert_eq!(f.status, ClaudeStatus::Busy);
        assert_eq!(f.proc_start.as_deref(), Some("134390285039436844"));
        assert_eq!(f.status_updated_at, Some(1794560016915));
    }

    #[test]
    fn tolerates_missing_fields() {
        let f = parse_session_file(r#"{"pid":1,"sessionId":"s","procStart":123}"#).unwrap();
        assert_eq!(f.status, ClaudeStatus::Unknown);
        assert_eq!(f.proc_start.as_deref(), Some("123"));
        assert!(parse_session_file(r#"{"sessionId":"s"}"#).is_none());
        let w = parse_session_file(r#"{"pid":1,"sessionId":"s","status":"waiting"}"#).unwrap();
        assert_eq!(w.status, ClaudeStatus::Unknown);
    }
}
