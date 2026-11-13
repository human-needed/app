//! One-click firmware update (D-037, D-044): `latest.json` → download → SHA-256 → `BOOTSEL` →
//! copy the .uf2 to the RPI-RP2 drive → wait for the puck to come back with the new version.
//!
//! `latest.json`: `{ "<board>": { "version", "file", "url", "sha256" } }`. An entry without a
//! SHA-256 is refused (D-039).

use crate::engine::Engine;
use crate::link::LinkEvent;
use crate::log;
use hn_core::proto::Hello;
use hn_core::version::is_newer;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_UF2: usize = 8 * 1024 * 1024;
const UF2_MAGIC: [u8; 8] = [0x55, 0x46, 0x32, 0x0A, 0x57, 0x51, 0x5D, 0x9E];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub version: String,
    pub file: String,
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Available {
    pub current: String,
    pub latest: Option<Entry>,
    pub newer: bool,
}

async fn fetch(url: &str, max: usize) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        let path = path.trim_start_matches('/');
        let p = if cfg!(windows) {
            PathBuf::from(path)
        } else {
            PathBuf::from(format!("/{path}"))
        };
        return std::fs::read(&p).map_err(|e| format!("{url}: {e}"));
    }
    if !url.starts_with("https://") && !url.starts_with("http://127.0.0.1") {
        return Err(format!("refusing non-https url {url}"));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("HumanNeeded-app/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if bytes.len() > max {
        return Err("file too large".into());
    }
    Ok(bytes.to_vec())
}

pub fn parse_latest(json: &[u8], board: &str) -> Result<Option<Entry>, String> {
    let v: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("bad latest.json: {e}"))?;
    match v.get(board) {
        None => Ok(None),
        Some(e) => serde_json::from_value(e.clone())
            .map(Some)
            .map_err(|e| format!("bad latest.json entry: {e}")),
    }
}

pub async fn check(latest_url: &str, hello: &Hello) -> Result<Available, String> {
    let json = fetch(latest_url, 64 * 1024).await?;
    let latest = parse_latest(&json, &hello.board)?;
    let newer = latest
        .as_ref()
        .is_some_and(|e| is_newer(&e.version, &hello.fw));
    Ok(Available {
        current: hello.fw.clone(),
        latest,
        newer,
    })
}

pub fn verify(data: &[u8], sha256: Option<&str>) -> Result<(), String> {
    let expected =
        sha256.ok_or("latest.json has no SHA-256 for this firmware; not installing it")?;
    let actual: String = Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(format!(
            "SHA-256 mismatch: expected {expected}, got {actual}"
        ));
    }
    if data.len() < 512 || data[..8] != UF2_MAGIC {
        return Err("not a UF2 file".into());
    }
    Ok(())
}

/// The RPI-RP2 drive: a volume whose INFO_UF2.TXT says `RPI-RP2`.
pub fn find_rp2_drive(fake: Option<&Path>) -> Option<PathBuf> {
    let is_rp2 = |dir: &Path| {
        std::fs::read_to_string(dir.join("INFO_UF2.TXT"))
            .map(|t| t.contains("RPI-RP2"))
            .unwrap_or(false)
    };
    if let Some(dir) = fake {
        return is_rp2(dir).then(|| dir.to_path_buf());
    }
    candidate_drives().into_iter().find(|d| is_rp2(d))
}

#[cfg(windows)]
fn candidate_drives() -> Vec<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    const DRIVE_REMOVABLE: u32 = 2;
    let mask = unsafe { GetLogicalDrives() };
    (2..26u32) // skip A: and B:
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let root = format!("{}:\\", (b'A' + i as u8) as char);
            let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
            (unsafe { GetDriveTypeW(wide.as_ptr()) } == DRIVE_REMOVABLE)
                .then(|| PathBuf::from(root))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn candidate_drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/Volumes/RPI-RP2")]
}

#[cfg(all(unix, not(target_os = "macos")))]
fn candidate_drives() -> Vec<PathBuf> {
    let user = std::env::var("USER").unwrap_or_default();
    vec![
        PathBuf::from(format!("/media/{user}/RPI-RP2")),
        PathBuf::from(format!("/run/media/{user}/RPI-RP2")),
    ]
}

