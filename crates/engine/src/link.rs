//! The puck link: the only owner of the puck's serial port (D-027).
//!
//! Finds the puck (VID 0x2E8A + product "HumanNeeded", confirmed by `HELLO`), runs one command
//! at a time (one reply line each, `!` lines accepted at any time), reconnects on its own, and
//! reports connects, disconnects and `!HELLO`.

use crate::log;
use hn_core::proto::{self, classify, Hello, Line, LineSplitter};
use hn_sim::Sim;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

const REPLY_TIMEOUT: Duration = Duration::from_millis(1500);
const RETRY: Duration = Duration::from_secs(2);

/// Where the puck is.
#[derive(Clone)]
pub enum PuckTarget {
    /// Find a real puck on USB serial.
    Auto,
    /// A specific serial port (e.g. `COM7`).
    Serial(String),
    /// A simulator over TCP (`hn-sim --listen …`).
    Tcp(String),
    /// A simulator inside this process.
    Sim(Sim),
    /// No puck at all.
    None,
}

impl PuckTarget {
    /// `auto` | `serial:COM7` | `tcp:127.0.0.1:47800` | `sim` | `none`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "auto" | "" => Self::Auto,
            "sim" => Self::Sim(Sim::new("2.0.0", None)),
            "none" => Self::None,
            _ if s.starts_with("serial:") => Self::Serial(s[7..].into()),
            _ if s.starts_with("tcp:") => Self::Tcp(s[4..].into()),
            _ => return None,
        })
    }
    pub fn describe(&self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Serial(p) => format!("serial:{p}"),
            Self::Tcp(a) => format!("tcp:{a}"),
            Self::Sim(_) => "sim".into(),
            Self::None => "none".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum LinkEvent {
    Connected { hello: Hello, port: String },
    Disconnected,
    /// The puck said `!HELLO` (it reset or re-enumerated): re-send state.
    Rehello,
}

type Reply = oneshot::Sender<Result<String, String>>;

#[derive(Clone)]
pub struct Link {
    tx: mpsc::Sender<(String, Option<Reply>)>,
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

impl Link {
    pub fn start(target: PuckTarget, events: mpsc::Sender<LinkEvent>) -> Self {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(run(target, rx, events));
        Self { tx }
    }

    /// Sends a command and waits for its reply line (`OK`, JSON, …). `ERR x` → `Err(x)`.
    pub async fn request(&self, line: impl Into<String>) -> Result<String, String> {
        let (rtx, rrx) = oneshot::channel();
        self.tx.send((line.into(), Some(rtx))).await.map_err(|_| "link stopped".to_string())?;
        rrx.await.map_err(|_| "link stopped".to_string())?
    }

    /// Queues a command without waiting (SNAP, FLASH). Order is kept.
    pub fn send(&self, line: impl Into<String>) {
        let _ = self.tx.try_send((line.into(), None));
    }
}

async fn run(target: PuckTarget, mut rx: mpsc::Receiver<(String, Option<Reply>)>, events: mpsc::Sender<LinkEvent>) {
    if matches!(target, PuckTarget::None) {
        while let Some((_, reply)) = rx.recv().await {
            if let Some(r) = reply {
                let _ = r.send(Err("no puck".into()));
            }
        }
        return;
    }
    let mut last_error = String::new();
    loop {
        match open(&target).await {
            Ok((stream, port, hello)) => {
                last_error.clear();
                log!("puck: connected on {port}: fw {} board {} proto {}", hello.fw, hello.board, hello.proto);
                let _ = events.send(LinkEvent::Connected { hello, port: port.clone() }).await;
                session(stream, &mut rx, &events).await;
                log!("puck: disconnected from {port}");
                let _ = events.send(LinkEvent::Disconnected).await;
            }
            Err(e) => {
                if e != last_error {
                    log!("puck: {e}");
                    last_error = e;
                }
            }
        }
        // While there's no puck, fail commands instead of letting them pile up.
        let deadline = tokio::time::Instant::now() + RETRY;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                cmd = rx.recv() => match cmd {
                    Some((_, Some(r))) => { let _ = r.send(Err("puck not connected".into())); }
                    Some((_, None)) => {}
                    None => return,
                },
            }
        }
    }
}

/// Opens the target and confirms it is a puck (`HELLO`).
async fn open(target: &PuckTarget) -> Result<(Box<dyn Stream>, String, Hello), String> {
    match target {
        PuckTarget::Tcp(addr) => {
            let s = tokio::net::TcpStream::connect(addr).await.map_err(|e| format!("tcp {addr}: {e}"))?;
            let _ = s.set_nodelay(true);
            handshake(Box::new(s), format!("tcp:{addr}")).await
        }
        PuckTarget::Sim(sim) => {
            let s = sim.connect_in_process().ok_or("simulator is rebooting")?;
            handshake(Box::new(s), "simulator".into()).await
        }
        PuckTarget::Serial(port) => handshake(open_serial(port)?, port.clone()).await,
        PuckTarget::Auto => {
            let candidates = find_pucks();
            if candidates.is_empty() {
                return Err("no puck found".into());
            }
            let mut errors = Vec::new();
            for port in candidates {
                match open_serial(&port) {
                    Ok(s) => match handshake(s, port.clone()).await {
                        Ok(ok) => return Ok(ok),
                        Err(e) => errors.push(format!("{port}: {e}")),
                    },
                    Err(e) => errors.push(format!("{port}: {e}")),
                }
            }
            Err(errors.join("; "))
        }
        PuckTarget::None => Err("no puck".into()),
    }
}

