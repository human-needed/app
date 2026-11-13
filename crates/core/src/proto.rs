//! Host side of `hn-usb/2` (pm/contracts/usb-protocol.md): command lines and reply parsing.

use serde::{Deserialize, Serialize};

pub const PROTO: u32 = 2;
pub const VID: u16 = 0x2E8A;
pub const PRODUCT: &str = "HumanNeeded";
pub const MAX_LINE: usize = 1024;
pub const MAX_ENTRIES: usize = 32;
/// Replies (e.g. SCHEMA) may be longer than a command line; anything beyond this is junk.
pub const MAX_REPLY: usize = 64 * 1024;
/// Lease the app sends with every snapshot (§3).
pub const LEASE_S: u32 = 30;
/// The app re-sends the snapshot at least this often (§3).
pub const RESEND_MS: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapState {
    Working,
    Needs,
}

impl SnapState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Needs => "needs",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "working" => Some(Self::Working),
            "needs" => Some(Self::Needs),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub name: String,
    pub fw: String,
    #[serde(default)]
    pub board: String,
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub proto: u32,
}

impl Hello {
    pub fn parse(line: &str) -> Result<Self, String> {
        let h: Hello =
            serde_json::from_str(line).map_err(|_| format!("not a HELLO reply: {line}"))?;
        if h.name != PRODUCT {
            return Err(format!("not a HumanNeeded puck ({})", h.name));
        }
        Ok(h)
    }
    /// Firmware speaks the protocol this app implements.
    pub fn compatible(&self) -> bool {
        self.proto == PROTO
    }
}

/// `SNAP <lease> id:state ...`; entries must already be valid (8 hex ids, ≤ 32).
pub fn snap_line(lease_s: u32, entries: &[(String, SnapState)]) -> String {
    let mut line = format!("SNAP {lease_s}");
    for (id, st) in entries.iter().take(MAX_ENTRIES) {
        line.push(' ');
        line.push_str(id);
        line.push(':');
        line.push_str(st.as_str());
    }
    line
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 8
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A reply line from the puck, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// `!…` unsolicited event (e.g. `!HELLO`).
    Unsolicited(String),
    Ok,
    Err(String),
    Json(serde_json::Value),
    Other(String),
}

pub fn classify(line: &str) -> Line {
    let line = line.trim_end_matches('\r');
    if let Some(rest) = line.strip_prefix('!') {
        return Line::Unsolicited(rest.to_string());
    }
    if line == "OK" {
        return Line::Ok;
    }
    if let Some(rest) = line.strip_prefix("ERR") {
        return Line::Err(rest.trim().to_string());
    }
    if line.starts_with('{') {
        if let Ok(v) = serde_json::from_str(line) {
            return Line::Json(v);
        }
    }
    Line::Other(line.to_string())
}

/// Splits a byte stream into lines (`\n`, `\r` ignored). Overlong lines are dropped.
#[derive(Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
    overflow: bool,
}

impl LineSplitter {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &b in bytes {
            match b {
                b'\n' => {
                    if !self.overflow {
                        out.push(String::from_utf8_lossy(&self.buf).into_owned());
                    }
                    self.buf.clear();
                    self.overflow = false;
                }
                b'\r' => {}
                _ => {
                    if self.buf.len() >= MAX_REPLY {
                        self.overflow = true;
                        self.buf.clear();
                    }
                    if !self.overflow {
                        self.buf.push(b);
                    }
                }
            }
        }
        out
    }
    pub fn clear(&mut self) {
        self.buf.clear();
        self.overflow = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap() {
        assert_eq!(snap_line(30, &[]), "SNAP 30");
        let e = vec![
            ("0000abcd".to_string(), SnapState::Needs),
            ("12345678".to_string(), SnapState::Working),
        ];
        assert_eq!(snap_line(30, &e), "SNAP 30 0000abcd:needs 12345678:working");
    }

    #[test]
    fn snap_caps_entries_and_fits_a_line() {
        let e: Vec<_> = (0..40)
            .map(|i| (format!("{i:08x}"), SnapState::Working))
            .collect();
        let line = snap_line(600, &e);
        assert_eq!(line.split(' ').count(), 2 + MAX_ENTRIES);
        assert!(line.len() < MAX_LINE);
    }

    #[test]
    fn hello() {
        let h = Hello::parse(
            r#"{"name":"HumanNeeded","fw":"2.0.0","board":"hn-v2","serial":"HN0011","proto":2}"#,
        )
        .unwrap();
        assert!(h.compatible());
        assert!(Hello::parse(r#"{"name":"Other","fw":"1"}"#).is_err());
        assert!(!Hello::parse(r#"{"name":"HumanNeeded","fw":"0.3.0"}"#)
            .unwrap()
            .compatible());
    }

    #[test]
    fn classify_lines() {
        assert_eq!(classify("!HELLO"), Line::Unsolicited("HELLO".into()));
        assert_eq!(classify("OK\r"), Line::Ok);
        assert_eq!(classify("ERR bad lease"), Line::Err("bad lease".into()));
        assert!(matches!(classify("{\"a\":1}"), Line::Json(_)));
        assert_eq!(classify("hm"), Line::Other("hm".into()));
    }

    #[test]
    fn splitter() {
        let mut s = LineSplitter::default();
        assert_eq!(s.push(b"OK\r\n!HEL"), vec!["OK"]);
        assert_eq!(s.push(b"LO\n"), vec!["!HELLO"]);
        let big = vec![b'x'; MAX_REPLY + 1];
        assert!(s.push(&big).is_empty());
        assert!(s.push(b"\n").is_empty());
        assert_eq!(s.push(b"OK\n"), vec!["OK"]);
    }

    #[test]
    fn ids() {
        assert!(valid_id("0a1b2c3d"));
        assert!(!valid_id("0A1B2C3D"));
        assert!(!valid_id("123"));
    }
}
