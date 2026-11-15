//! HumanNeeded tray app (D-027, D-031, D-055): one per user, starts at login, the only writer to
//! the puck. Tray only: no main window, one panel that opens from the tray / menu-bar icon. The
//! engine (hn-engine) does the work; this file wires the tray, the panel and the commands the
//! panel calls.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hn_install;
mod panel;
mod tray;
mod updater;

use hn_engine::{agents, log, paths, quiet, Config, Engine};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

type R<T> = Result<T, String>;

/// The panel changed a puck setting: the tray re-reads the colours.
pub static COLOURS_CHANGED: tokio::sync::Notify = tokio::sync::Notify::const_new();

fn json_reply(r: R<String>) -> R<Value> {
    let line = r?;
    serde_json::from_str(&line).map_err(|_| format!("unexpected reply: {line}"))
}

#[tauri::command]
async fn status(
    engine: State<'_, Engine>,
    updates: State<'_, std::sync::Arc<updater::Updates>>,
) -> R<Value> {
    let mut st = engine.status().await;
    st["update"] = serde_json::to_value(updates.status()).unwrap_or_default();
    Ok(st)
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

#[tauri::command]
async fn puck_schema(engine: State<'_, Engine>) -> R<Value> {
    json_reply(engine.puck("SCHEMA").await)
}

#[tauri::command]
async fn puck_settings(engine: State<'_, Engine>) -> R<Value> {
    json_reply(engine.puck("GET all").await)
}

fn safe_token(s: &str) -> R<&str> {
    if s.is_empty() || s.len() > 64 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("bad value".into());
    }
    Ok(s)
}

#[tauri::command]
async fn puck_set(engine: State<'_, Engine>, key: String, value: String) -> R<()> {
    engine
        .puck(format!("SET {} {}", safe_token(&key)?, safe_token(&value)?))
        .await?;
    COLOURS_CHANGED.notify_one();
    Ok(())
}

#[tauri::command]
async fn puck_action(engine: State<'_, Engine>, action: String, arg: Option<String>) -> R<()> {
    let line = match (action.as_str(), arg.as_deref()) {
        ("test", Some(st)) if ["needs", "done", "working", "idle", "hello"].contains(&st) => {
            format!("TEST {st}")
        }
        ("mute", Some(m)) => format!("MUTE {}", safe_token(m)?),
        ("reset", None) => {
            engine.puck("RESET").await?;
            // Levels live on the master (D-055): put the per-state ones back to 100.
            engine.full_levels().await;
            COLOURS_CHANGED.notify_one();
            return Ok(());
        }
        ("reboot", None) => "REBOOT".into(),
        _ => return Err("unknown action".into()),
    };
    engine.puck(line).await.map(|_| ())
}

#[tauri::command]
async fn cloud_set(engine: State<'_, Engine>, on: bool) -> R<()> {
    engine.cloud_set(on).await
}

/// Per-agent "lights the puck" switch (D-055).
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

#[tauri::command]
fn autostart_get(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn autostart_set(app: AppHandle, on: bool) -> R<()> {
    let a = app.autolaunch();
    if on { a.enable() } else { a.disable() }.map_err(|e| e.to_string())
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
    log::init(paths::data_dir().join("logs"));
    let first_run = !paths::data_dir().join("seen").exists();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            panel::show(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            status,
            puck_schema,
            puck_settings,
            puck_set,
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
            autostart_get,
            autostart_set,
            open_log,
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
            if first_run && !dev {
                let _ = std::fs::create_dir_all(paths::data_dir());
                let _ = std::fs::write(paths::data_dir().join("seen"), "1");
                let _ = app.autolaunch().enable(); // starts at login by default (D-027)
            }
            updater::start(app.handle());
            panel::init(app.handle())?;
            tray::build(app.handle(), engine.clone())?;
            // Started by hand (not at sign-in): show the panel once, so people see where it is.
            let background = std::env::args().any(|a| a == "--background");
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
