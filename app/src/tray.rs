// SPDX-License-Identifier: GPL-3.0-or-later
//! Tray / menu-bar icon (D-055): a ring in the colour of the state shown (on the puck, or on this
//! computer when there's no puck, D-059), with a small dot when sounds are muted (grey) or
//! something needs fixing (red). Left click opens the panel; right click: Screen edge glow,
//! Mute for 1 hour, Quit. The same loop drives the screen-edge glow.

use hn_engine::Engine;
use serde_json::Value;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter};

const GREY_RING: [u8; 3] = [0x6b, 0x70, 0x75];
const DOT_MUTED: [u8; 3] = [0x9a, 0xa0, 0xa5];
const DOT_PROBLEM: [u8; 3] = [0xff, 0x8f, 0x80];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dot {
    None,
    Muted,
    Problem,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Look {
    pub ring: [u8; 3],
    pub dot: Dot,
}

/// The brand mark (D-058) as a 32 px icon: a ring of 8 segments, gaps at the top, right, bottom
/// and left and between them, plus the dot.
pub fn icon(look: &Look) -> Image<'static> {
    const N: u32 = 32;
    let mut px = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) / 2.0;
    let (r_mid, half_w) = (10.6_f32, 2.5_f32);
    let (dx, dy, dr) = (24.5_f32, 24.5_f32, 5.2_f32);
    let dot = match look.dot {
        Dot::None => None,
        Dot::Muted => Some(DOT_MUTED),
        Dot::Problem => Some(DOT_PROBLEM),
    };
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 - c, y as f32 - c);
            let d = (fx * fx + fy * fy).sqrt();
            // Ring band, anti-aliased.
            let band = (half_w + 0.5 - (d - r_mid).abs()).clamp(0.0, 1.0);
            // 8 segments: a gap of ~21% of each (as the mark), edges anti-aliased over ~1 px.
            let a = fy.atan2(fx).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * 8.0;
            let f = a.fract();
            let seg = ((f.min(1.0 - f) - 0.1035) * 8.3 + 0.5).clamp(0.0, 1.0);
            let mut alpha = band * seg;
            let mut rgb = look.ring;
            if let Some(col) = dot {
                let dd = ((x as f32 - dx).powi(2) + (y as f32 - dy).powi(2)).sqrt();
                // Cut a clear border around the dot, then the dot itself.
                let cut = (dr + 2.0 - dd).clamp(0.0, 1.0);
                alpha *= 1.0 - cut;
                let inside = (dr + 0.5 - dd).clamp(0.0, 1.0);
                if inside > 0.0 {
                    rgb = col;
                    alpha = alpha.max(inside);
                }
            }
            let i = ((y * N + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], (alpha * 255.0) as u8]);
        }
    }
    Image::new_owned(px, N, N)
}

