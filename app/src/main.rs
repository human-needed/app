//! HumanNeeded tray app (D-027, D-031): one per user, starts at login, the only writer to the
//! puck. The engine (hn-engine) does the work; this file is the tray, the settings window and
//! the commands the window calls.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hn_install;
mod tray;

use hn_engine::{integrations, log, paths, Config, Engine};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

type R<T> = Result<T, String>;

fn json_reply(r: R<String>) -> R<Value> {
    let line = r?;
    serde_json::from_str(&line).map_err(|_| format!("unexpected reply: {line}"))
}

#[tauri::command]
async fn status(engine: State<'_, Engine>) -> R<Value> {
    Ok(engine.status().await)
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
        .await
        .map(|_| ())
}

#[tauri::command]
async fn puck_action(engine: State<'_, Engine>, action: String, arg: Option<String>) -> R<()> {
    let line = match (action.as_str(), arg.as_deref()) {
        ("test", Some(st)) if ["needs", "done", "working", "idle", "hello"].contains(&st) => {
            format!("TEST {st}")
        }
        ("mute", Some(m)) => format!("MUTE {}", safe_token(m)?),
        ("reset", None) => "RESET".into(),
        ("reboot", None) => "REBOOT".into(),
        _ => return Err("unknown action".into()),
    };
    engine.puck(line).await.map(|_| ())
}

#[tauri::command]
async fn cloud_set(engine: State<'_, Engine>, on: bool) -> R<()> {
    engine.cloud_set(on).await
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

#[tauri::command]
async fn integrations_list() -> R<Value> {
    let list =
        tauri::async_runtime::spawn_blocking(|| integrations::list(&integrations::Env::current()))
            .await
            .map_err(|e| e.to_string())?;
    Ok(json!({ "agents": list }))
}

#[tauri::command]
async fn integration_install(id: String, remove: bool) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let env = integrations::Env::current();
        if remove {
            integrations::uninstall(&id, &env)
        } else {
            integrations::install(&id, &env)
        }
    })
    .await
    .map_err(|e| e.to_string())?
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

pub fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("HumanNeeded")
        .inner_size(760.0, 860.0)
        .min_inner_size(420.0, 400.0)
        .build();
}

fn main() {
    log::init(paths::data_dir().join("logs"));
    let first_run = !paths::data_dir().join("seen").exists();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_settings(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .invoke_handler(tauri::generate_handler![
            status,
            puck_schema,
            puck_settings,
            puck_set,
            puck_action,
            clear_sessions,
            cloud_set,
            fw_check,
            fw_update,
            integrations_list,
            integration_install,
            autostart_get,
            autostart_set,
            open_log
        ])
        .setup(move |app| {
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
            tray::build(app.handle(), engine.clone())?;
            let background = std::env::args().any(|a| a == "--background");
            if !background {
                show_settings(app.handle());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|app, event| match event {
            // Closing the window keeps the tray app running.
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
