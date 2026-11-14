//! Tray icon: a dot in the colour of what the puck shows, a short menu.

use hn_engine::Engine;
use serde_json::Value;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Look {
    NoPuck,
    Idle,
    Working,
    Needs,
}

fn dot(look: Look) -> Image<'static> {
    const N: u32 = 32;
    let (r, g, b) = match look {
        Look::NoPuck => (120, 120, 120),
        Look::Idle => (255, 170, 102),
        Look::Working => (51, 102, 255),
        Look::Needs => (255, 136, 0),
    };
    let mut px = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let i = ((y * N + x) * 4) as usize;
            let (outer, inner) = (15.0, if look == Look::NoPuck { 10.0 } else { 0.0 });
            let a = ((outer - d).clamp(0.0, 1.0) * (d - inner + 1.0).clamp(0.0, 1.0) * 255.0) as u8;
            px[i..i + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    Image::new_owned(px, N, N)
}

fn look_of(status: &Value) -> (Look, String) {
    if let Some(err) = status["http"]["error"].as_str() {
        return (
            Look::NoPuck,
            format!("HumanNeeded: agents can't reach the app ({err})"),
        );
    }
    if status["puck"]["connected"] != true {
        return (Look::NoPuck, "HumanNeeded: puck not connected".into());
    }
    if status["puck"]["compatible"] != true {
        return (
            Look::NoPuck,
            "HumanNeeded: puck needs a firmware update".into(),
        );
    }
    let snap = status["snapshot"].as_array().cloned().unwrap_or_default();
    let needs = snap.iter().filter(|e| e["state"] == "needs").count();
    let working = snap.iter().filter(|e| e["state"] == "working").count();
    let look = if needs > 0 {
        Look::Needs
    } else if working > 0 {
        Look::Working
    } else {
        Look::Idle
    };
    let mut parts = vec![];
    if needs > 0 {
        parts.push(format!(
            "{needs} need{} you",
            if needs == 1 { "s" } else { "" }
        ));
    }
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    let text = if parts.is_empty() {
        "nothing running".into()
    } else {
        parts.join(", ")
    };
    (look, format!("HumanNeeded: {text}"))
}

pub fn build(app: &AppHandle, engine: Engine) -> tauri::Result<()> {
    let state_item = MenuItem::with_id(app, "state", "Starting…", false, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Open HumanNeeded…", true, None::<&str>)?;
    let test = MenuItem::with_id(app, "test", "Test the light", true, None::<&str>)?;
    let mute = CheckMenuItem::with_id(
        app,
        "mute",
        "Mute sounds for 1 hour",
        true,
        false,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit HumanNeeded", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &state_item,
            &sep,
            &settings,
            &test,
            &mute,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let e = engine.clone();
    let mute_item = mute.clone();
    let tray = TrayIconBuilder::with_id("main")
        .icon(dot(Look::NoPuck))
        .tooltip("HumanNeeded")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, ev| {
            let e = e.clone();
            match ev.id.as_ref() {
                "settings" => crate::show_settings(app),
                "test" => {
                    tauri::async_runtime::spawn(async move {
                        let _ = e.puck("TEST needs").await;
                    });
                }
                "mute" => {
                    let on = mute_item.is_checked().unwrap_or(false);
                    tauri::async_runtime::spawn(async move {
                        let _ = e.puck(if on { "MUTE 60" } else { "MUTE 0" }).await;
                    });
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                crate::show_settings(tray.app_handle());
            }
        })
        .build(app)?;

    // Follow the engine: tray look + tell the window to refresh.
    let app = app.clone();
    let mut rx = engine.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut last = None;
        loop {
            let status = engine.status().await;
            let (look, text) = look_of(&status);
            if last.as_ref() != Some(&(look, text.clone())) {
                let _ = tray.set_icon(Some(dot(look)));
                let _ = tray.set_tooltip(Some(&text));
                let _ = state_item.set_text(text.trim_start_matches("HumanNeeded: "));
                last = Some((look, text));
            }
            let _ = app.emit("changed", ());
            // Wake on any change, and at least every 5 s (puck STATUS, mute timers).
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), rx.changed()).await;
        }
    });
    Ok(())
}
