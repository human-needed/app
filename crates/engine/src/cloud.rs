//! Cloud link, PC side of `hn-relay/1` (relay-api §2, §3; D-021, D-023, D-030).
//!
//! Cloud chats (Cowork, Claude on the web) run their hooks in a cloud VM that can't reach this
//! PC. Their hook POSTs to the relay under the account's route key `K`; this module keeps a
//! WebSocket to the relay and feeds what arrives into the session table, so the puck lights for
//! them too. Opt-in (off by default).
//!
//! - `K = sha256("humanneeded-account:" + accountUuid)[:32 hex]`. Not secret. The app listens on
//!   every account it finds (D-051): Claude Desktop's `config.json` (`lastKnownAccountUuid`, follows
//!   the Desktop sign-in, which runs Cowork) and `~/.claude.json` (`oauthAccount.accountUuid`,
//!   Claude Code's login; can be stale or another account). One connection per `K`, same
//!   `install_id`; the files are read again every minute.
//! - `install_id` (128-bit) and secret `S` (256-bit) are made when the link is turned on and kept in
//!   the OS credential store (Windows Credential Manager, macOS Keychain); never in a file.
//! - Connect, `hello` at once, `ping` every 4 min, reconnect with backoff 5 → 120 s (the relay
//!   closes every 2 h). Every 3 min a probe is POSTed to `K`; seeing it come back proves the whole
//!   path works (shown as "working" in the UI).
//! - `{"t":"state"}` → session `h` of agent `claude-cloud`, with a 120 s ttl: heartbeats every
//!   45 s keep it; silence drops it (D-019).
//! - `{"t":"err","e":"key_full"}`: three other computers own `K`; the UI says so. Turning the link
//!   off on an owning PC sends `unbind` for itself (frees its slot) and deletes its secret.

use crate::ipc::Incoming;
use crate::log;
use futures_util::{SinkExt, StreamExt};
use hn_core::event::{AgentState, Event, Message};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message as WsMsg;

pub const DEFAULT_WS: &str = "wss://relay-ws.human-needed.com/v1";
pub const DEFAULT_POST: &str = "https://relay.human-needed.com";
/// Agent id for cloud sessions in the table (and the puck id).
pub const AGENT: &str = "claude-cloud";
/// A cloud session without any message for this long is dropped (relay-api §4, D-019).
pub const SESSION_TTL_S: u64 = 120;
const PING_EVERY: Duration = Duration::from_secs(4 * 60);
const PROBE_EVERY: Duration = Duration::from_secs(3 * 60);
const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(120);
/// A connection that lasted this long resets the backoff.
const STABLE: Duration = Duration::from_secs(60);
/// How often the account files are read again (sign-in changes, new accounts).
const RESCAN: Duration = Duration::from_secs(60);

/// Route key from Claude's account id (same as the cloud hook).
pub fn route_key(account_uuid: &str) -> String {
    let d = Sha256::digest(format!("humanneeded-account:{account_uuid}").as_bytes());
    hex(&d)[..32].to_string()
}

/// The probe's session id: fixed per key, so every PC of this account ignores it.
pub fn probe_h(k: &str) -> String {
    let d = Sha256::digest(format!("hn-probe:{k}").as_bytes());
    hex(&d)[..16].to_string()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("OS random numbers");
    hex(&b)
}

/// The account id in a Claude file: `~/.claude.json` (`oauthAccount.accountUuid`) or Claude
/// Desktop's `config.json` (`lastKnownAccountUuid`).
pub fn account_uuid(file: &Path) -> Option<String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let id = v["oauthAccount"]["accountUuid"]
        .as_str()
        .or(v["accountUuid"].as_str())
        .or(v["lastKnownAccountUuid"].as_str())?;
    let ok =
        !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    ok.then(|| id.to_string())
}

