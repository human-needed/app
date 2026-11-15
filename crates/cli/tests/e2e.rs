//! End to end on this machine: the real `hn` binary → IPC → engine → puck simulator.

use hn_engine::{Config, Engine, PuckTarget};
use hn_sim::Sim;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const HN: &str = env!("CARGO_BIN_EXE_hn");

fn endpoint(name: &str) -> PathBuf {
    let unique = format!("hn-test-{name}-{}-{}", std::process::id(), hn_sim::now_ms());
    if cfg!(windows) {
        PathBuf::from(format!(r"\\.\pipe\{unique}"))
    } else {
        // Short path: Unix socket paths max out at 104 bytes on macOS ($TMPDIR is long there).
        PathBuf::from("/tmp").join(unique).join("hn.sock")
    }
}

struct Rig {
    sim: Sim,
    engine: Engine,
    ipc: PathBuf,
    claude: tempfile::TempDir,
}

async fn rig(name: &str) -> Rig {
    rig_with(name, None, None).await
}

async fn rig_with(name: &str, latest: Option<String>, drive: Option<PathBuf>) -> Rig {
    rig_full(name, latest, drive, None).await
}

async fn rig_full(
    name: &str,
    latest: Option<String>,
    drive: Option<PathBuf>,
    http_port: Option<u16>,
) -> Rig {
    let sim = Sim::new("2.0.0", drive.clone());
    let claude = tempfile::tempdir().unwrap();
    std::fs::create_dir(claude.path().join("sessions")).unwrap();
    let ipc = endpoint(name);
    let engine = Engine::start(Config {
        puck: PuckTarget::Sim(sim.clone()),
        endpoint: ipc.clone(),
        claude_dir: Some(claude.path().to_path_buf()),
        latest_url: latest.unwrap_or_else(|| "file:///nowhere".into()),
        uf2_drive: drive,
        http_port,
        agents_file: None,
    })
    .await
    .unwrap();
    let rig = Rig {
        sim,
        engine,
        ipc,
        claude,
    };
    rig.wait_puck().await;
    rig
}

impl Rig {
    async fn wait_puck(&self) {
        for _ in 0..100 {
            if self.engine.puck_info().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("puck never connected");
    }

    /// Runs `hn event …` exactly like a hook would, with the hook JSON on stdin.
    fn hook(&self, state: &str, session: &str) -> (std::process::Output, Duration) {
        let start = Instant::now();
        let mut child = Command::new(HN)
            .args([
                "event",
                "--agent",
                "claude",
                "--state",
                state,
                "--stdin-json",
            ])
            .env("HN_IPC", &self.ipc)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        let json = format!(r#"{{"session_id":"{session}","hook_event_name":"x"}}"#);
        child
            .stdin
            .take()
            .unwrap()
            .write_all(json.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (out, start.elapsed())
    }

    fn hn(&self, args: &[&str]) -> serde_json::Value {
        let out = Command::new(HN)
            .args(args)
            .env("HN_IPC", &self.ipc)
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stdout)))
    }

    async fn expect_shown(&self, want: &str, within: Duration) {
        let mut rx = self.sim.subscribe();
        let r = tokio::time::timeout(within, rx.wait_for(|s| s == want)).await;
        assert!(
            r.is_ok(),
            "expected puck to show {want}, it shows {}",
            self.sim.shown()
        );
    }

    fn claude_file(&self, session: &str, status: &str) {
        let me = std::process::id();
        let text = format!(
            r#"{{"pid":{me},"sessionId":"{session}","status":"{status}","statusUpdatedAt":{}}}"#,
            hn_sim::now_ms()
        );
        std::fs::write(self.claude.path().join(format!("sessions/{me}.json")), text).unwrap();
    }
}

