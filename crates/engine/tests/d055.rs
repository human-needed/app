// SPDX-License-Identifier: GPL-3.0-or-later
//! D-055 engine features against the simulated puck: per-state levels held at 100, per-agent
//! switches, quiet hours.

use hn_engine::{quiet, Config, Engine, PuckTarget};
use hn_sim::Sim;
use serde_json::Value;
use std::io::{Read, Write};
use std::time::Duration;

struct Rig {
    engine: Engine,
    sim: Sim,
    port: u16,
    _dir: tempfile::TempDir,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn rig(name: &str) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let sim = Sim::new("2.0.0", None);
    let port = free_port();
    let engine = Engine::start(Config {
        puck: PuckTarget::Sim(sim.clone()),
        endpoint: if cfg!(windows) {
            format!(r"\\.\pipe\hn-d055-{name}-{}", std::process::id()).into()
        } else {
            std::path::PathBuf::from("/tmp")
                .join(format!("hn-d055-{name}-{}", std::process::id()))
                .join("hn.sock")
        },
        claude_dir: None,
        latest_url: String::new(),
        uf2_drive: None,
        http_port: Some(port),
        agents_file: Some(dir.path().join("agents.json")),
        cloud: None,
    })
    .await
    .unwrap();
    for _ in 0..100 {
        if engine.puck_info().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Rig {
        engine,
        sim,
        port,
        _dir: dir,
    }
}

async fn post(port: u16, path: &'static str) -> u16 {
    tokio::task::spawn_blocking(move || {
        let body = r#"{"session_id":"s1"}"#;
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut r = String::new();
        let _ = s.read_to_string(&mut r);
        r.split_whitespace().nth(1).unwrap_or("0").parse().unwrap_or(0)
    })
    .await
    .unwrap()
}

async fn get(e: &Engine, key: &str) -> String {
    let r: Value = serde_json::from_str(&e.puck(format!("GET {key}")).await.unwrap()).unwrap();
    match &r[key] {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

async fn wait<F: Fn() -> bool>(what: &str, f: F) {
    for _ in 0..60 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never: {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_state_levels_are_held_at_full() {
    let r = rig("levels").await;
    // The sim's idle glow starts at 10% of the master level.
    let mut ok = false;
    for _ in 0..40 {
        if get(&r.engine, "idle.brightness").await == "100" {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "idle.brightness never set to 100");
    for k in ["needs.volume", "done.brightness", "working.brightness"] {
        assert_eq!(get(&r.engine, k).await, "100", "{k}");
    }
    // The master levels are the user's and stay as they were.
    assert_eq!(get(&r.engine, "brightness").await, "60");
}

#[tokio::test(flavor = "multi_thread")]
async fn switched_off_agent_does_not_light_the_puck() {
    let r = rig("switch").await;
    r.engine.agent_set("codex", false).unwrap();
    assert!(r.engine.agent_set("nobody", false).is_err());
    assert_eq!(post(r.port, "/v1/event/codex/needs").await, 204);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let st = r.engine.status().await;
    assert!(st["snapshot"].as_array().unwrap().is_empty(), "{st}");
    let codex = st["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "codex")
        .unwrap()
        .clone();
    assert_eq!(codex["enabled"], false);
    assert_eq!(codex["state"], "connected", "still seen");

    // Claude lights; switching it off clears its session at once.
    assert_eq!(post(r.port, "/v1/event/claude/needs").await, 204);
    let sim = r.sim.clone();
    wait("needs", || sim.shown() == "needs").await;
    r.engine.agent_set("claude", false).unwrap();
    wait("cleared", || sim.shown() != "needs").await;
    r.engine.agent_set("codex", true).unwrap();
    assert_eq!(post(r.port, "/v1/event/codex/working").await, 204);
    wait("working", || sim.shown() == "working").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn quiet_hours_dim_mute_and_restore() {
    let r = rig("quiet").await;
    let now = quiet::local_minutes();
    let hm = |m: u32| format!("{:02}:{:02}", (m % 1440) / 60, m % 60);
    // A window around now.
    r.engine
        .quiet_set(quiet::Settings {
            enabled: true,
            from: hm(now + 1440 - 5),
            to: hm(now + 30),
        })
        .await
        .unwrap();
    assert_eq!(get(&r.engine, "brightness").await, "15");
    let st = r.engine.status().await;
    assert_eq!(st["quiet"]["active"], true);
    let muted = st["puck"]["status"]["muted"].as_str().unwrap().to_string();
    assert!(muted != "off" && muted != "on", "timed mute, got {muted}");

    // Off again: level back, timed mute lifted.
    let mut q = r.engine.quiet_get();
    q.enabled = false;
    r.engine.quiet_set(q).await.unwrap();
    assert_eq!(get(&r.engine, "brightness").await, "60");
    let st = r.engine.status().await;
    assert_eq!(st["quiet"]["active"], false);
    assert_eq!(st["puck"]["status"]["muted"], "off");
}