pub(crate) async fn update(
    engine: &Engine,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> Result<String, String> {
    let step = |s: &str| {
        log!("firmware: {s}");
        progress(s);
    };
    let puck = engine.puck_info().ok_or("puck not connected")?;
    step("Checking for the latest firmware");
    let avail = check(&engine.config.latest_url, &puck.hello).await?;
    let entry = avail.latest.ok_or(format!(
        "no firmware published for board {}",
        puck.hello.board
    ))?;
    if !avail.newer {
        return Err(format!("already up to date ({})", puck.hello.fw));
    }
    step(&format!("Downloading {}", entry.file));
    let data = fetch(&entry.url, MAX_UF2).await?;
    verify(&data, entry.sha256.as_deref())?;
    step("Checksum OK; restarting the puck into update mode");
    let mut events = engine.link_events();
    engine.set_updating(Some(format!("installing {}", entry.version)));
    let result = install(engine, &entry, &data, &mut events, &step).await;
    engine.set_updating(None);
    result
}

async fn install(
    engine: &Engine,
    entry: &Entry,
    data: &[u8],
    events: &mut tokio::sync::broadcast::Receiver<LinkEvent>,
    step: &(dyn Fn(&str) + Send + Sync),
) -> Result<String, String> {
    engine
        .puck("BOOTSEL")
        .await
        .map_err(|e| format!("BOOTSEL failed: {e}"))?;
    let fake = engine.config.uf2_drive.clone();
    step("Waiting for the RPI-RP2 drive");
    let mut drive = None;
    for _ in 0..300 {
        if let Some(d) = find_rp2_drive(fake.as_deref()) {
            drive = Some(d);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let drive = drive.ok_or("the RPI-RP2 drive didn't appear (unplug the puck, hold its button while plugging it in, then try again)")?;
    let name = Path::new(&entry.file)
        .file_name()
        .ok_or("bad file name")?
        .to_owned();
    step(&format!("Copying firmware to {}", drive.display()));
    let target = drive.join(name);
    let bytes = data.to_vec();
    tokio::task::spawn_blocking(move || std::fs::write(&target, bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("copy failed: {e}"))?;
    step("Waiting for the puck to restart");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let ev = tokio::time::timeout_at(deadline, events.recv())
            .await
            .map_err(|_| "the puck didn't come back after the update".to_string())?;
        if let Ok(LinkEvent::Connected { hello, .. }) = ev {
            if hello.fw == entry.version {
                step(&format!("Updated to {}", entry.version));
                return Ok(entry.version.clone());
            }
            return Err(format!(
                "puck came back with {} instead of {}",
                hello.fw, entry.version
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn fake_uf2() -> Vec<u8> {
        let mut d = UF2_MAGIC.to_vec();
        d.resize(512, 0);
        d
    }

    fn sha(d: &[u8]) -> String {
        Sha256::digest(d)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn verify_checks_sha_and_magic() {
        let d = fake_uf2();
        assert!(verify(&d, Some(&sha(&d))).is_ok());
        assert!(verify(&d, None).is_err());
        assert!(verify(&d, Some(&"0".repeat(64))).is_err());
        let junk = vec![1u8; 600];
        assert!(verify(&junk, Some(&sha(&junk)))
            .unwrap_err()
            .contains("UF2"));
    }

    #[test]
    fn latest_json() {
        let j = br#"{"hn-v2":{"version":"2.0.1","file":"a-2.0.1.uf2","url":"https://x/a.uf2","sha256":"ab"}}"#;
        assert_eq!(parse_latest(j, "hn-v2").unwrap().unwrap().version, "2.0.1");
        assert!(parse_latest(j, "other").unwrap().is_none());
        // Today's file has no sha256: parsed, but `verify` will refuse it.
        let old = br#"{"hn-v1":{"version":"0.3.0","file":"f.uf2","url":"https://x"}}"#;
        assert_eq!(parse_latest(old, "hn-v1").unwrap().unwrap().sha256, None);
    }

    #[test]
    fn finds_fake_drive() {
        let d = tempfile::tempdir().unwrap();
        assert!(find_rp2_drive(Some(d.path())).is_none());
        std::fs::write(d.path().join("INFO_UF2.TXT"), "Board-ID: RPI-RP2\n").unwrap();
        assert!(find_rp2_drive(Some(d.path())).is_some());
    }
}
