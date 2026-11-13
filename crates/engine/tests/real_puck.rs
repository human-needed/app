//! Hardware-in-loop: the app engine against a real puck (testing §2). Skipped unless
//! `HN_REAL_PUCK` is set (`auto` or a port like `COM13`). Never runs BOOTSEL.
//!
//!   HN_REAL_PUCK=auto cargo test -p hn-engine --test real_puck -- --test-threads=1 --nocapture

use hn_engine::link::{Link, LinkEvent};
use hn_engine::{Config, Engine, PuckTarget};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

fn target() -> Option<PuckTarget> {
    let v = std::env::var("HN_REAL_PUCK").ok()?;
    Some(if v == "auto" {
        PuckTarget::Auto
    } else {
        PuckTarget::Serial(v)
    })
}

async fn shown(link: &Link) -> String {
    let s: Value = serde_json::from_str(&link.request("STATUS").await.unwrap()).unwrap();
    s["shown"].as_str().unwrap_or("?").to_string()
}

async fn connected(rx: &mut mpsc::Receiver<LinkEvent>) -> hn_core::proto::Hello {
    loop {
        match tokio::time::timeout(Duration::from_secs(15), rx.recv())
            .await
            .expect("puck not found")
        {
            Some(LinkEvent::Connected { hello, port }) => {
                eprintln!("connected on {port}: {hello:?}");
                return hello;
            }
            Some(_) => continue,
            None => panic!("link stopped"),
        }
    }
}

#[tokio::test]
async fn protocol_on_real_puck() {
    let Some(target) = target() else {
        eprintln!("HN_REAL_PUCK not set: skipped");
        return;
    };
    let (tx, mut rx) = mpsc::channel(16);
    let link = Link::start(target, tx);
    let hello = connected(&mut rx).await;
    assert_eq!(hello.proto, 2);

    // SNAP + priority + lease expiry.
    assert_eq!(link.request("SNAP 6 0000000a:working").await.unwrap(), "OK");
    assert_eq!(shown(&link).await, "working");
    assert_eq!(
        link.request("SNAP 6 0000000a:working 0000000b:needs")
            .await
            .unwrap(),
        "OK"
    );
    assert_eq!(shown(&link).await, "needs");
    assert!(
        link.request("SNAP 6 0000000a:done").await.is_err(),
        "bad state must be refused"
    );
    assert!(
        link.request("SNAP 4").await.is_err(),
        "lease < 5 must be refused"
    );
    assert_eq!(
        shown(&link).await,
        "needs",
        "a refused SNAP changes nothing"
    );
    tokio::time::sleep(Duration::from_millis(6500)).await;
    assert_eq!(shown(&link).await, "idle", "lease ran out");

    // FLASH done plays over needs.
    link.request("SNAP 30 0000000b:needs").await.unwrap();
    assert_eq!(link.request("FLASH done").await.unwrap(), "OK");
    assert_eq!(shown(&link).await, "done");
    link.request("SNAP 30").await.unwrap();
    // Let the flash finish (done.duration s), or it hides the TEST check below.
    let d: Value = serde_json::from_str(&link.request("GET done.duration").await.unwrap()).unwrap();
    let secs = d["done.duration"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .or(d["done.duration"].as_u64())
        .unwrap_or(5);
    eprintln!("done.duration {secs}s");
    tokio::time::sleep(Duration::from_millis(secs * 1000 + 500)).await;
    assert_eq!(shown(&link).await, "idle");

    // Settings: SCHEMA (long line), GET all, SET + restore.
    let schema: Value = serde_json::from_str(&link.request("SCHEMA").await.unwrap()).unwrap();
    eprintln!(
        "SCHEMA: {} groups",
        schema["groups"].as_array().unwrap().len()
    );
    assert!(schema["groups"].as_array().unwrap().len() >= 4);
    let all: Value = serde_json::from_str(&link.request("GET all").await.unwrap()).unwrap();
    let before = all["needs.color"].as_str().unwrap().to_string();
    assert_eq!(link.request("SET needs.color #123456").await.unwrap(), "OK");
    let one: Value = serde_json::from_str(&link.request("GET needs.color").await.unwrap()).unwrap();
    assert!(one.to_string().to_lowercase().contains("#123456"), "{one}");
    link.request(format!("SET needs.color {before}"))
        .await
        .unwrap();
    assert!(
        link.request("SET done.duration 0").await.is_err(),
        "done.duration ≥ 1"
    );

    // Mute and test.
    assert_eq!(link.request("MUTE 5").await.unwrap(), "OK");
    let s: Value = serde_json::from_str(&link.request("STATUS").await.unwrap()).unwrap();
    assert_ne!(s["muted"], "off");
    assert_eq!(link.request("MUTE 0").await.unwrap(), "OK");
    assert_eq!(link.request("TEST working").await.unwrap(), "OK");
    eprintln!("during TEST: {}", shown(&link).await);
    link.request("SNAP 30 0000000a:needs").await.unwrap();
    assert_eq!(
        shown(&link).await,
        "needs",
        "SNAP that changes the light ends TEST"
    );
    link.request("SNAP 30").await.unwrap();

    // REBOOT → reconnect (new connect gets !HELLO, table cleared).
    assert_eq!(link.request("REBOOT").await.unwrap(), "OK");
    let mut saw_disconnect = false;
    loop {
        match tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("puck didn't come back")
        {
            Some(LinkEvent::Disconnected) => saw_disconnect = true,
            Some(LinkEvent::Connected { .. }) => break,
            Some(LinkEvent::Rehello) => eprintln!("!HELLO"),
            None => panic!(),
        }
    }
    assert!(saw_disconnect);
    assert_eq!(shown(&link).await, "idle");
}

#[tokio::test]
async fn engine_on_real_puck() {
    let Some(target) = target() else { return };
    let engine = Engine::start(Config {
        puck: target,
        endpoint: if cfg!(windows) {
            format!(r"\\.\pipe\hn-hil-{}", std::process::id()).into()
        } else {
            std::env::temp_dir().join(format!("hn-hil-{}/hn.sock", std::process::id()))
        },
        claude_dir: None,
        latest_url: String::new(),
        uf2_drive: None,
    })
    .await
    .unwrap();
    for _ in 0..100 {
        if engine.puck_info().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let st = engine.status().await;
    assert_eq!(st["puck"]["connected"], true, "{st}");
    // What the settings window does: SCHEMA (~5.5 KB line) and GET all at the same time as STATUS.
    let (schema, all, st2) = tokio::join!(
        engine.puck("SCHEMA"),
        engine.puck("GET all"),
        engine.status()
    );
    assert!(schema.unwrap().starts_with('{'));
    assert!(all.unwrap().starts_with('{'));
    assert_eq!(st2["puck"]["connected"], true);
    assert_eq!(engine.request("test", &["needs".into()]).await["ok"], true);
    assert_eq!(engine.request("mute", &["0".into()]).await["ok"], true);
    // Engine keeps the lease alive: after 35 s the puck still has a live lease.
    tokio::time::sleep(Duration::from_secs(35)).await;
    let st = engine.status().await;
    let left = st["puck"]["status"]["lease_left"].as_u64().unwrap();
    assert!(
        left > 15,
        "lease kept alive by the 10 s snapshots, left {left}"
    );
    engine.shutdown().await;
}
