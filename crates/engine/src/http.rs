//! Local HTTP receiver (D-047): agents post hook events straight to the app.
//!
//! `POST http://127.0.0.1:<port>/v1/event/<agent>/<state>[?ttl=<s>]` with the hook's raw JSON as
//! the body. The session comes from the body's `session_id` (else `conversation_id`, else
//! `default`). Reply `204` as soon as the body is read. `GET /v1/health` answers `200` with the
//! app version (for integrations and support).
//!
//! Only this user's local programs should reach it, so:
//! - bound to 127.0.0.1 only;
//! - any `Origin` header → 403 (browsers always send one on cross-site POSTs);
//! - `Host` must be `127.0.0.1:<port>` or `localhost:<port>` (DNS-rebinding guard);
//! - headers ≤ 8 KB, body ≤ 1 MB (larger → 413, connection closed), 2 s to send it all;
//! - one request per connection.

use crate::ipc::Incoming;
use crate::log;
use hn_core::event::{
    session_from_hook_json, valid_agent, AgentState, Event, Message, MAX_SESSION_LEN,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// Default port, agreed with WS-PLUG (hooks hard-code it). `HN_HTTP_PORT` overrides it.
pub const DEFAULT_PORT: u16 = 47823;
const MAX_HEADERS: usize = 8 * 1024;
const MAX_BODY: usize = 1024 * 1024;
/// Only this much of the body is used to find the session id (app-events: first 16 KB).
const SESSION_SCAN: usize = 16 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HttpStatus {
    pub port: u16,
    pub listening: bool,
    /// Why it isn't listening (e.g. port in use), for the UI.
    pub error: Option<String>,
}

/// Starts the receiver. If the port is taken, keeps retrying every 10 s and reports it.
pub fn serve(
    port: u16,
    tx: mpsc::Sender<Incoming>,
    status: Arc<Mutex<HttpStatus>>,
    on_change: impl Fn() + Send + 'static,
) {
    *status.lock().unwrap() = HttpStatus {
        port,
        listening: false,
        error: None,
    };
    tokio::spawn(async move {
        let mut last_err = String::new();
        loop {
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => {
                    log!("http: listening on 127.0.0.1:{port}");
                    *status.lock().unwrap() = HttpStatus {
                        port,
                        listening: true,
                        error: None,
                    };
                    on_change();
                    loop {
                        let Ok((stream, peer)) = listener.accept().await else {
                            continue;
                        };
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let _ =
                                tokio::time::timeout(READ_TIMEOUT, handle(stream, port, tx)).await;
                        });
                    }
                }
                Err(e) => {
                    let msg = if e.kind() == std::io::ErrorKind::AddrInUse {
                        format!("port {port} is already used by another program")
                    } else {
                        format!("can't listen on port {port}: {e}")
                    };
                    if msg != last_err {
                        log!("http: {msg}");
                        *status.lock().unwrap() = HttpStatus {
                            port,
                            listening: false,
                            error: Some(msg.clone()),
                        };
                        on_change();
                        last_err = msg;
                    }
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    });
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body_start: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn reply(s: &mut TcpStream, code: u16, body: &str) {
    let reason = match code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let head = if body.is_empty() {
        format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    } else {
        format!(
            "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    let _ = s.write_all(head.as_bytes()).await;
    let _ = s.flush().await;
}

async fn read_head(s: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let end = loop {
        let n = s.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADERS {
            return None;
        }
    };
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Some(Request {
        method,
        path,
        headers,
        body_start: buf[end + 4..].to_vec(),
    })
}

/// Reads the body (Content-Length or chunked). Err(code) on a bad or oversized body.
async fn read_body(s: &mut TcpStream, req: &Request) -> Result<Vec<u8>, u16> {
    let mut body = req.body_start.clone();
    let chunked = req
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(decoded) = decode_chunked(&body) {
                return if decoded.len() > MAX_BODY {
                    Err(413)
                } else {
                    Ok(decoded)
                };
            }
            if body.len() > MAX_BODY + 64 * 1024 {
                return Err(413);
            }
            let n = s.read(&mut chunk).await.map_err(|_| 400u16)?;
            if n == 0 {
                return Err(400);
            }
            body.extend_from_slice(&chunk[..n]);
        }
    }
    let len: usize = match req.header("content-length") {
        Some(v) => v.parse().map_err(|_| 400u16)?,
        None => return Err(411),
    };
    if len > MAX_BODY {
        return Err(413);
    }
    let mut chunk = [0u8; 8192];
    while body.len() < len {
        let n = s.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Ok(body)
}

/// Decodes a complete chunked body; `None` until the final `0` chunk has arrived.
fn decode_chunked(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n")?;
        let size_str = std::str::from_utf8(&data[..line_end]).ok()?;
        let size = usize::from_str_radix(size_str.split(';').next()?.trim(), 16).ok()?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        if data.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

fn host_ok(host: Option<&str>, port: u16) -> bool {
    matches!(host, Some(h) if h == format!("127.0.0.1:{port}") || h.eq_ignore_ascii_case(&format!("localhost:{port}")))
}

/// `/v1/event/<agent>/<state>[?ttl=N]` → (agent, state, ttl).
pub fn parse_event_path(path: &str) -> Option<(String, AgentState, Option<u64>)> {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let rest = path.strip_prefix("/v1/event/")?;
    let (agent, state) = rest.split_once('/')?;
    if !valid_agent(agent) {
        return None;
    }
    let state = AgentState::parse(state)?;
    let ttl = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("ttl="))
        .and_then(|t| t.parse::<u64>().ok())
        .filter(|t| (1..=86_400).contains(t));
    Some((agent.to_string(), state, ttl))
}

/// Session id from the hook JSON: whole body if it parses, else only the first 16 KB is scanned.
pub fn session_of(body: &[u8]) -> String {
    if let Some(s) = session_from_hook_json(body) {
        return s;
    }
    let head = &body[..body.len().min(SESSION_SCAN)];
    let text = String::from_utf8_lossy(head);
    for key in ["\"session_id\"", "\"conversation_id\""] {
        if let Some(i) = text.find(key) {
            let after = text[i + key.len()..]
                .trim_start()
                .strip_prefix(':')
                .map(str::trim_start);
            if let Some(v) = after.and_then(|a| a.strip_prefix('"')) {
                if let Some(end) = v.find('"') {
                    let id = &v[..end];
                    if !id.is_empty()
                        && id.len() <= MAX_SESSION_LEN
                        && !id.chars().any(char::is_control)
                    {
                        return id.to_string();
                    }
                }
            }
        }
    }
    "default".into()
}

async fn handle(mut s: TcpStream, port: u16, tx: mpsc::Sender<Incoming>) {
    let Some(req) = read_head(&mut s).await else {
        return;
    };
    if req.header("origin").is_some() || !host_ok(req.header("host"), port) {
        reply(&mut s, 403, "").await;
        return;
    }
    if req.method == "GET" && req.path == "/v1/health" {
        let body = json!({"app": "humanneeded", "version": crate::engine::APP_VERSION}).to_string();
        reply(&mut s, 200, &body).await;
        return;
    }
    if !req.path.starts_with("/v1/event/") {
        reply(&mut s, 404, "").await;
        return;
    }
    if req.method != "POST" {
        reply(&mut s, 405, "").await;
        return;
    }
    let Some((agent, state, ttl)) = parse_event_path(&req.path) else {
        reply(&mut s, 400, "").await;
        return;
    };
    let body = match read_body(&mut s, &req).await {
        Ok(b) => b,
        Err(code) => {
            reply(&mut s, code, "").await;
            return;
        }
    };
    reply(&mut s, 204, "").await;
    let ev = Event {
        v: 1,
        agent,
        session: session_of(&body),
        state,
        ts: hn_sim::now_ms(),
        ttl,
    };
    let _ = tx
        .send(Incoming {
            msg: Message::Event(ev),
            reply: None,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(
            parse_event_path("/v1/event/claude/working").unwrap().1,
            AgentState::Working
        );
        assert_eq!(
            parse_event_path("/v1/event/codex/needs?ttl=60").unwrap().2,
            Some(60)
        );
        assert_eq!(
            parse_event_path("/v1/event/codex/needs?ttl=0").unwrap().2,
            None
        );
        assert!(parse_event_path("/v1/event/Claude/working").is_none());
        assert!(parse_event_path("/v1/event/claude/busy").is_none());
        assert!(parse_event_path("/v1/event/claude").is_none());
        assert!(parse_event_path("/v2/event/claude/done").is_none());
    }

    #[test]
    fn sessions_from_bodies() {
        assert_eq!(session_of(br#"{"session_id":"abc","x":1}"#), "abc");
        assert_eq!(session_of(br#"{"conversation_id":"c1"}"#), "c1");
        assert_eq!(session_of(b""), "default");
        assert_eq!(session_of(b"not json"), "default");
        // Truncated / invalid JSON: found by scanning the first 16 KB.
        assert_eq!(
            session_of(br#"{"session_id": "s9", "tool_input": {"x": "#),
            "s9"
        );
    }

    #[test]
    fn chunked() {
        assert_eq!(
            decode_chunked(b"4\r\nabcd\r\n2\r\nef\r\n0\r\n\r\n").unwrap(),
            b"abcdef"
        );
        assert!(decode_chunked(b"4\r\nab").is_none());
    }

    #[test]
    fn hosts() {
        assert!(host_ok(Some("127.0.0.1:47823"), 47823));
        assert!(host_ok(Some("localhost:47823"), 47823));
        assert!(!host_ok(Some("evil.com:47823"), 47823));
        assert!(!host_ok(Some("127.0.0.1"), 47823));
        assert!(!host_ok(None, 47823));
    }
}
