// SPDX-License-Identifier: GPL-3.0-or-later
//! App auto-update (Tauri updater plugin, signed with the project's updater key).
//!
//! - Checks 10 s after start and then every 6 h when automatic checks are on (default).
//! - A found update is downloaded in the background and its signature verified (the plugin
//!   refuses a package that doesn't match the public key in tauri.conf). Then the panel's
//!   Settings offer "Restart to update". Nothing restarts on its own.
//! - Offline or any error: logged only; tried again in 30 min.
//!
//! Debug builds only (local end-to-end tests): `HN_UPDATE_URL` points the check at another
//! `latest.json`; `HN_UPDATE_TEST_INSTALL=1` installs as soon as an update is ready.

use hn_engine::{log, paths, Engine};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Notify;

const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY: Duration = Duration::from_secs(30 * 60);
const FIRST: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    /// Automatic checks on (setting).
    pub auto: bool,
    pub checking: bool,
    /// Downloaded, verified and waiting for "Restart to update".
    pub ready: Option<String>,
    /// Result of the last check, for the window ("up to date", or why it failed).
    pub last: Option<String>,
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<Status>,
    pending: Mutex<Option<(Update, Vec<u8>)>>,
    wake: Notify,
}

pub fn auto_enabled() -> bool {
    let path = paths::data_dir().join("app-settings.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["auto_update"].as_bool())
        .unwrap_or(true)
}

fn save_auto(on: bool) -> Result<(), String> {
    let path = paths::data_dir().join("app-settings.json");
    let mut v = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    v["auto_update"] = serde_json::Value::Bool(on);
    std::fs::create_dir_all(paths::data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(&path, v.to_string()).map_err(|e| e.to_string())
}

impl Updates {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn set_auto(&self, on: bool) -> Result<(), String> {
        save_auto(on)?;
        self.status.lock().unwrap().auto = on;
        if on {
            self.wake.notify_one();
        }
        Ok(())
    }

    /// "Check for updates": checks now, whatever the setting.
    pub fn check_now(&self) {
        self.wake.notify_one();
    }
}

pub fn start(app: &AppHandle) {
    log!("app package {}", app.package_info().version);
    let u = Arc::new(Updates::default());
    u.status.lock().unwrap().auto = auto_enabled();
    app.manage(u.clone());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut wait = FIRST;
        loop {
            let manual = tokio::select! {
                _ = tokio::time::sleep(wait) => false,
                _ = u.wake.notified() => true,
            };
            if manual || u.status().auto {
                wait = match check(&app, &u).await {
                    Ok(()) => EVERY,
                    Err(e) => {
                        log!("update: {e}");
                        u.status.lock().unwrap().last = Some(format!("Couldn't check: {e}"));
                        RETRY
                    }
                };
                u.status.lock().unwrap().checking = false;
                let _ = app.emit("changed", ());
            } else {
                wait = EVERY;
            }
        }
    });
}

async fn check(app: &AppHandle, u: &Updates) -> Result<(), String> {
    if u.status().ready.is_some() {
        return Ok(()); // already waiting for a restart
    }
    u.status.lock().unwrap().checking = true;
    let _ = app.emit("changed", ());
    #[allow(unused_mut)]
    let mut b = app.updater_builder().timeout(Duration::from_secs(60));
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("HN_UPDATE_URL") {
        let url = url.parse().map_err(|e| format!("HN_UPDATE_URL: {e}"))?;
        b = b.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    let updater = b.build().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        u.status.lock().unwrap().last = Some("Up to date".into());
        return Ok(());
    };
    let version = update.version.clone();
    log!("update: {version} found, downloading");
    // download() verifies the signature against the embedded public key before returning.
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| format!("{version}: {e}"))?;
    log!("update: {version} downloaded and verified");
    *u.pending.lock().unwrap() = Some((update, bytes));
    {
        let mut s = u.status.lock().unwrap();
        s.ready = Some(version.clone());
        s.last = None;
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("HN_UPDATE_TEST_INSTALL").is_some() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = install(&app).await;
        });
    }
    Ok(())
}

/// Installs the downloaded update and restarts (the user asked for it).
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let u = app.state::<Arc<Updates>>().inner().clone();
    let Some((update, bytes)) = u.pending.lock().unwrap().take() else {
        return Err("no update is ready".into());
    };
    log!("update: installing {}", update.version);
    // Let the puck go cleanly first (its lease would expire anyway).
    if let Some(engine) = app.try_state::<Engine>() {
        engine.inner().clone().shutdown().await;
    }
    // Windows: starts the installer and exits. macOS/Linux: replaces the app, then we restart.
    if let Err(e) = update.install(&bytes) {
        // The engine is stopped: restart the current version rather than run without it.
        log!("update: install failed: {e}");
    }
    app.restart();
}
