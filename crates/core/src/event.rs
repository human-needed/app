//! `hn-events/1` (pm/contracts/app-events.md §2): one JSON line per message on the local IPC.
//!
//! Event:   `{"v":1,"agent":"claude","session":"…","state":"working","ts":1731500000000,"ttl":60}`
//! Request: `{"v":1,"cmd":"status","args":[]}`: app-internal, for the other `hn` commands (§4).
//! The app answers a request with one JSON line. Requests are not part of the public contract.

use serde::{Deserialize, Serialize};

pub const MAX_LINE: usize = 4096;
pub const MAX_SESSION_LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Working,
    Needs,
    Done,
    Idle,
    End,
}

impl AgentState {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "working" => Self::Working,
            "needs" => Self::Needs,
            "done" => Self::Done,
            "idle" => Self::Idle,
            "end" => Self::End,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,
    pub agent: String,
    pub session: String,
    pub state: AgentState,
    /// Milliseconds since the Unix epoch, set by `hn` when it ran.
    pub ts: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Event(Event),
    Request(Request),
}

pub fn valid_agent(a: &str) -> bool {
    (1..=32).contains(&a.len())
        && a.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn valid_session(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_SESSION_LEN && !s.chars().any(|c| c.is_control())
}

/// Parses and validates one IPC line. Anything malformed is rejected with a reason (for the log).
pub fn parse_line(line: &str) -> Result<Message, String> {
    if line.len() > MAX_LINE {
        return Err("line too long".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("bad json: {e}"))?;
    if value.get("v").and_then(|v| v.as_u64()) != Some(1) {
        return Err("unsupported v".into());
    }
    if value.get("cmd").is_some() {
        let r: Request = serde_json::from_value(value).map_err(|e| format!("bad request: {e}"))?;
        return Ok(Message::Request(r));
    }
    let e: Event = serde_json::from_value(value).map_err(|e| format!("bad event: {e}"))?;
    if !valid_agent(&e.agent) {
        return Err("bad agent".into());
    }
    if !valid_session(&e.session) {
        return Err("bad session".into());
    }
    if let Some(ttl) = e.ttl {
        if ttl == 0 || ttl > 86_400 {
            return Err("bad ttl".into());
        }
    }
    Ok(Message::Event(e))
}

/// Takes the session id out of a hook's stdin JSON (`session_id`, else `conversation_id`).
pub fn session_from_hook_json(input: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(input).ok()?;
    for key in ["session_id", "conversation_id"] {
        if let Some(s) = v.get(key).and_then(|s| s.as_str()) {
            if valid_session(s) {
                return Some(s.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_event() {
        let m = parse_line(r#"{"v":1,"agent":"claude","session":"abc","state":"needs","ts":5}"#)
            .unwrap();
        match m {
            Message::Event(e) => {
                assert_eq!(e.state, AgentState::Needs);
                assert_eq!(e.ttl, None);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn rejects_bad_fields() {
        assert!(
            parse_line(r#"{"v":2,"agent":"claude","session":"a","state":"needs","ts":5}"#).is_err()
        );
        assert!(
            parse_line(r#"{"v":1,"agent":"Claude","session":"a","state":"needs","ts":5}"#).is_err()
        );
        assert!(
            parse_line(r#"{"v":1,"agent":"claude","session":"","state":"needs","ts":5}"#).is_err()
        );
        assert!(
            parse_line(r#"{"v":1,"agent":"claude","session":"a","state":"busy","ts":5}"#).is_err()
        );
        assert!(parse_line(
            r#"{"v":1,"agent":"claude","session":"a","state":"needs","ts":5,"ttl":0}"#
        )
        .is_err());
        let long = "x".repeat(129);
        let line = format!(r#"{{"v":1,"agent":"a","session":"{long}","state":"needs","ts":5}}"#);
        assert!(parse_line(&line).is_err());
        assert!(parse_line("not json").is_err());
    }

    #[test]
    fn parses_request() {
        let m = parse_line(r#"{"v":1,"cmd":"mute","args":["10"]}"#).unwrap();
        assert_eq!(
            m,
            Message::Request(Request {
                v: 1,
                cmd: "mute".into(),
                args: vec!["10".into()]
            })
        );
    }

    #[test]
    fn hook_json_session() {
        assert_eq!(
            session_from_hook_json(br#"{"session_id":"s1","x":1}"#).as_deref(),
            Some("s1")
        );
        assert_eq!(
            session_from_hook_json(br#"{"conversation_id":"c1"}"#).as_deref(),
            Some("c1")
        );
        assert_eq!(session_from_hook_json(b"{}"), None);
        assert_eq!(session_from_hook_json(b"garbage"), None);
    }

    #[test]
    fn roundtrip() {
        let e = Event {
            v: 1,
            agent: "codex".into(),
            session: "s".into(),
            state: AgentState::Done,
            ts: 9,
            ttl: Some(30),
        };
        let line = serde_json::to_string(&e).unwrap();
        assert_eq!(parse_line(&line).unwrap(), Message::Event(e));
    }
}
