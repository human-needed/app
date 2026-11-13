//! The app's core loop: `hn` events → session table → snapshot to the puck under a lease.

use crate::ipc::{self, Incoming};
use crate::link::{Link, LinkEvent, PuckTarget};
use crate::{firmware, liveness, log, paths};
use hn_core::event::Message;
use hn_core::proto::{snap_line, Hello, SnapState, LEASE_S, RESEND_MS};
use hn_core::sessions::Table;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_LATEST_URL: &str = "https://human-needed.com/firmware/latest.json";

#[derive(Clone)]
pub struct Config {
    pub puck: PuckTarget,
    pub endpoint: PathBuf,
    pub claude_dir: Option<PathBuf>,
    pub latest_url: String,
    /// Fake RPI-RP2 drive folder (tests); `None` = look for the real drive.
    pub uf2_drive: Option<PathBuf>,
}

impl Config {
    /// Defaults, overridable with `HN_PUCK`, `HN_IPC`, `HN_LATEST_URL`, `HN_UF2_DRIVE`.
    pub fn from_env() -> Self {
        let puck = std::env::var("HN_PUCK")
            .ok()
            .and_then(|s| PuckTarget::parse(&s))
            .unwrap_or(PuckTarget::Auto);
        Self {
            puck,
            endpoint: hn_core::endpoint::ipc_endpoint(),
            claude_dir: paths::claude_dir(),
            latest_url: std::env::var("HN_LATEST_URL")
                .unwrap_or_else(|_| DEFAULT_LATEST_URL.into()),
            uf2_drive: std::env::var_os("HN_UF2_DRIVE").map(PathBuf::from),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PuckInfo {
    pub hello: Hello,
    pub port: String,
}

struct Shared {
    table: Table,
    puck: Option<PuckInfo>,
    last_snap: Option<Vec<(String, SnapState)>>,
    last_sent: Instant,
    /// Set while a firmware update runs (no snapshots then).
    updating: Option<String>,
}

#[derive(Clone)]
pub struct Engine {
    pub config: Config,
    link: Link,
    shared: Arc<Mutex<Shared>>,
    changes: Arc<watch::Sender<u64>>,
    link_events: Arc<tokio::sync::broadcast::Sender<LinkEvent>>,
}

fn now_ms() -> u64 {
    hn_sim::now_ms()
}

impl Engine {
    /// Starts IPC, the puck link and the loop. Fails if another instance owns the IPC endpoint.
    pub async fn start(config: Config) -> Result<Self, String> {
        let (in_tx, in_rx) = mpsc::channel(256);
        ipc::serve(config.endpoint.clone(), in_tx)
            .map_err(|e| format!("IPC {}: {e}", config.endpoint.display()))?;
        let (ev_tx, ev_rx) = mpsc::channel(32);
        let link = Link::start(config.puck.clone(), ev_tx);
        let (changes, _) = watch::channel(0);
        let (bcast, _) = tokio::sync::broadcast::channel(32);
        let engine = Self {
            config,
            link,
            shared: Arc::new(Mutex::new(Shared {
                table: Table::new(),
                puck: None,
                last_snap: None,
                last_sent: Instant::now(),
                updating: None,
            })),
            changes: Arc::new(changes),
            link_events: Arc::new(bcast),
        };
        log!(
            "app {APP_VERSION} started; ipc {} puck {}",
            engine.config.endpoint.display(),
            engine.config.puck.describe()
        );
        tokio::spawn(engine.clone().run(in_rx, ev_rx));
        Ok(engine)
    }

    /// Bumped on every visible change (for the UI / tray).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn link_events(&self) -> tokio::sync::broadcast::Receiver<LinkEvent> {
        self.link_events.subscribe()
    }

    fn changed(&self) {
        self.changes.send_modify(|n| *n += 1);
    }

    async fn run(self, mut in_rx: mpsc::Receiver<Incoming>, mut ev_rx: mpsc::Receiver<LinkEvent>) {
        let mut tick = tokio::time::interval(Duration::from_millis(1000));
        let mut ticks: u64 = 0;
        loop {
            tokio::select! {
                Some(inc) = in_rx.recv() => self.incoming(inc),
                Some(ev) = ev_rx.recv() => {
                    let _ = self.link_events.send(ev.clone());
                    self.link_event(ev);
                }
                _ = tick.tick() => {
                    ticks += 1;
                    if ticks.is_multiple_of(2) {
                        let dir = self.config.claude_dir.clone();
                        let obs = tokio::task::spawn_blocking(move || liveness::observe(dir.as_deref())).await.unwrap_or_default();
                        let changed = self.shared.lock().unwrap().table.check_changes(&obs, now_ms());
                        for c in &changed {
                            log!("liveness {} {} -> idle ({})", c.agent, c.session, c.reason);
                        }
                        if !changed.is_empty() { self.changed(); }
                    }
                    self.send_snapshot(false);
                }
            }
        }
    }

    fn link_event(&self, ev: LinkEvent) {
        match ev {
            LinkEvent::Connected { hello, port } => {
                if !hello.compatible() {
                    log!(
                        "puck: firmware {} speaks protocol {}, app needs 2: update the firmware",
                        hello.fw,
                        hello.proto
                    );
                }
                self.shared.lock().unwrap().puck = Some(PuckInfo { hello, port });
                self.send_snapshot(true);
            }
            LinkEvent::Disconnected => {
                let mut s = self.shared.lock().unwrap();
                s.puck = None;
                s.last_snap = None;
            }
            LinkEvent::Rehello => self.send_snapshot(true),
        }
        self.changed();
    }

    fn incoming(&self, inc: Incoming) {
        match inc.msg {
            Message::Event(ev) => {
                let fx = self.shared.lock().unwrap().table.apply(&ev, now_ms());
                log!("event {} {} {:?}", ev.agent, ev.session, ev.state);
                if fx.flash_done && self.puck_ready() {
                    self.link.send("FLASH done");
                }
                self.send_snapshot(false);
                self.changed();
            }
            Message::Request(req) => {
                let me = self.clone();
                tokio::spawn(async move {
                    let answer = me.request(&req.cmd, &req.args).await;
                    if let Some(r) = inc.reply {
                        let _ = r.send(answer.to_string());
                    }
                });
            }
        }
    }

    /// `hn status | test | mute | set | clear` (app-events §4).
    pub async fn request(&self, cmd: &str, args: &[String]) -> Value {
        let result = match (cmd, args) {
            ("status", []) => return self.status().await,
            ("test", [st]) => self.puck(format!("TEST {st}")).await,
            ("mute", [m]) => self.puck(format!("MUTE {m}")).await,
            ("set", [k, v]) => self.puck(format!("SET {k} {v}")).await,
            ("clear", []) => {
                self.shared.lock().unwrap().table.clear();
                self.send_snapshot(true);
                self.changed();
                Ok("OK".into())
            }
            _ => Err(format!("unknown command {cmd}")),
        };
        match result {
            Ok(_) => json!({"ok": true}),
            Err(e) => json!({"ok": false, "error": e}),
        }
    }

    /// Sends a command line to the puck (settings UI, `hn`), waits for the reply.
    pub async fn puck(&self, line: impl Into<String>) -> Result<String, String> {
        let line = line.into();
        if line.contains('\n') || line.contains('\r') {
            return Err("bad command".into());
        }
        self.link.request(line).await
    }

    fn puck_ready(&self) -> bool {
        let s = self.shared.lock().unwrap();
        s.updating.is_none() && s.puck.as_ref().is_some_and(|p| p.hello.compatible())
    }

    /// Sends the snapshot if it changed, if 10 s passed, or when forced.
    fn send_snapshot(&self, force: bool) {
        if !self.puck_ready() {
            return;
        }
        let mut s = self.shared.lock().unwrap();
        let snap = s.table.snapshot();
        let due = s.last_sent.elapsed() >= Duration::from_millis(RESEND_MS - 300);
        if force || due || s.last_snap.as_ref() != Some(&snap) {
            self.link.send(snap_line(LEASE_S, &snap));
            s.last_snap = Some(snap);
            s.last_sent = Instant::now();
        }
    }

    pub fn puck_info(&self) -> Option<PuckInfo> {
        self.shared.lock().unwrap().puck.clone()
    }

    pub async fn status(&self) -> Value {
        let (puck, sessions, snapshot, updating) = {
            let s = self.shared.lock().unwrap();
            (
                s.puck.clone(),
                s.table.list(),
                s.table.snapshot(),
                s.updating.clone(),
            )
        };
        let puck_status = if puck.as_ref().is_some_and(|p| p.hello.compatible()) {
            self.puck("STATUS")
                .await
                .ok()
                .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        } else {
            None
        };
        let shown = puck_status.as_ref().and_then(|p| p.get("shown").cloned());
        json!({
            "ok": true,
            "app": {"version": APP_VERSION, "ipc": self.config.endpoint.display().to_string(), "log": log::path()},
            "puck": match puck {
                Some(p) => json!({
                    "connected": true, "port": p.port, "fw": p.hello.fw, "board": p.hello.board,
                    "serial": p.hello.serial, "proto": p.hello.proto, "compatible": p.hello.compatible(),
                    "shown": shown, "status": puck_status, "updating": updating,
                }),
                None => json!({"connected": false, "target": self.config.puck.describe(), "updating": updating}),
            },
            "sessions": sessions,
            "snapshot": snapshot.iter().map(|(id, st)| json!({"id": id, "state": st.as_str()})).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn set_updating(&self, what: Option<String>) {
        let mut s = self.shared.lock().unwrap();
        s.updating = what;
        s.last_snap = None;
        drop(s);
        self.changed();
    }

    /// Checks `latest.json` for a newer firmware for the connected puck.
    pub async fn check_update(&self) -> Result<firmware::Available, String> {
        let puck = self.puck_info().ok_or("puck not connected")?;
        firmware::check(&self.config.latest_url, &puck.hello).await
    }

    /// Downloads, verifies and installs a firmware (D-044). `progress` gets short status lines.
    pub async fn update_firmware(
        &self,
        progress: impl Fn(&str) + Send + Sync,
    ) -> Result<String, String> {
        firmware::update(self, &progress).await
    }

    /// Clears the light before the app exits (lease would clear it within 30 s anyway).
    pub async fn shutdown(&self) {
        if self.puck_ready() {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                self.link.request(snap_line(LEASE_S, &[])),
            )
            .await;
        }
        log!("app stopped");
    }
}