fn hex_rgb(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

/// A problem the panel shows on its top line (and the tray as a red dot). No puck is not a
/// problem (D-059): the app works on its own.
pub fn problem(status: &Value) -> bool {
    status["http"]["error"].is_string()
        || status["proxy"].is_object()
        || (status["puck"]["connected"] == true && status["puck"]["compatible"] != true)
}

/// What the tray shows, from the engine status and the output's colours (`GET all`).
pub fn look_of(status: &Value, colours: &Value) -> (Look, String) {
    let connected = status["puck"]["connected"] == true;
    let shown = status["output"]["shown"]
        .as_str()
        .unwrap_or("idle")
        .trim_start_matches("test:")
        .to_string();
    let base = match shown.as_str() {
        "needs" | "working" | "done" | "idle" => shown.as_str(),
        _ => "idle",
    };
    let ring = colours[format!("{base}.color")]
        .as_str()
        .and_then(hex_rgb)
        .unwrap_or(GREY_RING);
    let muted = status["output"]["status"]["muted"]
        .as_str()
        .is_some_and(|m| m != "off");
    let dot = if problem(status) {
        Dot::Problem
    } else if muted {
        Dot::Muted
    } else {
        Dot::None
    };
    let text = if status["http"]["error"].is_string() {
        "agents can't reach the app".to_string()
    } else if status["proxy"].is_object() {
        "a proxy setting blocks agents".to_string()
    } else if connected && status["puck"]["compatible"] != true {
        "the puck needs a firmware update".to_string()
    } else {
        let snap = status["snapshot"].as_array().cloned().unwrap_or_default();
        let needs = snap.iter().filter(|e| e["state"] == "needs").count();
        let working = snap.iter().filter(|e| e["state"] == "working").count();
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
        if parts.is_empty() {
            "nothing running".into()
        } else {
            parts.join(", ")
        }
    };
    (Look { ring, dot }, format!("HumanNeeded: {text}"))
}

pub fn build(app: &AppHandle, engine: Engine) -> tauri::Result<()> {
    let glow = CheckMenuItem::with_id(
        app,
        "glow",
        "Screen edge glow",
        true,
        crate::glow::enabled(app),
        None::<&str>,
    )?;
    let mute = MenuItem::with_id(app, "mute", "Mute for 1 hour", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit HumanNeeded", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&glow, &mute, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let e = engine.clone();
    let start = Look {
        ring: GREY_RING,
        dot: Dot::None,
    };
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon(&start))
        .tooltip("HumanNeeded")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, ev| match ev.id.as_ref() {
            "glow" => {
                let _ = crate::glow::set_enabled(app, !crate::glow::enabled(app));
                let _ = app.emit("changed", ());
            }
            "mute" => {
                let e = e.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = e.puck("MUTE 60").await;
                    let _ = app.emit("changed", ());
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = ev
            {
                let p = rect.position.to_physical::<f64>(1.0);
                let s = rect.size.to_physical::<f64>(1.0);
                crate::panel::toggle(tray.app_handle(), Some((p.x, p.y, s.width, s.height)));
            }
        })
        .build(app)?;

    // Follow the engine: icon + tell the panel to refresh.
    let app = app.clone();
    let mut rx = engine.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut last: Option<(Look, String)> = None;
        let mut colours = Value::Null;
        let mut colours_at: Option<std::time::Instant> = None;
        let mut was_output = String::new();
        let mut screens_at: Option<std::time::Instant> = None;
        loop {
            let status = engine.status().await;
            let output = status["output"]["device"]
                .as_str()
                .unwrap_or("")
                .to_string();
            // The output's settings: when it changes (puck plugged in or out), then every 30 s
            // (the panel may change them).
            let stale = colours_at.is_none_or(|t| t.elapsed().as_secs() >= 30);
            if output != was_output || stale {
                if let Ok(r) = engine.puck("GET all").await {
                    colours = serde_json::from_str(&r).unwrap_or(Value::Null);
                    colours_at = Some(std::time::Instant::now());
                }
            }
            was_output = output;
            let now = look_of(&status, &colours);
            if last.as_ref() != Some(&now) {
                let _ = tray.set_icon(Some(icon(&now.0)));
                let _ = tray.set_tooltip(Some(&now.1));
                last = Some(now);
            }
            // Screen-edge glow (D-059): windows for the current monitors, then what to show.
            if screens_at.is_none_or(|t| t.elapsed().as_secs() >= 5) {
                crate::glow::sync(&app);
                screens_at = Some(std::time::Instant::now());
            }
            crate::glow::show(&app, crate::glow::spec_of(&status, &colours));
            let _ = glow.set_checked(crate::glow::enabled(&app));
            let _ = app.emit("changed", ());
            // Wake on any change, and at least every 5 s (puck STATUS, mute timers).
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                r = rx.changed() => { if r.is_err() { break; } }
                _ = crate::COLOURS_CHANGED.notified() => { colours_at = None; }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ring_follows_the_shown_state_and_dots() {
        let colours = json!({"needs.color":"#ff8800","idle.color":"#ffaa66"});
        let ok = json!({"http":{},"puck":{"connected":true,"compatible":true},"output":{"device":"puck","shown":"needs","status":{"muted":"off"}},"snapshot":[{"state":"needs"}]});
        let (l, t) = look_of(&ok, &colours);
        assert_eq!(l.ring, [0xff, 0x88, 0x00]);
        assert_eq!(l.dot, Dot::None);
        assert_eq!(t, "HumanNeeded: 1 needs you");
        let mut muted = ok.clone();
        muted["output"]["status"]["muted"] = json!("42");
        assert_eq!(look_of(&muted, &colours).0.dot, Dot::Muted);
        // No puck (D-059): no nag; the ring shows this computer's state.
        let unplugged = json!({"http":{},"puck":{"connected":false},"output":{"device":"computer","shown":"idle","status":{"muted":"off"}},"snapshot":[]});
        let (l, t) = look_of(&unplugged, &colours);
        assert_eq!((l.ring, l.dot), ([0xff, 0xaa, 0x66], Dot::None));
        assert_eq!(t, "HumanNeeded: nothing running");
        let mut old = ok.clone();
        old["puck"]["compatible"] = json!(false);
        assert_eq!(look_of(&old, &colours).0.dot, Dot::Problem);
        let mut proxy = ok.clone();
        proxy["proxy"] = json!({"problem":"x"});
        assert_eq!(look_of(&proxy, &colours).0.dot, Dot::Problem);
    }

    #[test]
    fn icon_is_32px_and_has_a_dot() {
        let img = icon(&Look {
            ring: [255, 0, 0],
            dot: Dot::Problem,
        });
        assert_eq!((img.width(), img.height()), (32, 32));
        let px = img.rgba();
        let at = |x: usize, y: usize| &px[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(&at(24, 24)[..3], &DOT_PROBLEM);
        assert_eq!(at(16, 16)[3], 0, "centre is clear");
        // The 8-segment mark: a gap at the top, a solid segment 22.5° clockwise from it.
        assert!(at(16, 5)[3] < 64, "gap at the top");
        assert_eq!(at(20, 6)[3], 255, "segment between the gaps");
    }
}