/// `pattern` with one `*` inside path components (e.g. `Claude_*`) -> the existing files it names.
pub fn expand(pattern: &Path) -> Vec<PathBuf> {
    let mut found = vec![PathBuf::new()];
    for comp in pattern.components() {
        let part = comp.as_os_str().to_string_lossy().to_string();
        let Some((pre, post)) = part.split_once('*') else {
            found.iter_mut().for_each(|p| p.push(comp));
            continue;
        };
        let mut next: Vec<PathBuf> = found
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.len() >= pre.len() + post.len() && n.starts_with(pre) && n.ends_with(post)
            })
            .map(|e| e.path())
            .collect();
        next.sort();
        found = next;
    }
    found.into_iter().filter(|p| p.is_file()).collect()
}

/// Every account found, in file order, no duplicates.
pub fn accounts(files: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in files.iter().flat_map(|p| expand(p)) {
        if let Some(a) = account_uuid(&f) {
            if !out.contains(&a) {
                out.push(a);
            }
        }
    }
    out
}

/// Where Claude Desktop keeps `config.json` (undocumented; D-051). Windows: the regular install,
/// and the Store (MSIX) build's data folder (seen on PC2) and virtualized `%APPDATA%`.
pub fn desktop_configs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(windows) {
        if let Some(a) = std::env::var_os("APPDATA").map(PathBuf::from) {
            v.push(a.join("Claude").join("config.json"));
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            v.push(l.join("Claude-Data").join("*").join("config.json"));
            v.push(
                l.join("Packages")
                    .join("Claude_*")
                    .join("LocalCache/Roaming/Claude/config.json"),
            );
        }
    } else if let Some(h) = dirs::home_dir() {
        if cfg!(target_os = "macos") {
            v.push(h.join("Library/Application Support/Claude/config.json"));
        } else {
            v.push(h.join(".config/Claude/config.json"));
        }
    }
    v
}

/// This install's identity on the relay.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub i: String,
    pub s: String,
}

impl Identity {
    pub fn new() -> Self {
        Self {
            i: random_hex(16),
            s: random_hex(32),
        }
    }
}

impl Default for Identity {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the identity lives. The real one is the OS credential store.
pub trait CredStore: Send + Sync {
    fn load(&self) -> Option<Identity>;
    fn save(&self, id: &Identity) -> Result<(), String>;
    fn delete(&self);
}

/// Windows Credential Manager / macOS Keychain / Linux keyutils, via `keyring`.
pub struct OsCreds {
    pub service: String,
}

impl OsCreds {
    fn entry(&self) -> Option<keyring::Entry> {
        keyring::Entry::new(&self.service, "cloud-link").ok()
    }
}

impl CredStore for OsCreds {
    fn load(&self) -> Option<Identity> {
        serde_json::from_str(&self.entry()?.get_password().ok()?).ok()
    }
    fn save(&self, id: &Identity) -> Result<(), String> {
        let text = serde_json::to_string(id).map_err(|e| e.to_string())?;
        self.entry()
            .ok_or("no credential store")?
            .set_password(&text)
            .map_err(|e| format!("credential store: {e}"))
    }
    fn delete(&self) {
        if let Some(e) = self.entry() {
            let _ = e.delete_credential();
        }
    }
}

/// In memory (tests).
#[derive(Default)]
pub struct MemCreds(pub Mutex<Option<Identity>>);

impl CredStore for MemCreds {
    fn load(&self) -> Option<Identity> {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, id: &Identity) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(id.clone());
        Ok(())
    }
    fn delete(&self) {
        *self.0.lock().unwrap() = None;
    }
}

#[derive(Clone)]
pub struct CloudConfig {
    pub ws_url: String,
    pub post_url: String,
    /// Files holding Claude account ids, in order (Desktop `config.json`s, `~/.claude.json`);
    /// a `*` in a component matches part of a folder name.
    pub account_files: Vec<PathBuf>,
    /// `{"enabled": bool}` (no secrets).
    pub settings_file: PathBuf,
    pub creds: Arc<dyn CredStore>,
}

