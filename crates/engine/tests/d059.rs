// SPDX-License-Identifier: GPL-3.0-or-later
//! D-059: this computer always shows the state (glow) and plays the sounds as the app's alert
//! settings say; a puck gets its part of those settings pushed.

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
    // The hub's settings commands work without a puck (app-events unchanged).
    assert!(e.puck("SCHEMA").await.unwrap().contains("harp"));
    e.puck("SET needs.color #112233").await.unwrap();
    assert!(e.puck("SET needs.light disco").await.is_err());
    assert!(e.puck("REBOOT").await.is_err(), "puck-only command");
    assert_eq!(e.alerts_get().states["needs"].screen.color, "#112233");

    assert_eq!(post(port, "/v1/event/claude/needs").await, 204);
    let s = tokio::time::timeout(Duration::from_secs(3), sounds.recv())
        .await
        .expect("a sound")
        .unwrap();
    assert_eq!((s.state.as_str(), s.sound.as_str()), ("needs", "call"));
    let st = e.status().await;
    assert_eq!(st["output"]["shown"], "needs");
    assert_eq!(st["puck"]["connected"], false);

    // Mute (tray: Mute for 1 hour) silences the computer too.
    e.puck("MUTE 60").await.unwrap();
    assert_eq!(
        e.status().await["output"]["status"]["muted"].as_str(),
        Some("60")
    );
    // Settings survive a restart.
    let saved = std::fs::read_to_string(dir.path().join("alerts.json")).unwrap();
    assert!(saved.contains("#112233"));
}

