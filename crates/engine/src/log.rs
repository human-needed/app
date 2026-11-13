//! A small append-only log file (`<data>/logs/app.log`, trimmed at 1 MB). Also echoed to stderr.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
const MAX: u64 = 1024 * 1024;

pub fn init(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    *FILE.lock().unwrap() = Some(dir.join("app.log"));
}

pub fn path() -> Option<PathBuf> {
    FILE.lock().unwrap().clone()
}

pub fn write(msg: &str) {
    let ms = hn_sim::now_ms();
    let s = (ms / 1000) % 86_400;
    let line = format!(
        "{:02}:{:02}:{:02}.{:03}Z {msg}\n",
        s / 3600,
        (s / 60) % 60,
        s % 60,
        ms % 1000
    );
    eprint!("{line}");
    let guard = FILE.lock().unwrap();
    let Some(path) = guard.as_ref() else { return };
    if std::fs::metadata(path)
        .map(|m| m.len() > MAX)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log::write(&format!($($t)*)) };
}
