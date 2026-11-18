// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen-edge glow (D-059): the screen edges light up in the state's colour and animation, as
//! each state's "Screen edges" settings say. Works with or without a puck.
//!
//! What to show comes from the engine ([`hn_engine::Engine::screen`]), which changes it the
//! moment the shown state does (events, previews, durations). This module only draws it: no
//! timing here.
//!
//! One borderless, transparent, always-on-top window per monitor (or only the main one),
//! covering it. It never takes focus (not focusable, shown without activating) and never takes
//! clicks (cursor events pass through), and it's left out of screenshots and screen shares. The
//! page (`glow.html`) only draws the edges; with nothing to show it draws nothing and runs no
//! animation. Options (Settings → Screen edges): which screens, thickness, and skipping a
//! monitor while a full-screen app is in front there (Windows). The tray menu's
//! "Screen edge glow" pauses it all.

use hn_engine::output::Screen;
use hn_engine::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};

const PREFIX: &str = "glow-";

/// What a glow page draws.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Spec {
    /// `needs` | `done` | `working` | `idle`; `None` = nothing.
    pub state: Option<String>,
    pub color: String,
    /// The light's name (`pulse`, `spin`, …).
    pub light: String,
    /// 0..1.
    pub level: f32,
    /// `thin` | `mid` | `wide`.
    pub thickness: String,
}

impl Spec {
    fn of(look: &Option<Screen>, thickness: &str) -> Self {
        match look {
            None => Spec::default(),
            Some(l) => Spec {
                state: Some(l.state.clone()),
                color: l.color.clone(),
                light: l.light.clone(),
                level: l.level,
                thickness: thickness.into(),
            },
        }
    }
}

/// The glow's own settings (`glow.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// The tray's pause switch.
    pub enabled: bool,
    /// `all` | `main`.
    pub screens: String,
    /// `thin` | `mid` | `wide`.
    pub thickness: String,
    /// No glow on a monitor where a full-screen app (game, video) is in front.
    pub skip_fullscreen: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            enabled: true,
            screens: "all".into(),
            thickness: "mid".into(),
            skip_fullscreen: true,
        }
    }
}

impl Options {
    pub fn check(&self) -> Result<(), String> {
        if !["all", "main"].contains(&self.screens.as_str()) {
            return Err("screens: all or main".into());
        }
        if !["thin", "mid", "wide"].contains(&self.thickness.as_str()) {
            return Err("thickness: thin, mid or wide".into());
        }
        Ok(())
    }
}

/// A monitor rectangle (physical px): x, y, width, height.
type Rect = (i32, i32, u32, u32);

#[derive(Default)]
pub struct Glow {
    options: Mutex<Options>,
    /// What the engine says the edges show now.
    look: Mutex<Option<Screen>>,
    /// Window label → the monitor it covers.
    windows: Mutex<Vec<(String, Rect)>>,
    /// What each window was last sent.
    sent: Mutex<Vec<(String, Spec)>>,
    /// A [`sync`] is making or closing windows (only one at a time).
    syncing: AtomicBool,
}

fn file() -> PathBuf {
    hn_engine::paths::data_dir().join("glow.json")
}

