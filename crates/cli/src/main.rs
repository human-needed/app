//! `hn`: the one command agent integrations call (pm/contracts/app-events.md §1–4).
//!
//! `hn event …` must never hurt an agent: it always exits 0 within 1 s, prints nothing, and does
//! nothing if the app isn't running. The other commands are for people and skills and print JSON.

use hn_core::endpoint::ipc_endpoint;
use hn_core::event::{
    session_from_hook_json, valid_agent, valid_session, AgentState, Event, Request,
};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EVENT_BUDGET: Duration = Duration::from_millis(900);
const STDIN_LIMIT: u64 = 16 * 1024;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("event") => {
            event(&args[1..]);
            0
        }
        Some("version") | Some("--version") | Some("-V") => {
            println!(
                "{}",
                serde_json::json!({ "hn": env!("CARGO_PKG_VERSION"), "contract": hn_core::event::CONTRACT, "target": env!("HN_TARGET") })
            );
            0
        }
        Some("status") => request("status", vec![]),
        Some("test") if args.len() == 2 => request("test", vec![args[1].clone()]),
        Some("mute") if args.len() == 2 => request("mute", vec![args[1].clone()]),
        Some("set") if args.len() == 3 => request("set", vec![args[1].clone(), args[2].clone()]),
        Some("clear") => request("clear", vec![]),
        _ => {
            eprintln!(
                "usage:\n  hn event --agent <agent> --state <working|needs|done|idle|end> [--session <id>] [--stdin-json] [--ttl <s>]\n  hn status | hn test <needs|done|working|idle|hello> | hn mute <minutes|on|0> | hn set <key> <value> | hn clear | hn version"
            );
            2
        }
    };
    std::process::exit(code);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Parses `hn event` args. Returns None on anything invalid (silently ignored).
fn parse_event(args: &[String], stdin: impl FnOnce() -> Option<Vec<u8>>) -> Option<Event> {
    let (mut agent, mut state, mut session, mut ttl, mut use_stdin) =
        (None, None, None, None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--agent" => agent = it.next().cloned(),
            "--state" => state = it.next().and_then(|s| AgentState::parse(s)),
            "--session" => session = it.next().cloned(),
            "--ttl" => ttl = it.next().and_then(|t| t.parse::<u64>().ok()),
            "--stdin-json" => use_stdin = true,
            _ => {}
        }
    }
    let agent = agent.filter(|a| valid_agent(a))?;
    let state = state?;
    if use_stdin && session.is_none() {
        session = stdin().and_then(|b| session_from_hook_json(&b));
    }
    let session = session
        .filter(|s| valid_session(s))
        .unwrap_or_else(|| "default".into());
    let ttl = ttl.filter(|t| (1..=86_400).contains(t));
    Some(Event {
        v: 1,
        agent,
        session,
        state,
        ts: now_ms(),
        ttl,
    })
}

/// Reads the hook's JSON from stdin, at most 16 KB, giving up after `wait`.
fn read_stdin(wait: Duration) -> Option<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::stdin().take(STDIN_LIMIT).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(wait).ok()
}

fn event(args: &[String]) {
    let start = Instant::now();
    let Some(ev) = parse_event(args, || read_stdin(Duration::from_millis(400))) else {
        return;
    };
    // Cursor can import Claude's hooks; inside Cursor those would report a fake Claude session
    // (app-events §6b, D-045). Cursor's own hooks use --agent cursor.
    if ev.agent == "claude" && std::env::var_os("CURSOR_VERSION").is_some() {
        return;
    }
    let Ok(line) = serde_json::to_string(&ev) else {
        return;
    };
    let remaining = EVENT_BUDGET.saturating_sub(start.elapsed());
    // Run the send on a thread so a stuck pipe can never hold the agent past the budget.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let Some(mut conn) = connect(remaining) {
            let _ = conn.write_all(format!("{line}\n").as_bytes());
            let _ = conn.flush();
        }
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(EVENT_BUDGET.saturating_sub(start.elapsed()));
}

fn request(cmd: &str, args: Vec<String>) -> i32 {
    let req = Request {
        v: 1,
        cmd: cmd.into(),
        args,
    };
    let line = serde_json::to_string(&req).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let mut conn = connect(Duration::from_secs(2)).ok_or("app not running")?;
            conn.write_all(format!("{line}\n").as_bytes())
                .map_err(|_| "app not reachable")?;
            conn.flush().map_err(|_| "app not reachable")?;
            let mut reply = Vec::new();
            let mut byte = [0u8; 1];
            while conn.read(&mut byte).map_err(|_| "no reply")? == 1 && byte[0] != b'\n' {
                reply.push(byte[0]);
                if reply.len() > 1 << 20 {
                    break;
                }
            }
            Ok::<_, &str>(String::from_utf8_lossy(&reply).into_owned())
        })();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(reply)) => {
            let pretty = serde_json::from_str::<serde_json::Value>(&reply)
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or(reply.clone());
            println!("{pretty}");
            let failed = serde_json::from_str::<serde_json::Value>(&reply)
                .ok()
                .and_then(|v| v.get("ok").and_then(|o| o.as_bool()))
                == Some(false);
            i32::from(failed)
        }
        Ok(Err(e)) => {
            println!("{}", serde_json::json!({ "ok": false, "error": e }));
            1
        }
        Err(_) => {
            println!(
                "{}",
                serde_json::json!({ "ok": false, "error": "app did not answer" })
            );
            1
        }
    }
}

#[cfg(windows)]
fn connect(budget: Duration) -> Option<std::fs::File> {
    const ERROR_PIPE_BUSY: i32 = 231;
    let path = ipc_endpoint();
    let deadline = Instant::now() + budget;
    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(f) => return Some(f),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None, // not running
        }
    }
}

#[cfg(unix)]
fn connect(budget: Duration) -> Option<std::os::unix::net::UnixStream> {
    let s = std::os::unix::net::UnixStream::connect(ipc_endpoint()).ok()?;
    let _ = s.set_write_timeout(Some(budget));
    let _ = s.set_read_timeout(Some(Duration::from_secs(6)));
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn event_args() {
        let e = parse_event(
            &args("--agent claude --state needs --session abc --ttl 60"),
            || None,
        )
        .unwrap();
        assert_eq!(
            (e.agent.as_str(), e.state, e.session.as_str(), e.ttl),
            ("claude", AgentState::Needs, "abc", Some(60))
        );
    }

    #[test]
    fn stdin_json_session() {
        let e = parse_event(&args("--agent claude --state working --stdin-json"), || {
            Some(br#"{"session_id":"s-1","hook_event_name":"UserPromptSubmit"}"#.to_vec())
        })
        .unwrap();
        assert_eq!(e.session, "s-1");
    }

    #[test]
    fn falls_back_to_default_session() {
        let e = parse_event(&args("--agent codex --state done --stdin-json"), || {
            Some(b"nope".to_vec())
        })
        .unwrap();
        assert_eq!(e.session, "default");
    }

    #[test]
    fn invalid_is_ignored() {
        assert!(parse_event(&args("--agent Bad! --state needs"), || None).is_none());
        assert!(parse_event(&args("--agent claude --state nope"), || None).is_none());
        assert!(parse_event(&args("--state needs"), || None).is_none());
        let e = parse_event(&args("--agent a --state needs --ttl 0"), || None).unwrap();
        assert_eq!(e.ttl, None);
    }
}