#[test]
fn hn_is_silent_and_fast_without_the_app() {
    let start = Instant::now();
    let out = Command::new(HN)
        .args([
            "event",
            "--agent",
            "claude",
            "--state",
            "working",
            "--stdin-json",
        ])
        .env("HN_IPC", endpoint("absent"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    // Garbage args: still exit 0, silent.
    let out = Command::new(HN)
        .args(["event", "--agent", "BAD"])
        .env("HN_IPC", endpoint("absent"))
        .output()
        .unwrap();
    assert!(out.status.success() && out.stdout.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn working_needs_done_flash() {
    let r = rig("states").await;
    r.expect_shown("idle", Duration::from_secs(2)).await;

    let (out, took) = r.hook("working", "s1");
    assert!(out.status.success() && out.stdout.is_empty(), "{out:?}");
    assert!(took < Duration::from_secs(1));
    r.expect_shown("working", Duration::from_secs(2)).await;

    r.hook("needs", "s2");
    r.expect_shown("needs", Duration::from_secs(2)).await;
    assert!(r
        .sim
        .puck
        .lock()
        .unwrap()
        .sounds
        .iter()
        .any(|s| s.starts_with("needs")));

    r.hook("end", "s2");
    r.expect_shown("working", Duration::from_secs(2)).await;

    r.hook("done", "s1");
    r.expect_shown("done", Duration::from_secs(2)).await;
    // done.duration is 5 s in the simulator's defaults; then nothing is active.
    r.expect_shown("idle", Duration::from_secs(7)).await;
    assert!(r.engine.status().await["snapshot"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_button_clears_within_10s() {
    let r = rig("stop").await;
    r.claude_file("c1", "busy");
    r.hook("working", "c1");
    r.expect_shown("working", Duration::from_secs(2)).await;
    // Busy keeps it lit.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(r.sim.shown(), "working");
    // Stop: Claude flips the file to idle, no hook runs.
    let t = Instant::now();
    r.claude_file("c1", "idle");
    r.expect_shown("idle", Duration::from_secs(10)).await;
    eprintln!("stop cleared after {:?}", t.elapsed());
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_quit_clears_within_10s() {
    let r = rig("quit").await;
    r.claude_file("q1", "busy");
    r.hook("needs", "q1");
    r.expect_shown("needs", Duration::from_secs(2)).await;
    tokio::time::sleep(Duration::from_secs(3)).await; // bind to the file
    std::fs::remove_file(
        r.claude
            .path()
            .join(format!("sessions/{}.json", std::process::id())),
    )
    .unwrap();
    r.expect_shown("idle", Duration::from_secs(10)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn out_of_order_events_are_ignored() {
    let r = rig("order").await;
    let send = |state: &str, ts: u64| {
        let line =
            format!(r#"{{"v":1,"agent":"codex","session":"o","state":"{state}","ts":{ts}}}"#);
        send_raw(&r.ipc, &line);
    };
    send("done", 2000);
    send("working", 1000); // late
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(r.engine.status().await["snapshot"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn puck_reset_gets_state_again() {
    let r = rig("rehello").await;
    r.hook("working", "r1");
    r.expect_shown("working", Duration::from_secs(2)).await;
    assert_eq!(r.engine.puck("REBOOT").await.unwrap(), "OK");
    r.expect_shown("bootloader", Duration::from_secs(2)).await;
    r.expect_shown("working", Duration::from_secs(6)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hn_commands() {
    let r = rig("cmds").await;
    r.hook("working", "x1");
    r.expect_shown("working", Duration::from_secs(2)).await;
    let st = tokio::task::spawn_blocking({
        let ipc = r.ipc.clone();
        move || {
            let out = Command::new(HN)
                .arg("status")
                .env("HN_IPC", &ipc)
                .output()
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
        }
    })
    .await
    .unwrap();
    assert_eq!(st["puck"]["connected"], true);
    assert_eq!(st["puck"]["shown"], "working");
    assert_eq!(st["sessions"][0]["session"], "x1");

    let ipc = r.ipc.clone();
    let rr = Rig {
        sim: r.sim.clone(),
        engine: r.engine.clone(),
        ipc,
        claude: tempfile::tempdir().unwrap(),
    };
    let (mute, set, bad, test) = tokio::task::spawn_blocking(move || {
        (
            rr.hn(&["mute", "5"]),
            rr.hn(&["set", "needs.color", "#112233"]),
            rr.hn(&["set", "nope", "1"]),
            rr.hn(&["test", "needs"]),
        )
    })
    .await
    .unwrap();
    assert_eq!(mute["ok"], true);
    assert_eq!(set["ok"], true);
    assert_eq!(bad["ok"], false);
    assert_eq!(test["ok"], true);
    assert_eq!(
        r.sim.puck.lock().unwrap().settings["needs.color"],
        "#112233"
    );
    r.expect_shown("test:needs", Duration::from_secs(2)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn second_instance_is_refused() {
    let r = rig("single").await;
    let again = Engine::start(Config {
        puck: PuckTarget::None,
        endpoint: r.ipc.clone(),
        claude_dir: None,
        latest_url: String::new(),
        uf2_drive: None,
        http_port: None,
        agents_file: None,
    })
    .await;
    assert!(again.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn firmware_update_via_bootsel() {
    let dir = tempfile::tempdir().unwrap();
    let drive = dir.path().join("RPI-RP2");
    let mut uf2 = vec![0x55, 0x46, 0x32, 0x0A, 0x57, 0x51, 0x5D, 0x9E];
    uf2.resize(1024, 7);
    let file = dir.path().join("humanneeded-hn-v2-2.0.1.uf2");
    std::fs::write(&file, &uf2).unwrap();
    let sha: String = Sha256::digest(&uf2)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let latest = dir.path().join("latest.json");
    let url = |p: &Path| {
        format!(
            "file:///{}",
            p.display()
                .to_string()
                .replace('\\', "/")
                .trim_start_matches('/')
        )
    };
    std::fs::write(
        &latest,
        format!(
            r#"{{"hn-v2":{{"version":"2.0.1","file":"humanneeded-hn-v2-2.0.1.uf2","url":"{}","sha256":"{sha}"}}}}"#,
            url(&file)
        ),
    )
    .unwrap();
    let r = rig_with("fw", Some(url(&latest)), Some(drive)).await;
    let avail = r.engine.check_update().await.unwrap();
    assert!(avail.newer);
    r.hook("working", "f1");
    r.expect_shown("working", Duration::from_secs(2)).await;
    let v = r
        .engine
        .update_firmware(|s| eprintln!("update: {s}"))
        .await
        .unwrap();
    assert_eq!(v, "2.0.1");
    assert_eq!(r.sim.fw(), "2.0.1");
    // State comes back after the update.
    r.expect_shown("working", Duration::from_secs(5)).await;
    assert!(!r.engine.check_update().await.unwrap().newer);
}

#[tokio::test(flavor = "multi_thread")]
async fn firmware_without_sha_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let latest = dir.path().join("latest.json");
    let mut uf2 = vec![0x55, 0x46, 0x32, 0x0A, 0x57, 0x51, 0x5D, 0x9E];
    uf2.resize(1024, 7);
    let file = dir.path().join("a-2.0.1.uf2");
    std::fs::write(&file, &uf2).unwrap();
    let p = |p: &Path| {
        format!(
            "file:///{}",
            p.display()
                .to_string()
                .replace('\\', "/")
                .trim_start_matches('/')
        )
    };
    std::fs::write(
        &latest,
        format!(
            r#"{{"hn-v2":{{"version":"2.0.1","file":"a-2.0.1.uf2","url":"{}"}}}}"#,
            p(&file)
        ),
    )
    .unwrap();
    let r = rig_with("fwsha", Some(p(&latest)), None).await;
    let err = r.engine.update_firmware(|_| {}).await.unwrap_err();
    assert!(err.contains("SHA-256"), "{err}");
    assert_eq!(r.sim.fw(), "2.0.0");
}

fn send_raw(ipc: &Path, line: &str) {
    use std::io::Write;
    #[cfg(windows)]
    let mut f = {
        let mut tries = 0;
        loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(ipc) {
                Ok(f) => break f,
                Err(e) if e.raw_os_error() == Some(231) && tries < 100 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("{e}"),
            }
        }
    };
    #[cfg(unix)]
    let mut f = std::os::unix::net::UnixStream::connect(ipc).unwrap();
    f.write_all(format!("{line}\n").as_bytes()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_events_inside_cursor_are_dropped() {
    let r = rig("cursor").await;
    for (agent, env) in [("claude", Some("1.7.0")), ("cursor", Some("1.7.0"))] {
        let mut cmd = Command::new(HN);
        cmd.args([
            "event",
            "--agent",
            agent,
            "--state",
            "working",
            "--session",
            agent,
        ])
        .env("HN_IPC", &r.ipc);
        if let Some(v) = env {
            cmd.env("CURSOR_VERSION", v);
        }
        assert!(cmd.output().unwrap().status.success());
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let st = r.engine.status().await;
    let agents: Vec<&str> = st["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["agent"].as_str().unwrap())
        .collect();
    assert_eq!(agents, vec!["cursor"]);
}

// ---- D-047: local HTTP receiver ----

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Sends a raw HTTP request, returns the status code.
fn http(port: u16, raw: &str) -> u16 {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(raw.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

fn post(port: u16, path: &str, body: &str, extra: &str) -> u16 {
    http(
        port,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n{body}",
            body.len()
        ),
    )
}

async fn http_rig(name: &str) -> (Rig, u16) {
    let port = free_port();
    let r = rig_full(name, None, None, Some(port)).await;
    for _ in 0..50 {
        if r.engine.status().await["http"]["listening"] == true {
            return (r, port);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("http never listened");
}

async fn sessions(r: &Rig) -> Vec<String> {
    let st = r.engine.status().await;
    st["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["session"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn http_events_drive_the_puck() {
    let (r, port) = http_rig("http").await;
    let hook = r#"{"session_id":"h1","hook_event_name":"UserPromptSubmit","prompt":"hi"}"#;
    let t = Instant::now();
    assert_eq!(
        tokio::task::spawn_blocking(move || post(port, "/v1/event/claude/working", hook, ""))
            .await
            .unwrap(),
        204
    );
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "{:?}",
        t.elapsed()
    );
    r.expect_shown("working", Duration::from_secs(2)).await;
    let needs = r#"{"session_id":"h1"}"#;
    assert_eq!(
        tokio::task::spawn_blocking(move || post(port, "/v1/event/claude/needs", needs, ""))
            .await
            .unwrap(),
        204
    );
    r.expect_shown("needs", Duration::from_secs(2)).await;
    // Chunked body (some HTTP clients send one).
    let chunked = format!(
        "POST /v1/event/claude/done HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n13\r\n{{\"session_id\":\"h1\"}}\r\n0\r\n\r\n"
    );
    assert_eq!(
        tokio::task::spawn_blocking(move || http(port, &chunked))
            .await
            .unwrap(),
        204
    );
    r.expect_shown("done", Duration::from_secs(2)).await;
    assert_eq!(sessions(&r).await, vec!["h1"]);
    // D-054: Claude now shows as connected, the others as not set up.
    let st = r.engine.status().await;
    let states: Vec<_> = st["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            format!(
                "{}={}",
                a["id"].as_str().unwrap(),
                a["state"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        states,
        ["claude=connected", "codex=not_set_up", "cursor=not_set_up"]
    );
    assert_eq!(st["setup_url"], "https://human-needed.com/start/");
    let health = format!("GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    assert_eq!(
        tokio::task::spawn_blocking(move || http(port, &health))
            .await
            .unwrap(),
        200
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn http_rejects_browsers_and_junk() {
    let (r, port) = http_rig("httpbad").await;
    let codes = tokio::task::spawn_blocking(move || {
        let b = r#"{"session_id":"x"}"#;
        vec![
            post(port, "/v1/event/claude/working", b, "Origin: https://evil.example\r\n"),
            http(port, &format!("POST /v1/event/claude/working HTTP/1.1\r\nHost: evil.example:{port}\r\nContent-Length: 2\r\n\r\n{{}}")),
            http(port, &format!("POST /v1/event/claude/working HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: 40000000\r\n\r\n")),
            http(port, &format!("POST /v1/event/claude/working HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\r\n")),
            post(port, "/v1/event/claude/busy", b, ""),
            post(port, "/v1/event/Bad!/working", b, ""),
            http(port, &format!("GET /v1/event/claude/working HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")),
            post(port, "/elsewhere", b, ""),
            // POST without a JSON content type (app-events 2.0 §1): 405.
            http(port, &format!("POST /v1/event/claude/working HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 2\r\n\r\n{{}}")),
            http(port, &format!("POST /v1/event/claude/working HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{{}}")),
            http(port, &format!("POST /v1/clear HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 0\r\n\r\n")),
            http(port, &format!("POST /v1/event/claude/working HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: Application/JSON; charset=utf-8\r\nContent-Length: 2\r\n\r\n{{}}")),
        ]
    })
    .await
    .unwrap();
    assert_eq!(
        codes,
        vec![403, 403, 413, 411, 400, 400, 405, 404, 405, 405, 405, 204]
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(sessions(&r).await, vec!["default"]); // only the last, valid one
    r.engine.request("clear", &[]).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Not JSON at all: accepted as session "default".
    assert_eq!(
        tokio::task::spawn_blocking(move || post(port, "/v1/event/codex/working", "garbage", ""))
            .await
            .unwrap(),
        204
    );
    r.expect_shown("working", Duration::from_secs(2)).await;
    // codex/default (newest) plus the cleared claude/default.
    assert_eq!(sessions(&r).await, vec!["default", "default"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_port_in_use_is_reported() {
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let r = rig_full("httpbusy", None, None, Some(port)).await;
    let mut err = serde_json::Value::Null;
    for _ in 0..50 {
        err = r.engine.status().await["http"]["error"].clone();
        if !err.is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(err.as_str().unwrap_or("").contains("already used"), "{err}");
    assert_eq!(r.engine.status().await["http"]["listening"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_big_post_tool_use_body_is_accepted() {
    let (r, port) = http_rig("httpbig").await;
    // A PostToolUse with ~8 MB of tool output: read, mostly discarded, 204, session from the start.
    let big = format!(
        r#"{{"session_id":"big1","hook_event_name":"PostToolUse","tool_response":"{}"}}"#,
        "x".repeat(8 * 1024 * 1024)
    );
    let t = Instant::now();
    let code =
        tokio::task::spawn_blocking(move || post(port, "/v1/event/claude/working", &big, ""))
            .await
            .unwrap();
    assert_eq!(code, 204);
    eprintln!("8 MB body answered in {:?}", t.elapsed());
    r.expect_shown("working", Duration::from_secs(2)).await;
    assert_eq!(sessions(&r).await, vec!["big1"]);
}

/// Sends a request and returns (status, body).
fn http_full(port: u16, raw: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(raw.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let code = out
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (code, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn http_skill_endpoints() {
    let (r, port) = http_rig("httpskill").await;
    let req = move |method: &str, path: &str, body: &str| {
        http_full(
            port,
            &format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
    };
    let out = tokio::task::spawn_blocking(move || {
        vec![
            req("GET", "/v1/status", ""),
            req(
                "POST",
                "/v1/set",
                r##"{"key":"needs.color","value":"#9B5BFF"}"##,
            ),
            req("POST", "/v1/set", r#"{"key":"done.duration","value":10}"#),
            req("POST", "/v1/set", r#"{"key":"nope","value":1}"#),
            req("POST", "/v1/set", "not json"),
            req("POST", "/v1/mute/60", ""),
            req("POST", "/v1/test/needs", ""),
            req("POST", "/v1/clear", ""),
            req("GET", "/v1/test/needs", ""),
        ]
    })
    .await
    .unwrap();
    let json = |i: usize| serde_json::from_str::<serde_json::Value>(&out[i].1).unwrap();
    assert_eq!(out[0].0, 200);
    assert_eq!(json(0)["puck"]["connected"], true);
    assert_eq!(json(1)["ok"], true);
    assert_eq!(json(2)["ok"], true);
    assert_eq!(json(3)["ok"], false);
    assert_eq!(out[4].0, 400);
    assert_eq!(json(5)["ok"], true);
    assert_eq!(json(6)["ok"], true);
    assert_eq!(json(7)["ok"], true);
    assert_eq!(out[8].0, 405);
    let p = r.sim.puck.lock().unwrap();
    assert_eq!(p.settings["needs.color"], "#9b5bff");
    assert_eq!(p.settings["done.duration"], "10");
}

/// D-048: state-changing skill endpoints are POST-only and behind the Host/Origin guards.
#[tokio::test(flavor = "multi_thread")]
async fn http_skill_endpoints_are_guarded() {
    let (r, port) = http_rig("httpguard").await;
    let codes = tokio::task::spawn_blocking(move || {
        let mut codes = Vec::new();
        for path in ["/v1/test/needs", "/v1/mute/60", "/v1/clear", "/v1/set"] {
            let body = r##"{"key":"needs.color","value":"#000000"}"##;
            let send = |method: &str, extra: &str, host: &str| {
                http_full(
                    port,
                    &format!(
                        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\n{extra}\r\n{body}",
                        body.len()
                    ),
                )
                .0
            };
            let host = format!("127.0.0.1:{port}");
            codes.push((path, "GET", send("GET", "", &host)));
            codes.push((path, "PUT", send("PUT", "", &host)));
            codes.push((path, "Origin", send("POST", "Origin: https://evil.example\r\n", &host)));
            codes.push((path, "Host", send("POST", "", &format!("evil.example:{port}"))));
        }
        codes
    })
    .await
    .unwrap();
    for (path, what, code) in &codes {
        let want = if *what == "GET" || *what == "PUT" {
            405
        } else {
            403
        };
        assert_eq!(*code, want, "{what} {path}");
    }
    // Nothing got through.
    let p = r.sim.puck.lock().unwrap();
    assert_ne!(p.settings["needs.color"], "#000000");
    assert!(p.sounds.is_empty());
}

/// D-045 over HTTP: Cursor running imported Claude hooks sends `cursor_version`; dropped.
#[tokio::test(flavor = "multi_thread")]
async fn http_claude_events_from_cursor_are_dropped() {
    let (r, port) = http_rig("httpcursor").await;
    let codes = tokio::task::spawn_blocking(move || {
        vec![
            post(
                port,
                "/v1/event/claude/working",
                r#"{"conversation_id":"cur1","cursor_version":"1.7.0","hook_event_name":"beforeSubmitPrompt"}"#,
                "",
            ),
            // Cursor's own integration is fine.
            post(
                port,
                "/v1/event/cursor/working",
                r#"{"conversation_id":"cur2","cursor_version":"1.7.0"}"#,
                "",
            ),
        ]
    })
    .await
    .unwrap();
    assert_eq!(codes, vec![204, 204]);
    r.expect_shown("working", Duration::from_secs(2)).await;
    assert_eq!(sessions(&r).await, vec!["cur2"]);
}

#[cfg(windows)]
fn session_field(st: &serde_json::Value, session: &str, field: &str) -> serde_json::Value {
    st["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["session"] == session)
        .map(|s| s[field].clone())
        .unwrap_or_default()
}

/// app-events 2.0 §2: a non-Claude session lives as long as the agent process that sent it.
/// The hub finds it from the connection (Windows): here one agent is this test process itself
/// (`e2e-….exe`, agent id `e2e`), the other a copy of curl named `codex.exe` that exits at once.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn http_agent_process_liveness() {
    let curl = std::path::Path::new(r"C:\Windows\System32\curl.exe");
    if !curl.is_file() {
        eprintln!("no curl.exe; skipped");
        return;
    }
    let (r, port) = http_rig("httpproc").await;
    let code = tokio::task::spawn_blocking(move || {
        post(
            port,
            "/v1/event/e2e/working",
            r#"{"session_id":"alive"}"#,
            "",
        )
    })
    .await
    .unwrap();
    assert_eq!(code, 204);
    let dir = tempfile::tempdir().unwrap();
    let codex = dir.path().join("codex.exe");
    std::fs::copy(curl, &codex).unwrap();
    let out = std::process::Command::new(&codex)
        .args([
            "-s",
            "--noproxy",
            "*",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            r#"{"session_id":"gone"}"#,
            &format!("http://127.0.0.1:{port}/v1/event/codex/working"),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let st = r.engine.status().await;
    assert_eq!(
        session_field(&st, "alive", "proc")["pid"],
        std::process::id()
    );
    assert!(session_field(&st, "gone", "proc")["pid"].is_u64(), "{st}");
    // Liveness runs every 2 s: the exited one goes idle, the live one stays.
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let st = r.engine.status().await;
        if session_field(&st, "gone", "state") == "idle" {
            assert_eq!(session_field(&st, "gone", "reason"), "codex process gone");
            assert_eq!(session_field(&st, "alive", "state"), "working");
            break;
        }
        assert!(Instant::now() < deadline, "{st}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A hook body with a UTF-8 BOM (Windows re-encoding) still yields its session.
#[tokio::test(flavor = "multi_thread")]
async fn http_body_with_bom() {
    let (r, port) = http_rig("httpbom").await;
    let code = tokio::task::spawn_blocking(move || {
        post(
            port,
            "/v1/event/codex/working",
            "\u{feff}{\"session_id\":\"bom1\"}",
            "",
        )
    })
    .await
    .unwrap();
    assert_eq!(code, 204);
    r.expect_shown("working", Duration::from_secs(2)).await;
    assert_eq!(sessions(&r).await, vec!["bom1"]);
}