fn load() -> Options {
    let o: Options = std::fs::read_to_string(file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if o.check().is_ok() {
        o
    } else {
        Options::default()
    }
}

/// Loads the options and follows the engine: each change of what the edges show goes to the
/// glow windows at once. The windows are made off the main thread, by [`start`]'s loop, so
/// setup never waits on window creation.
pub fn init(app: &AppHandle) {
    let g = Glow::default();
    *g.options.lock().unwrap() = load();
    app.manage(g);
}

/// Starts the glow: follows the engine's look, and every few seconds matches the windows to
/// the monitors (they come and go) and re-checks full-screen apps.
pub fn start(app: &AppHandle, engine: &Engine) {
    let mut rx = engine.screen();
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let look = rx.borrow_and_update().clone();
            *a.state::<Glow>().look.lock().unwrap() = look;
            show(&a);
            if rx.changed().await.is_err() {
                break;
            }
        }
    });
    let a = app.clone();
    std::thread::spawn(move || {
        let mut first = true;
        loop {
            sync(&a);
            show(&a);
            if first {
                first = false;
                // Proof the main thread runs after startup (the CI smoke run waits for it).
                let _ = a.run_on_main_thread(|| hn_engine::log!("ui: ready"));
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

pub fn options(app: &AppHandle) -> Options {
    app.state::<Glow>().options.lock().unwrap().clone()
}

pub fn set_options(app: &AppHandle, o: Options) -> Result<(), String> {
    o.check()?;
    *app.state::<Glow>().options.lock().unwrap() = o.clone();
    let _ = std::fs::create_dir_all(hn_engine::paths::data_dir());
    std::fs::write(file(), serde_json::to_string(&o).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    hn_engine::log!(
        "glow: {} screens {} thickness {} skip full-screen {}",
        if o.enabled { "on" } else { "paused" },
        o.screens,
        o.thickness,
        o.skip_fullscreen
    );
    // Thickness changes the spec; screens the windows (made off the calling thread).
    let app = app.clone();
    std::thread::spawn(move || {
        sync(&app);
        show(&app);
    });
    Ok(())
}

pub fn enabled(app: &AppHandle) -> bool {
    options(app).enabled
}

pub fn set_enabled(app: &AppHandle, on: bool) -> Result<(), String> {
    set_options(
        app,
        Options {
            enabled: on,
            ..options(app)
        },
    )
}

/// What a glow window should show now (the page asks once it has loaded).
pub fn current(app: &AppHandle, label: &str) -> Spec {
    let g = app.state::<Glow>();
    let o = options(app);
    let spec = Spec::of(&g.look.lock().unwrap(), &o.thickness);
    let rect = g
        .windows
        .lock()
        .unwrap()
        .iter()
        .find(|(l, _)| l == label)
        .map(|(_, r)| *r);
    // (Each lock above is released at the end of its statement.)
    match rect {
        Some(r) if o.skip_fullscreen && spec.state.is_some() && fullscreen_rect() == Some(r) => {
            Spec::default()
        }
        _ => spec,
    }
}

/// Sends each glow window what it should show, if that changed.
pub fn show(app: &AppHandle) {
    let g = app.state::<Glow>();
    let o = options(app);
    let spec = Spec::of(&g.look.lock().unwrap(), &o.thickness);
    let windows = g.windows.lock().unwrap().clone();
    let full = if o.skip_fullscreen && spec.state.is_some() {
        fullscreen_rect()
    } else {
        None
    };
    // Work out what to send under the lock, send without it.
    let mut out = vec![];
    {
        let mut sent = g.sent.lock().unwrap();
        for (label, rect) in windows {
            let want = if full == Some(rect) {
                Spec::default()
            } else {
                spec.clone()
            };
            if sent.iter().any(|(l, s)| *l == label && *s == want) {
                continue;
            }
            sent.retain(|(l, _)| *l != label);
            sent.push((label.clone(), want.clone()));
            out.push((label, want));
        }
    }
    for (label, want) in out {
        let _ = app.emit_to(label.as_str(), "glow", &want);
    }
}

/// Makes one window per monitor (or the main one) while the glow is on, none while it's
/// paused. Called at start, on option changes and every few seconds (monitors come and go).
pub fn sync(app: &AppHandle) {
    let o = options(app);
    let rect = |m: &tauri::Monitor| {
        let (p, s) = (m.position(), m.size());
        (p.x, p.y, s.width, s.height)
    };
    let screens: Vec<Rect> = match (o.enabled, o.screens.as_str()) {
        (false, _) => vec![],
        (true, "main") => app
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| vec![rect(&m)])
            .unwrap_or_default(),
        _ => app
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(rect)
            .collect(),
    };
    let g = app.state::<Glow>();
    let alive = app
        .webview_windows()
        .into_iter()
        .filter(|(l, _)| l.starts_with(PREFIX))
        .count();
    let same = {
        let have = g.windows.lock().unwrap();
        have.iter().map(|(_, r)| *r).collect::<Vec<_>>() == screens && alive == screens.len()
    };
    if same || g.syncing.swap(true, Ordering::SeqCst) {
        return;
    }
    // No lock is held while windows are made or closed: making one runs the main thread's
    // event loop, where a glow page's `glow_spec` may need these locks (beta.7 hang).
    *g.windows.lock().unwrap() = vec![];
    g.sent.lock().unwrap().clear();
    for (label, w) in app.webview_windows() {
        if label.starts_with(PREFIX) {
            let _ = w.destroy();
        }
    }
    let mut made = vec![];
    for (i, r) in screens.into_iter().enumerate() {
        match make(app, i, r) {
            Ok(label) => made.push((label, r)),
            Err(e) => hn_engine::log!("glow: window for screen {i}: {e}"),
        }
    }
    hn_engine::log!("glow: {} window(s)", made.len());
    *g.windows.lock().unwrap() = made;
    g.syncing.store(false, Ordering::SeqCst);
}

fn make(app: &AppHandle, i: usize, (x, y, w, h): Rect) -> tauri::Result<String> {
    // A fresh label each time: a destroyed window's label can still be in use for a moment.
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let label = format!("{PREFIX}{i}-{n}");
    let win = tauri::WebviewWindowBuilder::new(app, &label, WebviewUrl::App("glow.html".into()))
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
        // Not in screenshots, recordings or screen shares: the glow is for the person at the desk.
        // (Debug builds: HN_GLOW_CAPTURE=1 lets tests sample it from the screen.)
        .content_protected(
            !(cfg!(debug_assertions) && std::env::var_os("HN_GLOW_CAPTURE").is_some()),
        )
        .visible(false)
        .build()?;
    win.set_position(PhysicalPosition::new(x, y))?;
    win.set_size(PhysicalSize::new(w, h))?;
    win.set_ignore_cursor_events(true)?;
    win.show()?;
    Ok(label)
}

/// The monitor rectangle a full-screen app covers right now, if any (Windows: the window in
/// front exactly covers its monitor and isn't the desktop).
#[cfg(windows)]
fn fullscreen_rect() -> Option<Rect> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowRect,
    };
    // SAFETY: plain Win32 queries on the foreground window; every out-pointer is a local.
    unsafe {
        let w = GetForegroundWindow();
        if w.is_null() {
            return None;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(w, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if ["Progman", "WorkerW", "Shell_TrayWnd"].contains(&class.as_str()) {
            return None;
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(w, &mut r) == 0 {
            return None;
        }
        let m = MonitorFromWindow(w, MONITOR_DEFAULTTONULL);
        if m.is_null() {
            return None;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(m, &mut mi) == 0 {
            return None;
        }
        let mr = mi.rcMonitor;
        let covers =
            r.left <= mr.left && r.top <= mr.top && r.right >= mr.right && r.bottom >= mr.bottom;
        covers.then_some((
            mr.left,
            mr.top,
            (mr.right - mr.left) as u32,
            (mr.bottom - mr.top) as u32,
        ))
    }
}

/// Not detected on this OS yet.
#[cfg(not(windows))]
fn fullscreen_rect() -> Option<Rect> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_checked() {
        assert!(Options::default().check().is_ok());
        let bad = Options {
            thickness: "huge".into(),
            ..Default::default()
        };
        assert!(bad.check().is_err());
    }
}