/// Serial ports that may be a puck: Raspberry Pi VID, "HumanNeeded" product first.
pub fn find_pucks() -> Vec<String> {
    let Ok(ports) = tokio_serial::available_ports() else { return vec![] };
    let mut named = Vec::new();
    let mut other = Vec::new();
    for p in ports {
        if let tokio_serial::SerialPortType::UsbPort(usb) = &p.port_type {
            if usb.vid != proto::VID {
                continue;
            }
            if usb.product.as_deref().is_some_and(|n| n.contains(proto::PRODUCT)) {
                named.push(p.port_name.clone());
            } else {
                other.push(p.port_name.clone());
            }
        }
    }
    named.extend(other);
    named
}

fn open_serial(port: &str) -> Result<Box<dyn Stream>, String> {
    use tokio_serial::SerialPort;
    let mut s = tokio_serial::SerialStream::open(&tokio_serial::new(port, 115_200).timeout(Duration::from_millis(100)))
        .map_err(|e| format!("can't open: {e} (in use by another program?)"))?;
    #[cfg(unix)]
    let _ = s.set_exclusive(true);
    let _ = s.write_data_terminal_ready(true);
    Ok(Box::new(s))
}

async fn handshake(mut s: Box<dyn Stream>, port: String) -> Result<(Box<dyn Stream>, String, Hello), String> {
    let mut split = LineSplitter::default();
    let reply = exchange(&mut s, &mut split, "HELLO", &mut |_| {}).await?;
    let hello = Hello::parse(&reply)?;
    Ok((s, port, hello))
}

/// Writes one command and returns its reply line; `!` lines are passed to `on_bang`.
async fn exchange(
    s: &mut Box<dyn Stream>,
    split: &mut LineSplitter,
    line: &str,
    on_bang: &mut (dyn FnMut(String) + Send),
) -> Result<String, String> {
    s.write_all(format!("{line}\n").as_bytes()).await.map_err(|e| format!("write: {e}"))?;
    s.flush().await.map_err(|e| format!("write: {e}"))?;
    let deadline = tokio::time::Instant::now() + REPLY_TIMEOUT;
    let mut buf = [0u8; 512];
    let mut pending: Vec<String> = Vec::new();
    loop {
        while !pending.is_empty() {
            let l = pending.remove(0);
            match classify(&l) {
                Line::Unsolicited(ev) => on_bang(ev),
                _ => return Ok(l),
            }
        }
        let n = tokio::time::timeout_at(deadline, s.read(&mut buf))
            .await
            .map_err(|_| "no reply".to_string())?
            .map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Err("closed".into());
        }
        pending.extend(split.push(&buf[..n]));
    }
}

async fn session(
    mut s: Box<dyn Stream>,
    rx: &mut mpsc::Receiver<(String, Option<Reply>)>,
    events: &mpsc::Sender<LinkEvent>,
) {
    let mut split = LineSplitter::default();
    let mut buf = [0u8; 512];
    let mut timeouts = 0;
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some((line, reply)) = cmd else { return };
                let ev = events.clone();
                let mut bangs = Vec::new();
                let result = exchange(&mut s, &mut split, &line, &mut |b| bangs.push(b)).await;
                for b in bangs {
                    if b == "HELLO" { let _ = ev.send(LinkEvent::Rehello).await; }
                }
                let fatal = matches!(&result, Err(e) if e != "no reply");
                if matches!(&result, Err(e) if e == "no reply") { timeouts += 1 } else { timeouts = 0 }
                let mapped = match result {
                    Ok(l) => match classify(&l) {
                        Line::Err(e) => Err(e),
                        _ => Ok(l),
                    },
                    Err(e) => Err(e),
                };
                if let Err(e) = &mapped {
                    if reply.is_none() { log!("puck: {line:?} failed: {e}"); }
                }
                if let Some(r) = reply { let _ = r.send(mapped); }
                if fatal || timeouts >= 3 { return; }
            }
            n = s.read(&mut buf) => {
                let n = match n { Ok(0) | Err(_) => return, Ok(n) => n };
                for l in split.push(&buf[..n]) {
                    if let Line::Unsolicited(ev) = classify(&l) {
                        if ev == "HELLO" { let _ = events.send(LinkEvent::Rehello).await; }
                    }
                }
            }
        }
    }
}
