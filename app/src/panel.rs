// SPDX-License-Identifier: FSL-1.1-ALv2
//! The tray panel (D-055): the app's only window. A small frameless window that opens next to the
//! tray / menu-bar icon and hides when it loses focus (click elsewhere).

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "panel";
/// CSS pixels; the page asks for its own height ([`fit`]).
const WIDTH: f64 = 340.0;
const START_HEIGHT: f64 = 480.0;
const MAX_HEIGHT: f64 = 640.0;
/// Gap between the panel and the taskbar / menu bar, in CSS pixels.
const GAP: f64 = 8.0;

/// Where the last open came from: the tray icon's rectangle (physical px), if known.
#[derive(Default)]
pub struct Anchor {
    rect: Mutex<Option<(f64, f64, f64, f64)>>,
    /// When the panel last hid by losing focus: a click on the tray icon that caused that blur
    /// must not open it again at once.
    hidden_at: Mutex<Option<Instant>>,
}

fn window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Ok(w);
    }
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("HumanNeeded")
        .inner_size(WIDTH, START_HEIGHT)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .shadow(true)
        .visible(false)
        .focused(true)
        .build()?;
    let app2 = app.clone();
    w.on_window_event(move |ev| {
        if let WindowEvent::Focused(false) = ev {
            if let Some(w) = app2.get_webview_window(LABEL) {
                let _ = w.hide();
            }
            *app2.state::<Anchor>().hidden_at.lock().unwrap() = Some(Instant::now());
        }
    });
    Ok(w)
}

/// Creates the (hidden) panel at startup so the first open is instant.
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    app.manage(Anchor::default());
    window(app).map(|_| ())
}

/// Places the panel next to the icon: above it when the taskbar is at the bottom (Windows),
/// below it when the bar is at the top (macOS menu bar), inside the screen's work area.
fn place(app: &AppHandle, w: &WebviewWindow) {
    let scale = w.scale_factor().unwrap_or(1.0);
    let size = w.outer_size().unwrap_or(PhysicalSize::new(
        (WIDTH * scale) as u32,
        (START_HEIGHT * scale) as u32,
    ));
    let (pw, ph) = (size.width as f64, size.height as f64);
    let rect = *app.state::<Anchor>().rect.lock().unwrap();
    let mon = match rect {
        Some((x, y, _, _)) => app.monitor_from_point(x, y).ok().flatten(),
        None => None,
    }
    .or_else(|| app.primary_monitor().ok().flatten());
    let Some(mon) = mon else { return };
    let wa = mon.work_area();
    let (ax, ay, aw, ah) = (
        wa.position.x as f64,
        wa.position.y as f64,
        wa.size.width as f64,
        wa.size.height as f64,
    );
    let gap = GAP * mon.scale_factor();
    let (x, y) = match rect {
        Some((ix, iy, iw, ih)) => {
            let x = ix + iw / 2.0 - pw / 2.0;
            // Icon in the lower half: open upwards.
            let y = if iy + ih / 2.0 > ay + ah / 2.0 {
                ay + ah - ph - gap
            } else {
                ay + gap
            };
            (x, y)
        }
        // No icon position (second launch): the usual tray corner.
        None if cfg!(target_os = "macos") => (ax + aw - pw - gap, ay + gap),
        None => (ax + aw - pw - gap, ay + ah - ph - gap),
    };
    let x = x.clamp(ax + gap, (ax + aw - pw - gap).max(ax + gap));
    let _ = w.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

pub fn show(app: &AppHandle) {
    let Ok(w) = window(app) else { return };
    place(app, &w);
    let _ = w.show();
    let _ = w.set_focus();
    let _ = tauri::Emitter::emit(&w, "panel-shown", ());
}

/// Tray icon left click: open (at the icon) or close.
pub fn toggle(app: &AppHandle, icon: Option<(f64, f64, f64, f64)>) {
    let anchor = app.state::<Anchor>();
    if icon.is_some() {
        *anchor.rect.lock().unwrap() = icon;
    }
    let just_hidden = anchor
        .hidden_at
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < Duration::from_millis(350));
    let visible = app
        .get_webview_window(LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        if let Some(w) = app.get_webview_window(LABEL) {
            let _ = w.hide();
        }
    } else if !just_hidden {
        show(app);
    }
}

/// The page's content height changed: resize (keeping the edge next to the icon in place).
pub fn fit(app: &AppHandle, height: f64) {
    let Some(w) = app.get_webview_window(LABEL) else {
        return;
    };
    let h = height.clamp(120.0, MAX_HEIGHT);
    let cur = w.inner_size().ok();
    let scale = w.scale_factor().unwrap_or(1.0);
    if cur.is_some_and(|c| ((c.height as f64) / scale - h).abs() < 1.0) {
        return;
    }
    let _ = w.set_size(tauri::LogicalSize::new(WIDTH, h));
    if w.is_visible().unwrap_or(false) {
        place(app, &w);
    }
}
