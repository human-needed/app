// SPDX-License-Identifier: FSL-1.1-ALv2
//! HumanNeeded puck simulator.
//!
//! [`SimPuck`] is the protocol state machine; [`Sim`] runs it behind a byte stream (TCP for a
//! separate process, or [`Sim::connect_in_process`] for tests and the app's "simulator" mode).
//! It sends `!HELLO` on every connect, drops the table on disconnect (USB unmount), and on
//! `BOOTSEL` becomes a fake RPI-RP2 drive folder (when one is configured) that "installs" the
//! first `.uf2` copied into it, taking the new version from the file name (`…-X.Y.Z.uf2`).

pub mod machine;
pub mod puck;

use puck::Pending;
pub use puck::SimPuck;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{watch, Notify};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct Sim {
    pub puck: Arc<Mutex<SimPuck>>,
    shown_tx: Arc<watch::Sender<String>>,
    /// Fake RPI-RP2 drive folder used after BOOTSEL. `None` = BOOTSEL just reboots.
    pub uf2_dir: Option<PathBuf>,
    /// Bumped whenever the current connection must be dropped (reboot/bootsel).
    kick: Arc<Notify>,
    available: Arc<watch::Sender<bool>>,
}

impl Sim {
    pub fn new(fw: &str, uf2_dir: Option<PathBuf>) -> Self {
        let (shown_tx, _) = watch::channel("idle".to_string());
        let (available, _) = watch::channel(true);
        let sim = Self {
            puck: Arc::new(Mutex::new(SimPuck::new(fw))),
            shown_tx: Arc::new(shown_tx),
            uf2_dir,
            kick: Arc::new(Notify::new()),
            available: Arc::new(available),
        };
        // Lease/flash timers run even with no host attached.
        let s = sim.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                s.publish();
            }
        });
        sim
    }

    /// What the ring shows right now ("idle", "working", "needs", "done", "test:…", "dark",
    /// "bootloader").
    pub fn shown(&self) -> String {
        self.shown_tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<String> {
        self.shown_tx.subscribe()
    }

    pub fn fw(&self) -> String {
        self.puck.lock().unwrap().fw.clone()
    }

    /// False while "in the bootloader" or rebooting (the USB serial device is gone).
    pub fn available(&self) -> bool {
        *self.available.borrow()
    }

    fn publish(&self) {
        let shown = if self.available() {
            self.puck.lock().unwrap().shown(now_ms())
        } else {
            "bootloader".into()
        };
        self.shown_tx.send_if_modified(|cur| {
            if *cur != shown {
                *cur = shown;
                true
            } else {
                false
            }
        });
    }

    /// Serves one host connection until it closes or the puck reboots.
    pub async fn serve<S: AsyncRead + AsyncWrite + Unpin>(&self, stream: S) {
        if !self.available() {
            return;
        }
        let (r, mut w) = tokio::io::split(stream);
        let mut lines = BufReader::new(r).lines();
        {
            let mut p = self.puck.lock().unwrap();
            p.reset_link();
        }
        self.publish();
        if w.write_all(b"!HELLO\r\n").await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { break };
                    let (reply, pending) = {
                        let mut p = self.puck.lock().unwrap();
                        let reply = p.handle(line.trim_end_matches('\r'), now_ms());
                        (reply, p.pending.take())
                    };
                    self.publish();
                    if w.write_all(format!("{reply}\r\n").as_bytes()).await.is_err() {
                        break;
                    }
                    let _ = w.flush().await;
                    if let Some(p) = pending {
                        self.restart(p);
                        break;
                    }
                }
                _ = self.kick.notified() => break,
            }
        }
        // USB unmount: table cleared, resting light.
        self.puck.lock().unwrap().reset_link();
        self.publish();
    }

    fn restart(&self, why: Pending) {
        self.available.send_replace(false);
        self.publish();
        let me = self.clone();
        tokio::spawn(async move {
            if why == Pending::Bootsel {
                if let Some(dir) = me.uf2_dir.clone() {
                    me.fake_bootloader(dir).await;
                } else {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                }
            } else {
                tokio::time::sleep(Duration::from_millis(800)).await;
            }
            me.puck.lock().unwrap().reset_link();
            me.available.send_replace(true);
            me.publish();
        });
    }

    async fn fake_bootloader(&self, dir: PathBuf) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("INFO_UF2.TXT"),
            "UF2 Bootloader v3.0\r\nModel: Raspberry Pi RP2\r\nBoard-ID: RPI-RP2\r\n",
        );
        // Wait (up to 2 min) for a .uf2, like the ROM bootloader.
        for _ in 0..1200 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("uf2") {
                    tokio::time::sleep(Duration::from_millis(300)).await; // let the copy finish
                    if let Some(v) = version_from_name(&path) {
                        self.puck.lock().unwrap().fw = v;
                    }
                    let _ = std::fs::remove_dir_all(&dir);
                    return;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Accepts host connections on a TCP address, one at a time (one USB host).
    pub async fn listen_tcp(&self, addr: &str) -> std::io::Result<std::net::SocketAddr> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let _ = stream.set_nodelay(true);
                if !me.available() {
                    drop(stream); // "no such device" while in the bootloader
                    continue;
                }
                // A new host replaces the old one.
                me.kick.notify_waiters();
                let m = me.clone();
                tokio::spawn(async move { m.serve(stream).await });
            }
        });
        Ok(local)
    }

    /// An in-process "USB cable": returns the host end of a fresh connection.
    pub fn connect_in_process(&self) -> Option<tokio::io::DuplexStream> {
        if !self.available() {
            return None;
        }
        self.kick.notify_waiters();
        let (host, device) = tokio::io::duplex(4096);
        let me = self.clone();
        tokio::spawn(async move { me.serve(device).await });
        Some(host)
    }
}

fn version_from_name(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let v = stem.rsplit('-').next()?;
    (v.split('.').count() == 3 && v.split('.').all(|p| p.parse::<u32>().is_ok()))
        .then(|| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_names() {
        assert_eq!(
            version_from_name(std::path::Path::new("humanneeded-hn-v2-2.0.1.uf2")).as_deref(),
            Some("2.0.1")
        );
        assert_eq!(version_from_name(std::path::Path::new("x.uf2")), None);
    }

    #[tokio::test]
    async fn in_process_hello_and_snap() {
        let sim = Sim::new("2.0.0", None);
        let host = sim.connect_in_process().unwrap();
        let (r, mut w) = tokio::io::split(host);
        let mut lines = BufReader::new(r).lines();
        assert_eq!(lines.next_line().await.unwrap().unwrap(), "!HELLO");
        w.write_all(b"SNAP 30 0000000a:needs\n").await.unwrap();
        assert_eq!(lines.next_line().await.unwrap().unwrap(), "OK");
        let mut rx = sim.subscribe();
        tokio::time::timeout(Duration::from_secs(2), rx.wait_for(|s| s == "needs"))
            .await
            .unwrap()
            .unwrap();
        drop(w);
        drop(lines);
        tokio::time::timeout(Duration::from_secs(2), rx.wait_for(|s| s == "idle"))
            .await
            .unwrap()
            .unwrap();
    }
}
