// SPDX-License-Identifier: FSL-1.1-ALv2
//! Cloud link against a local fake relay (relay-api §3, 1.1): a WebSocket server that checks the
//! hello, answers it and each `check` with `ok`, and forwards what is POSTed to `/v1/h/<K>` to the
//! sockets that said hello with `K`, like the real relay.

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
/// Claude Desktop's account (D-051: differs from the stale `~/.claude.json` one on PC2).
const DESKTOP_UUID: &str = "81c9fbe3-aaaa-bbbb-cccc-dddddddddddd";

type Bus = broadcast::Sender<(String, String)>;

#[derive(Default)]
struct Seen {
    hellos: Vec<Value>,
    other: Vec<Value>,
    /// The relay stops answering `check` (a broken link).
    silent: bool,
}

/// Fake relay: WS on one port, POST on another. `refuse` = answer every hello with that error.
async fn fake_relay(refuse: Option<&'static str>) -> (String, String, Arc<Mutex<Seen>>, Bus) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (bus, _) = broadcast::channel::<(String, String)>(64);
    let ws_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_url = format!("ws://{}/v1", ws_l.local_addr().unwrap());
    let (s2, b2) = (seen.clone(), bus.clone());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = ws_l.accept().await {
            let (seen, mut rx) = (s2.clone(), b2.subscribe());
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let mut my_k = String::new();
                loop {
                    tokio::select! {
                        m = ws.next() => {
                            let Some(Ok(WsMsg::Text(t))) = m else { return };
                            let v: Value = serde_json::from_str(&t).unwrap();
                            if v["t"] == "hello" {
                                my_k = v["k"].as_str().unwrap_or_default().to_string();
                                seen.lock().unwrap().hellos.push(v);
                                if let Some(e) = refuse {
                                    let _ = ws.send(WsMsg::text(json!({"t":"err","e":e}).to_string())).await;
                                    let _ = ws.close(None).await;
                                    return;
                                }
                                let silent = seen.lock().unwrap().silent;
                                if !silent {
                                    let _ = ws.send(WsMsg::text(r#"{"t":"ok"}"#)).await;
                                }
                            } else if v["t"] == "check" {
                                let silent = {
                                    let mut s = seen.lock().unwrap();
                                    s.other.push(v);
                                    s.silent
                                };
                                if !silent {
                                    let _ = ws.send(WsMsg::text(r#"{"t":"ok"}"#)).await;
                                }
                            } else {
                                seen.lock().unwrap().other.push(v);
                            }
                        }
                        Ok((k, out)) = rx.recv() => {
                            if k == my_k && ws.send(WsMsg::text(out)).await.is_err() { return; }
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
                            let k = text
                                .split_whitespace()
                                .nth(1)
                                .and_then(|p| p.strip_prefix("/v1/h/"))
                                .unwrap_or_default()
                                .to_string();
                            let _ = bus.send((k, json!({"t":"state","s":body["s"],"h":body["h"],"ts":body["ts"],"hb":body["hb"]}).to_string()));
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
    rig_with(name, ws_url, post_url, None).await
}

/// `desktop`: Claude Desktop's account, written to a Store-build style `config.json`.
async fn rig_with(name: &str, ws_url: String, post_url: String, desktop: Option<&str>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("Claude-Data/claude_pzs8sxrjxfjjc");
    std::fs::create_dir_all(&pkg).unwrap();
    if let Some(a) = desktop {
        std::fs::write(
            pkg.join("config.json"),
            json!({"lastKnownAccountUuid": a}).to_string(),
        )
        .unwrap();
    }
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
            account_files: vec![dir.path().join("Claude-Data/*/config.json"), claude_json],
            settings_file: dir.path().join("cloud.json"),
            creds: creds.clone(),
            // Checked often, so the tests see it work and fail.
            check_every: Duration::from_millis(400),
            ok_wait: Duration::from_millis(500),
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
    // Hello, answered with ok (relay-api 1.1).
    let c = wait_cloud(&r.engine, "working", 10).await;
    let k = cloud::route_key(UUID);
    assert_eq!(c["key"], k);
    let id = r.creds.0.lock().unwrap().clone().unwrap();
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.hellos[0], json!({"t":"hello","k":k,"i":id.i,"s":id.s}));
    }
    // An older app of the same account still POSTs the 1.0 self-check: not a session.
    bus.send((
        k.clone(),
        json!({"t":"state","s":"idle","h":cloud::probe_h(&k),"ts":1,"hb":true}).to_string(),
    ))
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(r.engine.status().await["sessions"]
        .as_array()
        .unwrap()
        .is_empty());
    // The link is checked, and stays working while the relay answers.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(seen
        .lock()
        .unwrap()
        .other
        .iter()
        .any(|m| m == &json!({"t":"check"})));
    assert_eq!(r.engine.status().await["cloud"]["state"], "working");

    // A cloud chat needs you, then finishes.
    bus.send((
        k.clone(),
        json!({"t":"state","s":"needs","h":"0123456789abcdef","ts":1,"hb":false}).to_string(),
    ))
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
    bus.send((
        k.clone(),
        json!({"t":"state","s":"done","h":"0123456789abcdef","ts":2,"hb":false}).to_string(),
    ))
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

/// D-051 (PC2): Desktop is signed in to another account than the stale `~/.claude.json`. The app
/// listens on both, with one identity; a Cowork chat of the Desktop account lights the puck.
#[tokio::test(flavor = "multi_thread")]
async fn cloud_listens_on_every_account() {
    let (ws, post, seen, bus) = fake_relay(None).await;
    let r = rig_with("multi", ws, post, Some(DESKTOP_UUID)).await;
    r.engine.cloud_set(true).await.unwrap();
    let c = wait_cloud(&r.engine, "working", 10).await;
    let (kd, kc) = (cloud::route_key(DESKTOP_UUID), cloud::route_key(UUID));
    assert_eq!(c["key"], kd, "Desktop's account first");
    let keys: Vec<_> = c["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(keys, [kd.clone(), kc.clone()]);
    // Both links are answered.
    let end = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let st = r.engine.status().await;
        if st["cloud"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .all(|k| k["state"] == "working")
        {
            break;
        }
        assert!(tokio::time::Instant::now() < end, "{}", st["cloud"]);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let id = r.creds.0.lock().unwrap().clone().unwrap();
    {
        let s = seen.lock().unwrap();
        let mut ks: Vec<_> = s.hellos.iter().map(|h| h["k"].as_str().unwrap()).collect();
        ks.sort();
        let mut want = [kd.as_str(), kc.as_str()];
        want.sort();
        assert_eq!(ks, want);
        assert!(s.hellos.iter().all(|h| h["i"] == id.i.as_str()));
    }
    bus.send((
        kd.clone(),
        json!({"t":"state","s":"needs","h":"c0ffee0000000001","ts":1,"hb":false}).to_string(),
    ))
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        r.sim.subscribe().wait_for(|s| s == "needs"),
    )
    .await
    .expect("timeout")
    .expect("needs shown");
    assert_eq!(
        r.engine.status().await["sessions"][0]["agent"],
        "claude-cloud"
    );

    // Off: unbinds itself from both keys.
    r.engine.cloud_set(false).await.unwrap();
    let other = seen.lock().unwrap().other.clone();
    let unbinds = other.iter().filter(|m| m["t"] == "unbind").count();
    assert_eq!(unbinds, 2, "{other:?}");
}

/// Only Claude Desktop is signed in (Cowork-only user, no `~/.claude.json` account): still works.
#[tokio::test(flavor = "multi_thread")]
async fn cloud_uses_desktop_account_alone() {
    let (ws, post, _seen, _bus) = fake_relay(None).await;
    let r = rig_with("desk", ws, post, Some(DESKTOP_UUID)).await;
    std::fs::write(r._dir.path().join(".claude.json"), "{}").unwrap();
    r.engine.cloud_set(true).await.unwrap();
    let c = wait_cloud(&r.engine, "working", 10).await;
    assert_eq!(c["key"], cloud::route_key(DESKTOP_UUID));
    assert_eq!(c["keys"].as_array().unwrap().len(), 1);
}

/// Turning the link on reads the accounts again: a Desktop sign-in switch made while it was off
/// is used at once (no restart).
#[tokio::test(flavor = "multi_thread")]
async fn cloud_turn_on_detects_account_switch() {
    let (ws, post, _seen, _bus) = fake_relay(None).await;
    let r = rig_with("switch", ws, post, Some(DESKTOP_UUID)).await;
    r.engine.cloud_set(true).await.unwrap();
    let c = wait_cloud(&r.engine, "working", 10).await;
    assert_eq!(c["key"], cloud::route_key(DESKTOP_UUID));
    r.engine.cloud_set(false).await.unwrap();
    // The user signs Claude Desktop in to another account.
    let other = "5e5e5e5e-1111-2222-3333-444444444444";
    std::fs::write(
        r._dir
            .path()
            .join("Claude-Data/claude_pzs8sxrjxfjjc/config.json"),
        json!({"lastKnownAccountUuid": other}).to_string(),
    )
    .unwrap();
    r.engine.cloud_set(true).await.unwrap();
    let c = wait_cloud(&r.engine, "working", 10).await;
    assert_eq!(
        c["key"],
        cloud::route_key(other),
        "new Desktop account first"
    );
    let keys: Vec<_> = c["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(keys, [cloud::route_key(other), cloud::route_key(UUID)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn cloud_reconnects_when_the_relay_stops_answering() {
    let (ws, post, seen, _bus) = fake_relay(None).await;
    let r = rig("silent", ws, post).await;
    r.engine.cloud_set(true).await.unwrap();
    wait_cloud(&r.engine, "working", 10).await;
    let hellos = seen.lock().unwrap().hellos.len();

    // No ok to the next check: the link says so and reconnects (after the usual backoff).
    seen.lock().unwrap().silent = true;
    let c = wait_cloud(&r.engine, "connecting", 5).await;
    assert_eq!(
        c["error"],
        "The cloud link stopped answering. Reconnecting…"
    );

    // The relay answers again: working once more, on a new connection.
    seen.lock().unwrap().silent = false;
    wait_cloud(&r.engine, "working", 15).await;
    assert!(seen.lock().unwrap().hellos.len() > hellos);
}
