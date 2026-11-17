//! The app's core loop: `hn` events → session table → snapshot to the puck under a lease.

use crate::ipc::{self, Incoming};
use crate::link::{Link, LinkEvent, PuckTarget};
use crate::{agents, firmware, here, liveness, log, paths, quiet};
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
    /// Local HTTP receiver port (D-047); `None` = off.
    pub http_port: Option<u16>,
    /// Cloud link (relay-api §3); `None` = not available (tests).
    pub cloud: Option<crate::cloud::CloudConfig>,
    /// Last event per agent (D-054 status); `None` = memory only.
    pub agents_file: Option<PathBuf>,
}

impl Config {
    /// Defaults, overridable with `HN_PUCK`, `HN_IPC`, `HN_LATEST_URL`, `HN_UF2_DRIVE`,
    /// `HN_HTTP_PORT` (`0` = off).
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
            http_port: match std::env::var("HN_HTTP_PORT")
                .ok()
                .and_then(|p| p.parse::<u16>().ok())
            {
                Some(0) => None,
                Some(p) => Some(p),
                None => Some(crate::http::DEFAULT_PORT),
            },
            cloud: Some(crate::cloud::CloudConfig::from_env()),
            agents_file: Some(paths::data_dir().join("agents.json")),
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
    seen: agents::Seen,
    quiet: quiet::Quiet,
    /// "This computer" (D-059): the output while no puck is connected.
    here: here::Here,
    /// `here` holds the snapshot (no puck): cleared once a puck takes over.
    here_live: bool,
}

#[derive(Clone)]
pub struct Engine {
    pub config: Config,
    link: Link,
    shared: Arc<Mutex<Shared>>,
    changes: Arc<watch::Sender<u64>>,
    link_events: Arc<tokio::sync::broadcast::Sender<LinkEvent>>,
    http: Arc<Mutex<crate::http::HttpStatus>>,
    cloud: Option<crate::cloud::Cloud>,
    sounds: Arc<tokio::sync::broadcast::Sender<here::Sound>>,
}

/// Commands that only mean something to a real puck: never answered by "this computer".
const PUCK_ONLY: [&str; 4] = ["BOOTSEL", "REBOOT", "SELFTEST", "HELLO"];

fn now_ms() -> u64 {
    hn_sim::now_ms()
}