impl CloudConfig {
    /// Real endpoints and stores. With `HN_DEV=1` only (relay-api §4 rule for overrides):
    /// `HN_RELAY_WS`, `HN_RELAY_POST`, `HN_CLAUDE_JSON`, `HN_CLAUDE_DESKTOP_CONFIG`,
    /// `HN_CRED_SERVICE` (setting either account file uses only the ones set).
    pub fn from_env() -> Self {
        let dev = std::env::var_os("HN_DEV").is_some();
        let var = |n: &str| dev.then(|| std::env::var(n).ok()).flatten();
        let (cj, dc) = (var("HN_CLAUDE_JSON"), var("HN_CLAUDE_DESKTOP_CONFIG"));
        let account_files = if cj.is_some() || dc.is_some() {
            dc.into_iter().chain(cj).map(PathBuf::from).collect()
        } else {
            let mut v = desktop_configs();
            v.extend(dirs::home_dir().map(|h| h.join(".claude.json")));
            v
        };
        Self {
            ws_url: var("HN_RELAY_WS").unwrap_or_else(|| DEFAULT_WS.into()),
            post_url: var("HN_RELAY_POST").unwrap_or_else(|| DEFAULT_POST.into()),
            account_files,
            settings_file: crate::paths::data_dir().join("cloud.json"),
            creds: Arc::new(OsCreds {
                service: var("HN_CRED_SERVICE").unwrap_or_else(|| "HumanNeeded".into()),
            }),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    #[default]
    Off,
    /// No Claude account found (signed out of Claude Desktop and Claude Code).
    NoAccount,
    Connecting,
    /// Socket open, hello sent; waiting for the first probe to come back.
    Connected,
    /// A probe came back: cloud chats will light the puck.
    Working,
    /// Three other computers own this account's key.
    KeyFull,
    Error,
}

/// One account's link.
#[derive(Debug, Clone, Default, Serialize)]
pub struct KeyStatus {
    pub key: String,
    pub state: LinkState,
    pub error: Option<String>,
    pub verified_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CloudStatus {
    pub enabled: bool,
    pub state: LinkState,
    /// Plain words for the UI when something is wrong.
    pub error: Option<String>,
    /// Last time (ms) a probe came back.
    pub verified_at: Option<u64>,
    /// Non-secret route key of the first account (support).
    pub key: Option<String>,
    /// Every account's link (D-051), in `account_files` order.
    pub keys: Vec<KeyStatus>,
}

/// Best first: the overall state is the best account's.
fn rank(s: &LinkState) -> u8 {
    match s {
        LinkState::Working => 0,
        LinkState::Connected => 1,
        LinkState::Connecting => 2,
        LinkState::Error => 3,
        LinkState::KeyFull => 4,
        LinkState::NoAccount => 5,
        LinkState::Off => 6,
    }
}

#[derive(Clone)]
pub struct Cloud {
    cfg: CloudConfig,
    status: Arc<Mutex<CloudStatus>>,
    enabled: Arc<watch::Sender<bool>>,
}

fn read_enabled(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["enabled"].as_bool())
        .unwrap_or(false)
}

impl Cloud {
    /// Starts the manager task. `on_change` is called when the status changes.
    pub fn start(
        cfg: CloudConfig,
        tx: mpsc::Sender<Incoming>,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let on = read_enabled(&cfg.settings_file);
        let (en_tx, en_rx) = watch::channel(on);
        let cloud = Self {
            cfg,
            status: Arc::new(Mutex::new(CloudStatus {
                enabled: on,
                ..Default::default()
            })),
            enabled: Arc::new(en_tx),
        };
        let c = cloud.clone();
        tokio::spawn(async move { c.manage(en_rx, tx, Arc::new(on_change)).await });
        cloud
    }

    pub fn status(&self) -> CloudStatus {
        self.status.lock().unwrap().clone()
    }

    /// Turns the link on (creating this install's identity if needed) or off (unbinding this
    /// install from the key and deleting its secret).
    pub async fn set_enabled(&self, on: bool) -> Result<(), String> {
        if on && self.cfg.creds.load().is_none() {
            self.cfg.creds.save(&Identity::new())?;
        }
        if !on {
            self.unbind_self().await;
            self.cfg.creds.delete();
        }
        if let Some(dir) = self.cfg.settings_file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(
            &self.cfg.settings_file,
            json!({ "enabled": on }).to_string(),
        )
        .map_err(|e| e.to_string())?;
        self.status.lock().unwrap().enabled = on;
        self.enabled.send_replace(on);
        Ok(())
    }

    /// Frees this install's slot on every key (best effort, own short connections).
    async fn unbind_self(&self) {
        let Some(id) = self.cfg.creds.load() else {
            return;
        };
        let mut keys = self.keys();
        for k in self.status().keys.into_iter().map(|s| s.key) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        for k in keys {
            self.unbind(&k, &id).await;
        }
    }

    async fn unbind(&self, k: &str, id: &Identity) {
        let r = tokio::time::timeout(Duration::from_secs(10), async {
            let (mut ws, _) = tokio_tungstenite::connect_async(&self.cfg.ws_url)
                .await
                .ok()?;
            ws.send(WsMsg::text(hello(k, id))).await.ok()?;
            tokio::time::sleep(Duration::from_millis(1500)).await;
            ws.send(WsMsg::text(json!({"t": "unbind", "i": id.i}).to_string()))
                .await
                .ok()?;
            tokio::time::sleep(Duration::from_millis(1000)).await;
            let _ = ws.close(None).await;
            Some(())
        })
        .await;
        log!(
            "cloud: unbind {}",
            if matches!(r, Ok(Some(()))) {
                "sent"
            } else {
                "failed (slot frees itself in 30 days)"
            }
        );
    }

    /// Route keys of every account found now.
    fn keys(&self) -> Vec<String> {
        accounts(&self.cfg.account_files)
            .iter()
            .map(|a| route_key(a))
            .collect()
    }

    /// Sets the overall state when no account link runs (off, no account, store error).
    fn set(&self, state: LinkState, error: Option<String>, on_change: &(dyn Fn() + Send + Sync)) {
        let mut s = self.status.lock().unwrap();
        if s.state != state || s.error != error || !s.keys.is_empty() {
            s.state = state;
            s.error = error;
            s.keys.clear();
            s.key = None;
            drop(s);
            on_change();
        }
    }

    /// Sets one account's state and recomputes the overall one.
    fn set_key(
        &self,
        k: &str,
        state: LinkState,
        error: Option<String>,
        on_change: &(dyn Fn() + Send + Sync),
    ) {
        let mut s = self.status.lock().unwrap();
        let Some(ks) = s.keys.iter_mut().find(|x| x.key == k) else {
            return;
        };
        if ks.state == state && ks.error == error {
            return;
        }
        ks.state = state;
        ks.error = error;
        Self::summarize(&mut s);
        drop(s);
        on_change();
    }

    fn summarize(s: &mut CloudStatus) {
        let best = s.keys.iter().min_by_key(|k| rank(&k.state)).cloned();
        s.key = s.keys.first().map(|k| k.key.clone());
        s.verified_at = s.keys.iter().filter_map(|k| k.verified_at).max();
        match best {
            Some(b) => {
                s.state = b.state;
                s.error = b.error;
            }
            None => {
                s.state = LinkState::NoAccount;
                s.error = None;
            }
        }
    }

    /// Keeps one link per account found; reads the account files again every [`RESCAN`].
    async fn manage(
        self,
        mut en_rx: watch::Receiver<bool>,
        tx: mpsc::Sender<Incoming>,
        on_change: Arc<dyn Fn() + Send + Sync>,
    ) {
        let mut links: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
        fn stop_all(links: &mut HashMap<String, tokio::task::JoinHandle<()>>) {
            links.drain().for_each(|(_, h)| h.abort());
        }
        loop {
            if !*en_rx.borrow() {
                stop_all(&mut links);
                self.set(LinkState::Off, None, &*on_change);
                if en_rx.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let keys = self.keys();
            if keys.is_empty() {
                stop_all(&mut links);
                self.set(
                    LinkState::NoAccount,
                    Some("Sign in to Claude on this computer first (no account found).".into()),
                    &*on_change,
                );
                let _ = tokio::time::timeout(RESCAN, en_rx.changed()).await;
                continue;
            }
            let id = match self.cfg.creds.load() {
                Some(id) => id,
                None => {
                    let id = Identity::new();
                    if let Err(e) = self.cfg.creds.save(&id) {
                        stop_all(&mut links);
                        self.set(LinkState::Error, Some(e), &*on_change);
                        let _ = tokio::time::timeout(BACKOFF_MAX, en_rx.changed()).await;
                        continue;
                    }
                    id
                }
            };
            let changed = {
                let mut s = self.status.lock().unwrap();
                let before: Vec<String> = s.keys.iter().map(|k| k.key.clone()).collect();
                let mut old: BTreeMap<String, KeyStatus> =
                    s.keys.drain(..).map(|k| (k.key.clone(), k)).collect();
                s.keys = keys
                    .iter()
                    .map(|k| {
                        old.remove(k).unwrap_or_else(|| KeyStatus {
                            key: k.clone(),
                            state: LinkState::Connecting,
                            ..Default::default()
                        })
                    })
                    .collect();
                Self::summarize(&mut s);
                before != keys
            };
            links.retain(|k, h| {
                let keep = keys.contains(k) && !h.is_finished();
                if !keep {
                    h.abort();
                }
                keep
            });
            for k in &keys {
                if !links.contains_key(k) {
                    log!("cloud: listening on key {}", &k[..8]);
                    let me = self.clone();
                    let (k2, id, tx, oc) = (k.clone(), id.clone(), tx.clone(), on_change.clone());
                    links.insert(
                        k.clone(),
                        tokio::spawn(async move { me.key_loop(k2, id, tx, oc).await }),
                    );
                }
            }
            if changed {
                on_change();
            }
            let _ = tokio::time::timeout(RESCAN, en_rx.changed()).await;
        }
    }

    /// One account's link: connect, run, back off, again (until aborted).
    async fn key_loop(
        self,
        k: String,
        id: Identity,
        tx: mpsc::Sender<Incoming>,
        on_change: Arc<dyn Fn() + Send + Sync>,
    ) {
        let mut backoff = BACKOFF_MIN;
        loop {
            let started = tokio::time::Instant::now();
            match self.session(&k, &id, &tx, &*on_change).await {
                End::KeyFull => {
                    self.set_key(
                        &k,
                        LinkState::KeyFull,
                        Some("Three other computers already use the cloud link for this Claude account. Turn it off on one of them, or wait: a computer that hasn't connected for 30 days frees its place.".into()),
                        &*on_change,
                    );
                    // Retry rarely: a slot may free up.
                    tokio::time::sleep(Duration::from_secs(30 * 60)).await;
                    continue;
                }
                End::Refused(e) => {
                    log!("cloud: refused: {e}");
                    self.set_key(
                        &k,
                        LinkState::Error,
                        Some(format!("The relay refused this computer ({e}). Turn the cloud link off and on again to reset it.")),
                        &*on_change,
                    );
                }
                End::Lost(e) => {
                    log!("cloud: connection lost: {e}");
                    self.set_key(
                        &k,
                        LinkState::Connecting,
                        Some("Reconnecting…".into()),
                        &*on_change,
                    );
                }
            }
            if started.elapsed() > STABLE {
                backoff = BACKOFF_MIN;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    }

    /// One connection, until it ends.
    async fn session(
        &self,
        k: &str,
        id: &Identity,
        tx: &mpsc::Sender<Incoming>,
        on_change: &(dyn Fn() + Send + Sync),
    ) -> End {
        let conn = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async(&self.cfg.ws_url),
        )
        .await;
        let mut ws = match conn {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(e)) => return End::Lost(e.to_string()),
            Err(_) => return End::Lost("connect timeout".into()),
        };
        if let Err(e) = ws.send(WsMsg::text(hello(k, id))).await {
            return End::Lost(e.to_string());
        }
        log!("cloud: connected");
        self.set_key(k, LinkState::Connected, None, on_change);
        let probe = probe_h(k);
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
        // First probe soon after hello (the relay processes hello asynchronously).
        let mut probe_tick = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(2),
            PROBE_EVERY,
        );
        let mut last_probe_seen = tokio::time::Instant::now();
        loop {
            tokio::select! {
                msg = ws.next() => {
                    let text = match msg {
                        Some(Ok(WsMsg::Text(t))) => t.to_string(),
                        Some(Ok(WsMsg::Close(_))) | None => return End::Lost("closed by relay".into()),
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => return End::Lost(e.to_string()),
                    };
                    match parse(&text) {
                        Some(Relay::State { s, h }) if h == probe => {
                            last_probe_seen = tokio::time::Instant::now();
                            let first = {
                                let mut st = self.status.lock().unwrap();
                                let mut first = false;
                                if let Some(x) = st.keys.iter_mut().find(|x| x.key == k) {
                                    first = x.state != LinkState::Working;
                                    x.verified_at = Some(hn_sim::now_ms());
                                }
                                Self::summarize(&mut st);
                                first
                            };
                            if first {
                                log!("cloud: probe came back on key {}, link works", &k[..8]);
                            }
                            self.set_key(k, LinkState::Working, None, on_change);
                            let _ = s;
                        }
                        Some(Relay::State { s, h }) => {
                            let ev = Event {
                                v: 1,
                                agent: AGENT.into(),
                                session: h,
                                state: s,
                                ts: hn_sim::now_ms(),
                                ttl: Some(SESSION_TTL_S),
                                proc: None,
                            };
                            let _ = tx.send(Incoming { msg: Message::Event(ev), reply: None }).await;
                        }
                        Some(Relay::Err(e)) if e == "key_full" => return End::KeyFull,
                        Some(Relay::Err(e)) => return End::Refused(e),
                        None => {}
                    }
                }
                _ = ping.tick() => {
                    if let Err(e) = ws.send(WsMsg::text(r#"{"t":"ping"}"#)).await {
                        return End::Lost(e.to_string());
                    }
                }
                _ = probe_tick.tick() => {
                    // Two probes in a row didn't come back: the path is broken; say so.
                    if last_probe_seen.elapsed() > PROBE_EVERY * 2 + Duration::from_secs(30) {
                        self.set_key(k, LinkState::Connected, Some("Connected, but test messages aren't coming back.".into()), on_change);
                    }
                    let url = format!("{}/v1/h/{k}", self.cfg.post_url.trim_end_matches('/'));
                    let body = json!({"v": 1, "s": "idle", "h": probe, "ts": hn_sim::now_ms(), "hb": true});
                    tokio::spawn(async move {
                        let c = reqwest::Client::builder().timeout(Duration::from_secs(10)).build();
                        if let Ok(c) = c {
                            if let Err(e) = c.post(&url).header("content-type", "application/json").body(body.to_string()).send().await {
                                log!("cloud: probe post failed: {e}");
                            }
                        }
                    });
                }
            }
        }
    }
}

enum End {
    KeyFull,
    Refused(String),
    Lost(String),
}

fn hello(k: &str, id: &Identity) -> String {
    json!({"t": "hello", "k": k, "i": id.i, "s": id.s}).to_string()
}

#[derive(Debug, PartialEq, Eq)]
enum Relay {
    State { s: AgentState, h: String },
    Err(String),
}

fn parse(text: &str) -> Option<Relay> {
    let v: Value = serde_json::from_str(text).ok()?;
    match v["t"].as_str()? {
        "state" => {
            let s = AgentState::parse(v["s"].as_str()?)?;
            let h = v["h"].as_str()?;
            let ok = !h.is_empty() && h.len() <= 64 && h.chars().all(|c| c.is_ascii_alphanumeric());
            ok.then(|| Relay::State {
                s,
                h: h.to_string(),
            })
        }
        "err" => Some(Relay::Err(v["e"].as_str().unwrap_or("error").to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_key_matches_the_cloud_hook() {
        // printf '%s' "humanneeded-account:<uuid>" | sha256sum | cut -c1-32
        let k = route_key("00000000-1111-2222-3333-444444444444");
        assert_eq!(k.len(), 32);
        let d = Sha256::digest(b"humanneeded-account:00000000-1111-2222-3333-444444444444");
        assert_eq!(k, hex(&d)[..32]);
        assert_eq!(probe_h(&k).len(), 16);
    }

    #[test]
    fn identity_sizes() {
        let id = Identity::new();
        assert_eq!(id.i.len(), 32);
        assert_eq!(id.s.len(), 64);
        assert_ne!(id, Identity::new());
    }

    #[test]
    fn reads_account_uuid() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(".claude.json");
        std::fs::write(
            &p,
            r#"{"oauthAccount":{"accountUuid":"ab-12","emailAddress":"x"}}"#,
        )
        .unwrap();
        assert_eq!(account_uuid(&p).as_deref(), Some("ab-12"));
        std::fs::write(&p, r#"{"oauthAccount":{"accountUuid":"bad uuid!"}}"#).unwrap();
        assert_eq!(account_uuid(&p), None);
        std::fs::write(&p, "{}").unwrap();
        assert_eq!(account_uuid(&p), None);
    }

    #[test]
    fn reads_every_account_desktop_first() {
        let d = tempfile::tempdir().unwrap();
        let cj = d.path().join(".claude.json");
        std::fs::write(&cj, r#"{"oauthAccount":{"accountUuid":"bfe1829d-0001"}}"#).unwrap();
        // PC2: the Store build keeps its config under a per-package folder.
        let msix = d.path().join("Claude-Data/claude_pzs8sxrjxfjjc");
        std::fs::create_dir_all(&msix).unwrap();
        std::fs::write(
            msix.join("config.json"),
            r#"{"lastKnownAccountUuid":"81c9fbe3-0002","other":1}"#,
        )
        .unwrap();
        // A second Desktop with the same account as ~/.claude.json: listed once.
        let reg = d.path().join("Claude");
        std::fs::create_dir_all(&reg).unwrap();
        std::fs::write(
            reg.join("config.json"),
            r#"{"lastKnownAccountUuid":"bfe1829d-0001"}"#,
        )
        .unwrap();
        let files = vec![
            d.path().join("Claude/config.json"),
            d.path().join("Claude-Data/*/config.json"),
            d.path().join("missing/*/config.json"),
            cj,
        ];
        assert_eq!(accounts(&files), ["bfe1829d-0001", "81c9fbe3-0002"]);
        let only_msix = vec![d.path().join("Claude-Data/claude_*/config.json")];
        assert_eq!(accounts(&only_msix), ["81c9fbe3-0002"]);
        assert!(accounts(&[d.path().join("Claude-Data/other_*/config.json")]).is_empty());
    }

    #[test]
    fn parses_relay_messages() {
        assert_eq!(
            parse(r#"{"t":"state","s":"needs","h":"0123456789abcdef","ts":1,"hb":false}"#),
            Some(Relay::State {
                s: AgentState::Needs,
                h: "0123456789abcdef".into()
            })
        );
        assert_eq!(
            parse(r#"{"t":"err","e":"key_full"}"#),
            Some(Relay::Err("key_full".into()))
        );
        assert_eq!(parse(r#"{"t":"state","s":"bogus","h":"ab"}"#), None);
        assert_eq!(parse(r#"{"t":"state","s":"done","h":"a/b"}"#), None);
        assert_eq!(parse("nope"), None);
    }
}
