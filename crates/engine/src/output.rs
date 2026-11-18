// SPDX-License-Identifier: GPL-3.0-or-later
//! What this computer's screen edges show (D-059), worked out in one place from the state
//! machine's shown state ([`crate::here`]) and the app's alert settings. Previews and real
//! agent events take the same path: the machine decides what is shown and for how long, this
//! only turns it into a look. The app's glow windows draw exactly this.

use crate::alerts::Alerts;
use serde::Serialize;

/// The screen edges' look. `None` (no glow) is the resting state.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Screen {
    /// `needs` | `done` | `working` | `idle`.
    pub state: String,
    /// `#rrggbb`.
    pub color: String,
    /// The light's name (`breathe`, `spin`, …), as on the puck.
    pub light: String,
    /// Opacity, 0.2..1 (the state's brightness, dimmed in quiet hours).
    pub level: f32,
    /// A `TEST` preview.
    pub preview: bool,
    /// When this state started (ms since the epoch; 0 for idle). Animations are timed from
    /// here, like the state's duration, so they end on a whole cycle.
    pub since: u64,
}

/// The look for `state` (as the machine shows it), or none when its screen edges are off.
pub fn screen(
    alerts: &Alerts,
    state: &str,
    preview: bool,
    since: u64,
    quiet: bool,
) -> Option<Screen> {
    let a = alerts.states.get(state)?;
    let l = &a.screen;
    if !l.on {
        return None;
    }
    let mut level = l.brightness.min(100) as f32 / 100.0;
    if quiet {
        level = level * crate::quiet::dimmed(100) as f32 / 100.0;
    }
    if level <= 0.01 {
        return None;
    }
    Some(Screen {
        state: state.into(),
        color: l.color.clone(),
        light: l.light.clone(),
        level: 0.2 + 0.8 * level,
        preview,
        since,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_state_settings() {
        let mut a = Alerts::default();
        let n = screen(&a, "needs", false, 0, false).unwrap();
        assert_eq!((n.color.as_str(), n.light.as_str()), ("#ffa62b", "breathe"));
        assert!((n.level - 1.0).abs() < 1e-5);
        // Working and idle don't glow by default.
        assert_eq!(screen(&a, "working", false, 0, false), None);
        assert_eq!(screen(&a, "idle", false, 0, false), None);
        assert_eq!(screen(&a, "dark", false, 0, false), None);
        a.states.get_mut("done").unwrap().screen.brightness = 50;
        assert!((screen(&a, "done", true, 7, false).unwrap().level - 0.6).abs() < 1e-5);
        assert!(screen(&a, "done", true, 7, false).unwrap().preview);
    }

    #[test]
    fn quiet_hours_dim_it() {
        let a = Alerts::default();
        assert!((screen(&a, "needs", false, 0, true).unwrap().level - 0.4).abs() < 1e-5);
    }
}