impl Engine {
    /// Starts IPC, the puck link and the loop. Fails if another instance owns the IPC endpoint.
    pub async fn start(config: Config) -> Result<Self, String> {
        let (in_tx, in_rx) = mpsc::channel(256);
        // The pipe also keeps the app single-instance (D-047 keeps it as a fallback for `hn`).
        ipc::serve(config.endpoint.clone(), in_tx.clone())
            .map_err(|e| format!("IPC {}: {e}", config.endpoint.display()))?;
        let (ev_tx, ev_rx) = mpsc::channel(32);
        let link = Link::start(config.puck.clone(), ev_tx);
        let (changes, _) = watch::channel(0);
        let (bcast, _) = tokio::sync::broadcast::channel(32);
        let changes = Arc::new(changes);
        let cloud = config.cloud.clone().map(|cfg| {
            let changes = changes.clone();
            crate::cloud::Cloud::start(cfg, in_tx.clone(), move || changes.send_modify(|n| *n += 1))
        });
        let seen = agents::Seen::load(config.agents_file.clone());
        let quiet = quiet::Quiet::load(
            config
                .agents_file
                .as_ref()
                .map(|f| f.with_file_name("quiet.json")),
        );
        let here = here::Here::load(
            config
                .agents_file
                .as_ref()
                .map(|f| f.with_file_name("here.json")),
        );
        let engine = Self {
            config,
            link,
            shared: Arc::new(Mutex::new(Shared {
                table: Table::new(),
                puck: None,
                last_snap: None,
                last_sent: Instant::now(),
                updating: None,
                seen,
                quiet,
                here,
                here_live: false,
            })),
            changes,
            link_events: Arc::new(bcast),
            http: Arc::new(Mutex::new(Default::default())),
            cloud,
            sounds: Arc::new(tokio::sync::broadcast::channel(16).0),
        };
        if let Some(port) = engine.config.http_port {
            let changes = engine.changes.clone();
            crate::http::serve(port, in_tx, engine.http.clone(), move || {
                changes.send_modify(|n| *n += 1)
            });
        }
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

    /// Sounds for the computer to play (D-059: only while no puck is connected).
    pub fn sounds(&self) -> tokio::sync::broadcast::Receiver<here::Sound> {
        self.sounds.subscribe()
    }

    /// Runs a command on "this computer"; its sounds go to [`Engine::sounds`].
    fn here(&self, line: &str) -> Result<String, String> {
        let (reply, sounds) = self.shared.lock().unwrap().here.handle(line, now_ms());
        for s in sounds {
            log!("sound {} {} on this computer", s.state, s.sound);
            let _ = self.sounds.send(s);
        }
        match reply.strip_prefix("ERR ") {
            Some(e) => Err(e.to_string()),
            None => Ok(reply),
        }
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
                        let procs = self.shared.lock().unwrap().table.agent_procs();
                        let obs = tokio::task::spawn_blocking(move || liveness::observe(dir.as_deref(), &procs)).await.unwrap_or_default();
                        let changed = self.shared.lock().unwrap().table.check_changes(&obs, now_ms());
                        for c in &changed {
                            log!("liveness {} {} -> idle ({})", c.agent, c.session, c.reason);
                        }
                        if !changed.is_empty() { self.changed(); }
                    }
                    if ticks.is_multiple_of(15) {
                        self.quiet_tick().await;
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
                let ok = hello.compatible();
                self.shared.lock().unwrap().puck = Some(PuckInfo { hello, port });
                self.send_snapshot(true);
                if ok {
                    let me = self.clone();
                    tokio::spawn(async move {
                        // Let the link settle (it answers our HELLO first).
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        me.full_levels().await;
                        // "This computer" takes the puck's look (D-059).
                        if let Ok(all) = me.link.request("GET all").await {
                            if let Ok(v) = serde_json::from_str::<Value>(&all) {
                                me.shared.lock().unwrap().here.mirror(&v);
                                me.changed();
                            }
                        }
                    });
                }
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
                let fx = {
                    let mut s = self.shared.lock().unwrap();
                    s.seen.saw(&ev.agent, now_ms());
                    if !s.seen.enabled(&ev.agent) {
                        return; // switched off (D-055): seen, but doesn't light the puck
                    }
                    s.table.apply(&ev, now_ms())
                };
                log!("event {} {} {:?}", ev.agent, ev.session, ev.state);
                if fx.flash_done {
                    if self.puck_ready() {
                        self.link.send("FLASH done");
                    } else {
                        let _ = self.here("FLASH done");
                    }
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

    /// Sends a command line to the puck (settings UI, `hn`), waits for the reply. With no puck
    /// connected, "this computer" answers (D-059). Settings changed on a puck are copied there.
    pub async fn puck(&self, line: impl Into<String>) -> Result<String, String> {
        let line = line.into();
        if line.contains('\n') || line.contains('\r') {
            return Err("bad command".into());
        }
        let cmd = line.split_whitespace().next().unwrap_or("");
        if !self.puck_ready() && !PUCK_ONLY.contains(&cmd) {
            return self.here(&line);
        }
        let r = self.link.request(line.clone()).await;
        if r.is_ok() && cmd == "SET" {
            self.shared.lock().unwrap().here.mirror_set(&line);
        }
        r
    }

    /// Which output shows the state: `puck` or `computer`.
    pub fn output(&self) -> &'static str {
        if self.puck_ready() {
            "puck"
        } else {
            "computer"
        }
    }

    fn puck_ready(&self) -> bool {
        let s = self.shared.lock().unwrap();
        s.updating.is_none() && s.puck.as_ref().is_some_and(|p| p.hello.compatible())
    }

    /// Sends the snapshot if it changed, if 10 s passed, or when forced.
    fn send_snapshot(&self, force: bool) {
        if !self.puck_ready() {
            // No puck: "this computer" shows it. Its lease is renewed every second.
            let line = {
                let mut s = self.shared.lock().unwrap();
                s.here_live = true;
                snap_line(LEASE_S, &s.table.snapshot())
            };
            let _ = self.here(&line);
            return;
        }
        let mut s = self.shared.lock().unwrap();
        if s.here_live {
            s.here_live = false;
            s.here.handle(&snap_line(LEASE_S, &[]), now_ms());
        }
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
        let (puck, sessions, snapshot, updating, agents, quiet) = {
            let s = self.shared.lock().unwrap();
            (
                s.puck.clone(),
                s.table.list(),
                s.table.snapshot(),
                s.updating.clone(),
                s.seen.list(now_ms()),
                s.quiet.status(),
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
        // What shows the state now (D-059): the puck, or this computer (glow and sound).
        let output = match &puck_status {
            Some(st) => json!({"device": "puck", "shown": shown, "status": st}),
            None => {
                let st = self
                    .here("STATUS")
                    .ok()
                    .and_then(|r| serde_json::from_str::<Value>(&r).ok())
                    .unwrap_or(Value::Null);
                json!({"device": "computer", "shown": st["shown"], "status": st})
            }
        };
        json!({
            "ok": true,
            "app": {"version": APP_VERSION, "ipc": self.config.endpoint.display().to_string(), "log": log::path()},
            "http": self.http.lock().unwrap().clone(),
            // D-049: Claude would send its hooks to a proxy.
            "proxy": crate::proxy::current(self.config.claude_dir.as_deref()),
            "cloud": self.cloud.as_ref().map(|c| c.status()),
            "puck": match puck {
                Some(p) => json!({
                    "connected": true, "port": p.port, "fw": p.hello.fw, "board": p.hello.board,
                    "serial": p.hello.serial, "proto": p.hello.proto, "compatible": p.hello.compatible(),
                    "shown": shown, "status": puck_status, "updating": updating,
                }),
                None => json!({"connected": false, "target": self.config.puck.describe(), "updating": updating}),
            },
            "sessions": sessions,
            // D-054: read-only; the user installs the plugins (guides at agents::SETUP_URL).
            "agents": agents,
            "quiet": quiet,
            "output": output,
            "setup_url": agents::SETUP_URL,
            "snapshot": snapshot.iter().map(|(id, st)| json!({"id": id, "state": st.as_str()})).collect::<Vec<_>>(),
        })
    }

    /// Turns the cloud link on or off (relay-api §3, D-030: opt-in).
    /// D-055: brightness and volume are set once, on the puck's master levels. Each state's own
    /// level is a share of the master (hn-usb/2 §5), so the app keeps them at 100. Only changed
    /// fields are written (the puck saves to flash only on change).
    pub async fn full_levels(&self) {
        let Ok(all) = self.puck("GET all").await else {
            return;
        };
        let Ok(v) = serde_json::from_str::<Value>(&all) else {
            return;
        };
        for st in ["needs", "done", "working", "idle"] {
            for f in ["brightness", "volume"] {
                let key = format!("{st}.{f}");
                let cur = match &v[&key] {
                    Value::Number(n) => n.as_u64(),
                    Value::String(s) => s.parse().ok(),
                    _ => None, // idle has no volume
                };
                if cur.is_some_and(|c| c != 100) {
                    match self.puck(format!("SET {key} 100")).await {
                        Ok(_) => log!(
                            "puck: {key} {} -> 100 (levels live on the master)",
                            cur.unwrap()
                        ),
                        Err(e) => log!("puck: SET {key} failed: {e}"),
                    }
                }
            }
        }
    }

    /// Switches an agent on or off (D-055). Off: its sessions stop lighting the puck at once.
    pub fn agent_set(&self, id: &str, on: bool) -> Result<(), String> {
        {
            let mut s = self.shared.lock().unwrap();
            s.seen.set_enabled(id, on)?;
            if !on {
                s.table.clear_agent(id);
            }
        }
        log!("agent {id} {}", if on { "on" } else { "off" });
        self.send_snapshot(true);
        self.changed();
        Ok(())
    }

    pub fn quiet_get(&self) -> quiet::Settings {
        self.shared.lock().unwrap().quiet.settings()
    }

    /// Saves quiet hours and applies them at once.
    pub async fn quiet_set(&self, q: quiet::Settings) -> Result<(), String> {
        self.shared.lock().unwrap().quiet.set(q)?;
        self.quiet_tick().await;
        self.changed();
        Ok(())
    }

    /// Enters or leaves quiet hours (checked every 15 s and on change).
    async fn quiet_tick(&self) {
        if self.shared.lock().unwrap().updating.is_some() {
            return;
        }
        let action = {
            let s = self.shared.lock().unwrap();
            s.quiet.due(quiet::local_minutes())
        };
        match action {
            quiet::Action::None => {}
            quiet::Action::Enter { mute_min } => {
                let b = self
                    .puck("GET brightness")
                    .await
                    .ok()
                    .and_then(|r| quiet::level_of(&r, "brightness"));
                let Some(b) = b else { return };
                let dim = quiet::dimmed(b);
                let _ = self.puck(format!("SET brightness {dim}")).await;
                let _ = self.puck(format!("MUTE {mute_min}")).await;
                self.shared.lock().unwrap().quiet.entered(b, dim);
                log!("quiet hours: on (brightness {b} -> {dim}, muted {mute_min} min)");
                self.changed();
            }
            quiet::Action::Leave { restore } => {
                let now = self
                    .puck("GET brightness")
                    .await
                    .ok()
                    .and_then(|r| quiet::level_of(&r, "brightness"));
                // Put the level back unless the user changed it meanwhile.
                if let (Some((saved, dim)), Some(now)) = (restore, now) {
                    if now == dim {
                        let _ = self.puck(format!("SET brightness {saved}")).await;
                    }
                }
                let muted = self.puck("STATUS").await.ok().and_then(|r| {
                    serde_json::from_str::<Value>(&r).ok()?["muted"]
                        .as_str()
                        .map(String::from)
                });
                // Our timed mute (a number of minutes); "on" = the user's own mute: keep it.
                if muted.is_some_and(|m| m != "on" && m != "off") {
                    let _ = self.puck("MUTE 0").await;
                }
                self.shared.lock().unwrap().quiet.left();
                log!("quiet hours: off");
                self.changed();
            }
        }
    }

    pub async fn cloud_set(&self, on: bool) -> Result<(), String> {
        let c = self.cloud.as_ref().ok_or("cloud link not available")?;
        c.set_enabled(on).await?;
        log!("cloud: link {}", if on { "on" } else { "off" });
        self.changed();
        Ok(())
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
