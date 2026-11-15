//! Cloud link against a local fake relay (relay-api §3): a WebSocket server that checks the
//! hello and forwards what is POSTed to `/v1/h/<K>`, like the real relay.

use futures_util::{SinkExt, StreamExt};
use hn_engine::cloud::{self, CloudConfig, MemCreds};
use hn_engine::{Config, Engine, PuckTarget};
use hn_sim::Sim;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message as WsMsg;

const UUID: &str = "0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9";

#[derive(Default)]
struct Seen {
    hellos: Vec<Value>,
    other: Vec<Value>,
}

/// Fake relay: WS on one port, POST on another. `refuse` = answer every hello with that error.
async fn fake_relay(
    refuse: Option<&'static str>,
) -> (String, String, Arc<Mutex<Seen>>, broadcast::Sender<String>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (bus, _) = broadcast::channel::<String>(64);
    let ws_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_url = format!("ws://{}/v1", ws_l.local_addr().unwrap());
    let (s2, b2) = (seen.clone(), bus.clone());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = ws_l.accept().await {
            let (seen, mut rx) = (s2.clone(), b2.subscribe());
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                loop {
                    tokio::select! {
                        m = ws.next() => {
                            let Some(Ok(WsMsg::Text(t))) = m else { return };
                            let v: Value = serde_json::from_str(&t).unwrap();
                            if v["t"] == "hello" {
                                seen.lock().unwrap().hellos.push(v);
                                if let Some(e) = refuse {
                                    let _ = ws.send(WsMsg::text(json!({"t":"err","e":e}).to_string())).await;
                                    let _ = ws.close(None).await;
                                    return;
                                }
                            } else {
                                seen.lock().unwrap().other.push(v);
                            }
                        }
                        Ok(out) = rx.recv() => {
                            if ws.send(WsMsg::text(out)).await.is_err() { return; }
                        }
                    }
                }
            });
        }
    });
    // POST /v1/h/<K>: forward as a state message (no K check needed here).
    let post_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let post_url = format!("http://{}", post_l.local_addr().unwrap());
    let b3 = bus.clone();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = post_l.accept().await {
            let bus = b3.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let mut got = Vec::new();
                loop {
                    let n = tcp.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len: usize = text
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if got.len() >= i + 4 + len {
                            let body: Value =
                                serde_json::from_slice(&got[i + 4..i + 4 + len]).unwrap();
                            let _ = bus.send(json!({"t":"state","s":body["s"],"h":body["h"],"ts":body["ts"],"hb":body["hb"]}).to_string());
                            break;
                        }
                    }
                }
                let _ = tcp
                    .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                    .await;
            });
        }
    });
    (ws_url, post_url, seen, bus)
}

struct Rig {
    engine: Engine,
    sim: Sim,
    creds: Arc<MemCreds>,
    _dir: tempfile::TempDir,
}

async fn rig(name: &str, ws_url: String, post_url: String) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let claude_json = dir.path().join(".claude.json");
    std::fs::write(
        &claude_json,
        json!({"oauthAccount": {"accountUuid": UUID}}).to_string(),
    )
    .unwrap();
    let creds = Arc::new(MemCreds::default());
    let sim = Sim::new("2.0.0", None);
    let engine = Engine::start(Config {
        puck: PuckTarget::Sim(sim.clone()),
        endpoint: if cfg!(windows) {
            format!(r"\\.\pipe\hn-cloud-{name}-{}", std::process::id()).into()
        } else {
            std::path::PathBuf::from("/tmp")
                .join(format!("hn-cloud-{name}-{}", std::process::id()))
                .join("hn.sock")
        },
        claude_dir: None,
        latest_url: String::new(),
        uf2_drive: None,
        http_port: None,
        agents_file: None,
        cloud: Some(CloudConfig {
            ws_url,
            post_url,
            claude_json: Some(claude_json),
            settings_file: dir.path().join("cloud.json"),
            creds: creds.clone(),
        }),
    })
    .await
    .unwrap();
    Rig {
        engine,
        sim,
        creds,
        _dir: dir,
    }
}

async fn wait_cloud(e: &Engine, want: &str, secs: u64) -> Value {
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let st = e.status().await;
        if st["cloud"]["state"] == want {
            return st["cloud"].clone();
        }
        assert!(
            tokio::time::Instant::now() < end,
            "cloud never {want}: {}",
            st["cloud"]
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cloud_states_light_the_puck() {
    let (ws, post, seen, bus) = fake_relay(None).await;
    let r = rig("ok", ws, post).await;
    // Off by default (D-030).
    assert_eq!(r.engine.status().await["cloud"]["state"], "off");
    assert!(seen.lock().unwrap().hellos.is_empty());

    r.engine.cloud_set(true).await.unwrap();
    // Hello, then the self-check probe comes back through POST → relay → socket.
    let c = wait_cloud(&r.engine, "working", 10).await;
    let k = cloud::route_key(UUID);
    assert_eq!(c["key"], k);
    let id = r.creds.0.lock().unwrap().clone().unwrap();
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.hellos[0], json!({"t":"hello","k":k,"i":id.i,"s":id.s}));
    }
    // The probe isn't a session.
    assert!(r.engine.status().await["sessions"]
        .as_array()
        .unwrap()
        .is_empty());

    // A cloud chat needs you, then finishes.
    bus.send(json!({"t":"state","s":"needs","h":"0123456789abcdef","ts":1,"hb":false}).to_string())
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        r.sim.subscribe().wait_for(|s| s == "needs"),
    )
    .await
    .expect("timeout")
    .expect("needs shown");
    let st = r.engine.status().await;
    let s = &st["sessions"][0];
    assert_eq!(s["agent"], "claude-cloud");
    assert_eq!(s["session"], "0123456789abcdef");
    assert_eq!(s["ttl"], 120);
    bus.send(json!({"t":"state","s":"done","h":"0123456789abcdef","ts":2,"hb":false}).to_string())
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        r.sim.subscribe().wait_for(|s| s == "done"),
    )
    .await
    .expect("timeout")
    .expect("done flash");

    // Off: unbinds itself and forgets its secret.
    r.engine.cloud_set(false).await.unwrap();
    assert!(r.creds.0.lock().unwrap().is_none());
    let other = seen.lock().unwrap().other.clone();
    assert!(
        other.iter().any(|m| m == &json!({"t":"unbind","i":id.i})),
        "{other:?}"
    );
    assert_eq!(r.engine.status().await["cloud"]["state"], "off");
}

#[tokio::test(flavor = "multi_thread")]
async fn cloud_key_full_is_reported() {
    let (ws, post, _seen, _bus) = fake_relay(Some("key_full")).await;
    let r = rig("full", ws, post).await;
    r.engine.cloud_set(true).await.unwrap();
    let c = wait_cloud(&r.engine, "key_full", 10).await;
    assert!(
        c["error"]
            .as_str()
            .unwrap()
            .contains("Three other computers"),
        "{c}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cloud_without_account_says_so() {
    let (ws, post, seen, _bus) = fake_relay(None).await;
    let r = rig("noacct", ws, post).await;
    std::fs::write(r._dir.path().join(".claude.json"), "{}").unwrap();
    r.engine.cloud_set(true).await.unwrap();
    wait_cloud(&r.engine, "no_account", 5).await;
    assert!(seen.lock().unwrap().hellos.is_empty());
}
