//! Screen-edge glow (D-059): the screen edges light up in the state's colour and animation, like
//! the puck. Works with or without a puck, on every monitor.
//!
//! One borderless, transparent, always-on-top window per monitor, covering it. It never takes
//! focus (not focusable, shown without activating) and never takes clicks (cursor events pass
//! through). The page (`glow.html`) only draws the edges; with nothing to show it draws
//! nothing and runs no animation, so it costs nothing. Only `needs`, `done` and `working` glow:
//! idle never does.

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};

const PREFIX: &str = "glow-";

/// What the edges show.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Spec {
    /// `needs` | `done` | `working`; `None` = nothing.
    pub state: Option<String>,
    pub color: String,
    /// The puck's light name (`pulse`, `spin`, …).
    pub light: String,
    /// 0..1.
    pub level: f32,
}

#[derive(Default)]
pub struct Glow {
    enabled: Mutex<bool>,
    spec: Mutex<Spec>,
    /// Monitor rectangles the windows were made for (physical px).
    screens: Mutex<Vec<(i32, i32, u32, u32)>>,
}

fn file() -> PathBuf {
    hn_engine::paths::data_dir().join("glow.json")
}

/// On unless the user turned it off.
fn load_enabled() -> bool {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["enabled"].as_bool())
        .unwrap_or(true)
}

fn num(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.as_f64().map(|f| f as f32),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// The glow for an engine status (`output.shown`) and the output's settings (`GET all`).
pub fn spec_of(status: &Value, settings: &Value) -> Spec {
    let shown = status["output"]["shown"].as_str().unwrap_or("idle");
    let st = shown.trim_start_matches("test:");
    if !matches!(st, "needs" | "done" | "working") {
        return Spec::default();
    }
    let light = settings[format!("{st}.light")]
        .as_str()
        .unwrap_or("solid")
        .to_string();
    let master = num(&settings["brightness"]).unwrap_or(60.0) / 100.0;
    let own = num(&settings[format!("{st}.brightness")]).unwrap_or(100.0) / 100.0;
    // "Working" can last hours: it glows softer than the states that want you.
    let soft = if st == "working" { 0.55 } else { 1.0 };
    let level = (master * own * soft).clamp(0.0, 1.0);
    if light == "off" || level <= 0.01 {
        return Spec::default();
    }
    Spec {
        state: Some(st.to_string()),
        color: settings[format!("{st}.color")]
            .as_str()
            .unwrap_or("#ffffff")
            .to_string(),
        light,
        level: 0.25 + 0.75 * level,
    }
}

pub fn init(app: &AppHandle) {
    let g = Glow::default();
    *g.enabled.lock().unwrap() = load_enabled();
    app.manage(g);
    sync(app);
}

pub fn enabled(app: &AppHandle) -> bool {
    *app.state::<Glow>().enabled.lock().unwrap()
}

pub fn set_enabled(app: &AppHandle, on: bool) -> Result<(), String> {
    *app.state::<Glow>().enabled.lock().unwrap() = on;
    let _ = std::fs::create_dir_all(hn_engine::paths::data_dir());
    std::fs::write(file(), serde_json::json!({ "enabled": on }).to_string())
        .map_err(|e| e.to_string())?;
    hn_engine::log!("glow: {}", if on { "on" } else { "off" });
    sync(app);
    Ok(())
}

/// The current spec (the page asks once it has loaded).
pub fn current(app: &AppHandle) -> Spec {
    app.state::<Glow>().spec.lock().unwrap().clone()
}

/// New state to show: sent to every glow window if it changed.
pub fn show(app: &AppHandle, spec: Spec) {
    let g = app.state::<Glow>();
    let mut cur = g.spec.lock().unwrap();
    if *cur == spec {
        return;
    }
    *cur = spec.clone();
    drop(cur);
    let _ = app.emit("glow", &spec);
}

/// Makes one window per monitor while the glow is on (none while it's off). Called at start,
/// on the toggle and every few seconds (monitors come and go, resolutions change).
pub fn sync(app: &AppHandle) {
    let on = enabled(app);
    let screens: Vec<(i32, i32, u32, u32)> = if on {
        app.available_monitors()
            .unwrap_or_default()
            .iter()
            .map(|m| {
                let (p, s) = (m.position(), m.size());
                (p.x, p.y, s.width, s.height)
            })
            .collect()
    } else {
        vec![]
    };
    let g = app.state::<Glow>();
    let mut have = g.screens.lock().unwrap();
    let windows = app
        .webview_windows()
        .into_iter()
        .filter(|(l, _)| l.starts_with(PREFIX))
        .count();
    if *have == screens && windows == screens.len() {
        return;
    }
    for (label, w) in app.webview_windows() {
        if label.starts_with(PREFIX) {
            let _ = w.destroy();
        }
    }
    *have = screens.clone();
    drop(have);
    for (i, (x, y, w, h)) in screens.into_iter().enumerate() {
        if let Err(e) = make(app, i, x, y, w, h) {
            hn_engine::log!("glow: window for screen {i}: {e}");
        }
    }
}

fn make(app: &AppHandle, i: usize, x: i32, y: i32, w: u32, h: u32) -> tauri::Result<()> {
    // A fresh label each time: a destroyed window's label can still be in use for a moment.
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let win = tauri::WebviewWindowBuilder::new(
        app,
        format!("{PREFIX}{i}-{n}"),
        WebviewUrl::App("glow.html".into()),
    )
    .title("HumanNeeded glow")
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()?;
    win.set_position(PhysicalPosition::new(x, y))?;
    win.set_size(PhysicalSize::new(w, h))?;
    win.set_ignore_cursor_events(true)?;
    win.show()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spec_follows_the_shown_state() {
        let set = json!({"brightness":"60","needs.color":"#ffa62b","needs.light":"pulse","needs.brightness":"100",
            "working.color":"#3366ff","working.light":"spin","working.brightness":100,
            "done.light":"off","done.color":"#22e04a","done.brightness":"100"});
        let st = |s: &str| json!({"output":{"shown":s}});
        let n = spec_of(&st("needs"), &set);
        assert_eq!(n.state.as_deref(), Some("needs"));
        assert_eq!((n.color.as_str(), n.light.as_str()), ("#ffa62b", "pulse"));
        assert!((n.level - (0.25 + 0.75 * 0.6)).abs() < 1e-5);
        let w = spec_of(&st("test:working"), &set);
        assert_eq!(w.light, "spin");
        assert!(w.level < n.level, "working glows softer");
        assert_eq!(spec_of(&st("idle"), &set), Spec::default());
        assert_eq!(spec_of(&st("done"), &set), Spec::default(), "light off");
        assert_eq!(spec_of(&json!({}), &set), Spec::default());
    }
}