#[tokio::test(flavor = "multi_thread")]
async fn previews_and_events_reach_the_computer_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (e, port) = start("now", PuckTarget::None, dir.path()).await;
    let mut rx = e.subscribe();
    rx.borrow_and_update();
    // A preview: shown at once, and the change is signalled (the glow and tray follow).
    e.puck("TEST needs").await.unwrap();
    assert_eq!(e.computer_now().0, "test:needs");
    assert!(rx.has_changed().unwrap(), "preview signalled");
    // A real event takes over the preview; its end clears it at once.
    assert_eq!(post(port, "/v1/event/claude/working").await, 204);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(e.computer_now().0, "working");
    assert_eq!(post(port, "/v1/event/claude/end").await, 204);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(e.computer_now().0, "idle");
    // Done flashes for the screen's duration, then clears on its own.
    let mut a = e.alerts_get();
    a.states.get_mut("done").unwrap().screen.duration = 3;
    e.alerts_set(a).await.unwrap();
    assert_eq!(post(port, "/v1/event/claude/working").await, 204);
    assert_eq!(post(port, "/v1/event/claude/done").await, 204);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(e.computer_now().0, "done");
    tokio::time::sleep(Duration::from_millis(3_100)).await;
    assert_eq!(e.computer_now().0, "idle");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_computer_sound_follows_its_switch() {
    let dir = tempfile::tempdir().unwrap();
    let (e, port) = start("quiet-pc", PuckTarget::None, dir.path()).await;
    let mut sounds = e.sounds();
    let mut a = e.alerts_get();
    a.states
        .get_mut("needs")
        .unwrap()
        .sound
        .as_mut()
        .unwrap()
        .on = false;
    e.alerts_set(a).await.unwrap();
    assert_eq!(post(port, "/v1/event/claude/needs").await, 204);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(sounds.try_recv().is_err(), "sound switched off");
    assert_eq!(e.status().await["output"]["shown"], "needs", "still shown");
    // A bad setting is refused and nothing changes.
    let mut bad = e.alerts_get();
    bad.states.get_mut("done").unwrap().screen.brightness = 101;
    assert!(e.alerts_set(bad).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_app_pushes_its_settings_to_the_puck() {
    let dir = tempfile::tempdir().unwrap();
    // Not a first connect: the app's settings win over the puck's.
    let mut a = hn_engine::alerts::Alerts {
        puck_imported: true,
        ..Default::default()
    };
    a.states.get_mut("working").unwrap().puck.color = "#abcdef".into();
    a.states
        .get_mut("done")
        .unwrap()
        .puck_sound
        .as_mut()
        .unwrap()
        .on = false;
    a.save(&Some(dir.path().join("alerts.json"))).unwrap();
    let sim = Sim::new("2.0.0", None);
    let (e, _) = start("puck", PuckTarget::Sim(sim.clone()), dir.path()).await;
    let setting = |k: &str| sim.puck.lock().unwrap().settings.get(k).cloned();
    let mut ok = false;
    for _ in 0..80 {
        if setting("working.color").as_deref() == Some("#abcdef") {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "pushed");
    assert_eq!(setting("done.sound").as_deref(), Some("none"));
    assert_eq!(setting("brightness").as_deref(), Some("100"));
    assert_eq!(setting("needs.light").as_deref(), Some("pulse"));
    // A change in the app reaches the puck at once.
    let mut b = e.alerts_get();
    b.states.get_mut("needs").unwrap().puck.on = false;
    e.alerts_set(b).await.unwrap();
    assert_eq!(setting("needs.light").as_deref(), Some("off"));
    // The legacy hub key sets the puck too.
    e.puck("SET done.color #fedcba").await.unwrap();
    assert_eq!(setting("done.color").as_deref(), Some("#fedcba"));
}

/// Waits for the screen look to change; returns it and how long that took.
async fn next_look(
    rx: &mut tokio::sync::watch::Receiver<Option<hn_engine::output::Screen>>,
) -> (Option<hn_engine::output::Screen>, Duration) {
    let t = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(15), rx.changed())
        .await
        .expect("a change")
        .unwrap();
    (rx.borrow_and_update().clone(), t.elapsed())
}

#[tokio::test(flavor = "multi_thread")]
async fn screen_timing_is_exact_for_previews_and_events() {
    let dir = tempfile::tempdir().unwrap();
    let (e, port) = start("timing", PuckTarget::None, dir.path()).await;
    let mut a = e.alerts_get();
    a.states.get_mut("needs").unwrap().screen.duration = 3;
    a.states.get_mut("needs").unwrap().screen.light = "blink".into();
    a.states.get_mut("done").unwrap().screen.duration = 1;
    e.alerts_set(a).await.unwrap();
    let mut rx = e.screen();
    rx.borrow_and_update();

    // Preview: shows at once, for the state's duration (3 s), then off.
    e.puck("TEST needs").await.unwrap();
    let (look, _) = next_look(&mut rx).await;
    let look = look.expect("on");
    assert_eq!(
        (look.state.as_str(), look.light.as_str(), look.preview),
        ("needs", "blink", true)
    );
    let (look, took) = next_look(&mut rx).await;
    assert_eq!(look, None);
    assert!(
        took >= Duration::from_millis(2_800) && took < Duration::from_millis(3_600),
        "{took:?}"
    );

    // A real event takes the same path and the same duration.
    assert_eq!(post(port, "/v1/event/claude/needs").await, 204);
    let (look, _) = next_look(&mut rx).await;
    assert_eq!(
        look.as_ref().map(|l| (l.state.as_str(), l.preview)),
        Some(("needs", false))
    );
    let (look, took) = next_look(&mut rx).await;
    assert_eq!(look, None, "needs ran out; working underneath doesn't glow");
    assert!(
        took >= Duration::from_millis(2_500) && took < Duration::from_millis(3_600),
        "{took:?}"
    );

    // Done's preview lasts its own duration too.
    e.puck("TEST done").await.unwrap();
    let (look, _) = next_look(&mut rx).await;
    assert_eq!(look.map(|l| l.state), Some("done".into()));
    let (look, took) = next_look(&mut rx).await;
    assert_eq!(look, None);
    assert!(
        took < Duration::from_millis(1_600),
        "done lasts 1 s: {took:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_preview_lasts_exactly_the_set_duration_even_over_10s() {
    let dir = tempfile::tempdir().unwrap();
    let (e, _) = start("long", PuckTarget::None, dir.path()).await;
    let mut a = e.alerts_get();
    a.states.get_mut("done").unwrap().screen.duration = 12;
    e.alerts_set(a).await.unwrap();
    let mut rx = e.screen();
    rx.borrow_and_update();
    e.puck("TEST done").await.unwrap();
    assert!(next_look(&mut rx).await.0.is_some());
    let (look, took) = next_look(&mut rx).await;
    assert_eq!(look, None);
    assert!(
        took >= Duration::from_millis(11_800) && took < Duration::from_millis(12_600),
        "{took:?}"
    );
}
