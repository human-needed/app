// SPDX-License-Identifier: GPL-3.0-or-later
//! D-059: with no puck, "this computer" shows the state (glow) and plays the sounds; a puck
//! that connects takes over and its look is copied.

use hn_engine::{Config, Engine, PuckTarget};
use hn_sim::Sim;
use std::io::{Read, Write};
use std::time::Duration;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn start(name: &str, puck: PuckTarget, dir: &std::path::Path) -> (Engine, u16) {
    let port = free_port();
    let engine = Engine::start(Config {
        puck,
        endpoint: if cfg!(windows) {
            format!(r"\\.\pipe\hn-d059-{name}-{}", std::process::id()).into()
        } else {
            std::path::PathBuf::from("/tmp")
                .join(format!("hn-d059-{name}-{}", std::process::id()))
                .join("hn.sock")
        },
        claude_dir: None,
        latest_url: String::new(),
        uf2_drive: None,
        http_port: Some(port),
        agents_file: Some(dir.join("agents.json")),
        cloud: None,
    })
    .await
    .unwrap();
    (engine, port)
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

#[tokio::test(flavor = "multi_thread")]
async fn no_puck_glows_and_sounds_on_the_computer() {
    let dir = tempfile::tempdir().unwrap();
    let (e, port) = start("none", PuckTarget::None, dir.path()).await;
    let mut sounds = e.sounds();
    // The panel works without a puck: settings, schema, previews.
    assert!(e.puck("SCHEMA").await.unwrap().contains("harp"));
    e.puck("SET needs.color #112233").await.unwrap();
    assert!(e.puck("REBOOT").await.is_err(), "puck-only command");

    assert_eq!(post(port, "/v1/event/claude/needs").await, 204);
    let s = tokio::time::timeout(Duration::from_secs(3), sounds.recv())
        .await
        .expect("a sound")
        .unwrap();
    assert_eq!((s.state.as_str(), s.sound.as_str()), ("needs", "call"));
    let st = e.status().await;
    assert_eq!(st["output"]["device"], "computer");
    assert_eq!(st["output"]["shown"], "needs");
    assert_eq!(st["puck"]["connected"], false);

    // Mute (tray: Mute for 1 hour) silences the computer too.
    e.puck("MUTE 60").await.unwrap();
    assert_eq!(
        e.status().await["output"]["status"]["muted"].as_str(),
        Some("60")
    );
    // Settings survive a restart.
    let saved = std::fs::read_to_string(dir.path().join("here.json")).unwrap();
    assert!(saved.contains("#112233"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_puck_takes_over_and_lends_its_look() {
    let dir = tempfile::tempdir().unwrap();
    let sim = Sim::new("2.0.0", None);
    sim.puck
        .lock()
        .unwrap()
        .handle("SET working.color #abcdef", 0);
    let (e, _) = start("puck", PuckTarget::Sim(sim.clone()), dir.path()).await;
    for _ in 0..100 {
        if e.status().await["output"]["device"] == "puck" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(e.status().await["output"]["device"], "puck");
    // The puck's colours are copied to "this computer" shortly after it connects.
    let mut ok = false;
    for _ in 0..60 {
        let saved = std::fs::read_to_string(dir.path().join("here.json")).unwrap_or_default();
        if saved.contains("#abcdef") {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "puck look copied");
    // A change made on the puck is copied too.
    e.puck("SET done.color #fedcba").await.unwrap();
    let saved = std::fs::read_to_string(dir.path().join("here.json")).unwrap();
    assert!(saved.contains("#fedcba"));
}
