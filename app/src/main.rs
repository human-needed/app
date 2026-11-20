// SPDX-License-Identifier: FSL-1.1-ALv2
//! HumanNeeded tray app (D-027, D-031, D-055): one per user, starts at login, the only writer to
//! the puck. Tray only: no main window, one panel that opens from the tray / menu-bar icon. The
//! engine (hn-engine) does the work; this file wires the tray, the panel and the commands the
//! panel calls.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod glow;
mod hn_install;
mod login;
mod panel;
mod sound;
mod tray;
mod uninstall;
mod updater;

use hn_engine::{agents, alerts, diagnostics, log, paths, quiet, Config, Engine};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

type R<T> = Result<T, String>;

/// The panel changed the alert settings: the tray re-reads the colours.
pub static COLOURS_CHANGED: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[tauri::command]
async fn status(
    app: AppHandle,
    engine: State<'_, Engine>,
    updates: State<'_, std::sync::Arc<updater::Updates>>,
) -> R<Value> {
    let mut st = engine.status().await;
    st["update"] = serde_json::to_value(updates.status()).unwrap_or_default();
    st["glow"] = serde_json::to_value(glow::options(&app)).unwrap_or_default();
    st["welcome"] = welcome_file().exists().into();
    Ok(st)
}

/// "Copy diagnostics" (Settings → Advanced): a summary to paste into a bug report.
#[tauri::command]
async fn diagnostics(
    app: AppHandle,
    engine: State<'_, Engine>,
    updates: State<'_, std::sync::Arc<updater::Updates>>,
) -> R<String> {
    let st = status(app, engine, updates).await?;
    let home = dirs_home();
    Ok(diagnostics::report(
        &st,
        &diagnostics::os(),
        &diagnostics::log_tail(),
        home.as_deref(),
    ))
}

fn dirs_home() -> Option<String> {
    std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).ok()
}

/// What this glow window shows now (the glow page asks when it loads). Async, so it never
/// runs on the main thread (which may be busy making windows).
#[tauri::command]
async fn glow_spec(app: AppHandle, window: tauri::Window) -> Result<glow::Spec, ()> {
    Ok(glow::current(&app, window.label()))
}

/// Screen-edge options: paused or not, screens, thickness, skip full-screen apps.
#[tauri::command]
fn glow_set(app: AppHandle, options: glow::Options) -> R<()> {
    glow::set_options(&app, options)?;
    let _ = app.emit("changed", ());
    Ok(())
}

/// The alert settings, with each state's defaults (for the "Default" button).
#[tauri::command]
fn alerts_get(engine: State<'_, Engine>) -> Value {
    serde_json::json!({
        "alerts": engine.alerts_get(),
        "defaults": alerts::Alerts::default().states,
    })
}

#[tauri::command]
async fn alerts_set(engine: State<'_, Engine>, alerts: alerts::Alerts) -> R<()> {
    engine.alerts_set(alerts).await?;
    COLOURS_CHANGED.notify_one();
    Ok(())
}

#[tauri::command]
fn update_check(updates: State<'_, std::sync::Arc<updater::Updates>>) {
    updates.check_now();
}

#[tauri::command]
fn update_auto(updates: State<'_, std::sync::Arc<updater::Updates>>, on: bool) -> R<()> {
    updates.set_auto(on)
}

#[tauri::command]
async fn update_install(app: AppHandle) -> R<()> {
    updater::install(&app).await
}

fn safe_token(s: &str) -> R<&str> {
    if s.is_empty() || s.len() > 64 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("bad value".into());
    }
    Ok(s)
}

#[tauri::command]
async fn puck_action(engine: State<'_, Engine>, action: String, arg: Option<String>) -> R<()> {
    let line = match (action.as_str(), arg.as_deref()) {
        ("test", Some(st)) if ["needs", "done", "working", "idle", "hello"].contains(&st) => {
            format!("TEST {st}")
        }
        ("mute", Some(m)) => format!("MUTE {}", safe_token(m)?),
        // The Puck page's Test: the puck's own hello light, nothing on the screen.
        ("puck_test", None) => return engine.puck_only("TEST hello").await.map(|_| ()),
        ("reboot", None) => "REBOOT".into(),
        _ => return Err("unknown action".into()),
    };
    engine.puck(line).await.map(|_| ())
}

#[tauri::command]
async fn cloud_set(engine: State<'_, Engine>, on: bool) -> R<()> {
    engine.cloud_set(on).await
}

/// Per-agent switch (D-055): its events alert you or not.
#[tauri::command]
fn agent_set(engine: State<'_, Engine>, id: String, on: bool) -> R<()> {
    engine.agent_set(&id, on)
}

#[tauri::command]
async fn quiet_set(engine: State<'_, Engine>, enabled: bool, from: String, to: String) -> R<()> {
    engine
        .quiet_set(quiet::Settings { enabled, from, to })
        .await
}

/// The panel's content height (CSS px).
#[tauri::command]
fn panel_fit(app: AppHandle, height: f64) {
    panel::fit(&app, height);
}

