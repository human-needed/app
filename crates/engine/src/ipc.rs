//! Local IPC server for `hn` (app-events §1). One JSON line per connection; requests get one
//! JSON line back. Only the current user can reach it (per-user pipe name / socket in a 0700
//! folder; remote pipe clients rejected).

use crate::log;
use hn_core::event::{parse_line, Message, MAX_LINE};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

pub struct Incoming {
    pub msg: Message,
    /// Present for requests: the engine's JSON answer.
    pub reply: Option<oneshot::Sender<String>>,
}

async fn handle<S: AsyncRead + AsyncWrite + Unpin>(stream: S, tx: mpsc::Sender<Incoming>) {
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r.take(MAX_LINE as u64 + 2));
    let mut line = String::new();
    match tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line)).await {
        Ok(Ok(n)) if n > 0 => {}
        _ => return,
    }
    let msg = match parse_line(line.trim_end()) {
        Ok(m) => m,
        Err(e) => {
            log!("ipc: rejected message: {e}");
            return;
        }
    };
    match msg {
        Message::Event(_) => {
            let _ = tx.send(Incoming { msg, reply: None }).await;
        }
        Message::Request(_) => {
            let (rtx, rrx) = oneshot::channel();
            if tx.send(Incoming { msg, reply: Some(rtx) }).await.is_err() {
                return;
            }
            if let Ok(Ok(answer)) = tokio::time::timeout(Duration::from_secs(6), rrx).await {
                let _ = w.write_all(format!("{answer}\n").as_bytes()).await;
                let _ = w.flush().await;
            }
        }
    }
}

/// Starts the server. Fails if another app instance already owns the endpoint.
#[cfg(windows)]
pub fn serve(endpoint: PathBuf, tx: mpsc::Sender<Incoming>) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = endpoint.to_string_lossy().into_owned();
    let first = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)?;
    tokio::spawn(async move {
        let mut server = first;
        loop {
            if server.connect().await.is_err() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let next = loop {
                match ServerOptions::new().reject_remote_clients(true).create(&name) {
                    Ok(s) => break s,
                    Err(e) => {
                        log!("ipc: can't create pipe instance: {e}");
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            };
            let client = std::mem::replace(&mut server, next);
            tokio::spawn(handle(client, tx.clone()));
        }
    });
    Ok(())
}

#[cfg(unix)]
pub fn serve(endpoint: PathBuf, tx: mpsc::Sender<Incoming>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = endpoint.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    if std::os::unix::net::UnixStream::connect(&endpoint).is_ok() {
        return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "app already running"));
    }
    let _ = std::fs::remove_file(&endpoint);
    let listener = tokio::net::UnixListener::bind(&endpoint)?;
    tokio::spawn(async move {
        loop {
            if let Ok((s, _)) = listener.accept().await {
                tokio::spawn(handle(s, tx.clone()));
            }
        }
    });
    Ok(())
}