#[tauri::command]
fn panel_hide(app: AppHandle) {
    if let Some(w) = app.get_webview_window(panel::LABEL) {
        let _ = w.hide();
    }
}

#[tauri::command]
async fn clear_sessions(engine: State<'_, Engine>) -> R<()> {
    engine.request("clear", &[]).await;
    Ok(())
}

#[tauri::command]
async fn fw_check(engine: State<'_, Engine>) -> R<Value> {
    let a = engine.check_update().await?;
    Ok(serde_json::to_value(a).unwrap_or_default())
}

#[tauri::command]
async fn fw_update(app: AppHandle, engine: State<'_, Engine>) -> R<String> {
    let engine = engine.inner().clone();
    engine
        .update_firmware(move |s| {
            let _ = app.emit("fw-progress", s.to_string());
        })
        .await
}

/// D-054: the user installs the plugins; this opens the per-agent guides (fixed URL).
#[tauri::command]
fn open_setup_guide() -> R<()> {
    open_path(std::path::Path::new(agents::SETUP_URL))
}

/// "Don't have one? See the Puck" (fixed URL).
#[tauri::command]
fn open_puck_page() -> R<()> {
    open_path(std::path::Path::new(PUCK_URL))
}

/// Where the puck is described (WS-WEB).
const PUCK_URL: &str = "https://human-needed.com/";

#[tauri::command]
fn autostart_get() -> bool {
    login::is_enabled()
}

#[tauri::command]
fn autostart_set(on: bool) -> R<()> {
    login::set(on)
}

/// R-01: the first run shows the welcome screen until the user finishes it.
fn welcome_file() -> std::path::PathBuf {
    paths::data_dir().join("welcome-pending")
}

#[tauri::command]
fn welcome_done() {
    let _ = std::fs::remove_file(welcome_file());
    log!("welcome: done");
}

/// Settings → Advanced → Uninstall (R-05).
#[tauri::command]
fn uninstall(app: AppHandle, keep_settings: bool) -> R<()> {
    uninstall::start(&app, keep_settings)
}

#[tauri::command]
fn open_log() -> R<()> {
    let path = log::path().ok_or("no log")?;
    open_path(&path)
}

pub fn open_path(path: &std::path::Path) -> R<()> {
    #[cfg(windows)]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn main() {
    // The Windows uninstaller's clean-up (R-05): before anything else starts or writes a log.
    if uninstall::from_args() {
        return;
    }
    log::init(paths::data_dir().join("logs"));
    let first_run = !paths::data_dir().join("seen").exists();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            panel::show(app)
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            status,
            diagnostics,
            glow_spec,
            glow_set,
            alerts_get,
            alerts_set,
            puck_action,
            clear_sessions,
            cloud_set,
            agent_set,
            quiet_set,
            panel_fit,
            panel_hide,
            fw_check,
            fw_update,
            open_setup_guide,
            open_puck_page,
            autostart_get,
            autostart_set,
            open_log,
            welcome_done,
            uninstall,
            update_check,
            update_auto,
            update_install
        ])
        .setup(move |app| {
            // Menu-bar app: no Dock icon (D-055).
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let engine =
                tauri::async_runtime::block_on(async { Engine::start(Config::from_env()).await });
            let engine = match engine {
                Ok(e) => e,
                Err(e) => {
                    log!("can't start: {e}");
                    std::process::exit(1);
                }
            };
            app.manage(engine.clone());
            // HN_DEV=1 (development, tests): don't touch PATH or the login items.
            let dev = std::env::var_os("HN_DEV").is_some();
            // app-events 2.0 retired `hn`: remove what 1.0 builds put on PATH (skipped in dev).
            if !dev {
                std::thread::spawn(|| {
                    if let Some(m) = hn_install::retire() {
                        log!("hn: {m}");
                    }
                });
            }
            if first_run {
                let _ = std::fs::create_dir_all(paths::data_dir());
                let _ = std::fs::write(paths::data_dir().join("seen"), "1");
                // The welcome screen (R-01) shows until the user finishes it.
                let _ = std::fs::write(welcome_file(), "1");
                if !dev {
                    // Starts at sign-in by default (D-027).
                    match login::set(true) {
                        Ok(()) => log!("login item on"),
                        Err(e) => log!("login item: {e}"),
                    }
                }
            } else if !dev {
                login::refresh();
            }
            updater::start(app.handle());
            panel::init(app.handle())?;
            glow::init(app.handle());
            glow::start(app.handle(), &engine);
            sound::start(&engine);
            tray::build(app.handle(), engine.clone())?;
            // Started by hand (not at sign-in): show the panel once, so people see where it is.
            // An update restarts it too: stay in the background then.
            let updated = updater::just_updated(); // always read, so the marker goes
            let background = updated || std::env::args().any(|a| a == "--background");
            if !background {
                panel::show(app.handle());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|app, event| match event {
            // The panel hiding never quits the tray app.
            tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } => api.prevent_exit(),
            tauri::RunEvent::Exit => {
                let engine = app.state::<Engine>().inner().clone();
                tauri::async_runtime::block_on(engine.shutdown());
            }
            _ => {}
        });
}
