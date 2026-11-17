//! "This computer" (D-059): the app's own output when no puck is connected. The app is useful on
//! its own: the screen edges glow and the computer plays the sounds.
//!
//! It is the puck's own state machine (`hn_sim::SimPuck`, firmware 2.0.x defaults and sounds)
//! run inside the app, so the panel, quiet hours, mute and previews work the same with or
//! without a puck. Its settings are kept in `here.json`. While a puck is connected, every
//! setting the panel changes on the puck is copied here too, so the glow keeps the puck's look
//! after it's unplugged.

use hn_sim::SimPuck;
use serde_json::{Map, Value};
use std::path::PathBuf;

/// A sound for the computer to play.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    /// The state it's for (`needs`, `done`).
    pub state: String,
    /// Firmware sound name (`call`, `bright`, …).
    pub sound: String,
    /// 0..1: the master volume times the state's own volume.
    pub gain: f32,
    /// Times to play it (1..5).
    pub repeat: u8,
}

pub struct Here {
    puck: SimPuck,
    file: Option<PathBuf>,
}

/// A setting's value as text (`GET all` gives numbers for levels).
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

impl Here {
    /// Loads saved settings from `file` (missing or bad = defaults). `None` = memory only.
    pub fn load(file: Option<PathBuf>) -> Self {
        let mut puck = SimPuck::with_fw2_defaults("app");
        let saved = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str::<Map<String, Value>>(&s).ok())
            .unwrap_or_default();
        for (k, v) in saved {
            if let Some(v) = text(&v) {
                puck.handle(&format!("SET {k} {v}"), 0);
            }
        }
        Self { puck, file }
    }

    fn save(&self) {
        let Some(f) = &self.file else { return };
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = f.with_extension("json.tmp");
        let body = serde_json::to_string(&self.puck.settings).unwrap_or_default();
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, f);
        }
    }

    /// Runs one `hn-usb/2` command. Returns the reply line and the sounds it started.
    pub fn handle(&mut self, line: &str, now: u64) -> (String, Vec<Sound>) {
        let reply = self.puck.handle(line, now);
        if reply == "OK" && (line.starts_with("SET ") || line == "RESET") {
            self.save();
        }
        let played = std::mem::take(&mut self.puck.sounds);
        let sounds = played
            .iter()
            .filter_map(|p| {
                let (state, sound) = p.split_once(':')?;
                Some(self.sound(state, sound))
            })
            .collect();
        (reply, sounds)
    }

    fn level(&self, key: &str) -> f32 {
        self.puck
            .settings
            .get(key)
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(100.0)
            .clamp(0.0, 100.0)
            / 100.0
    }

    fn sound(&self, state: &str, sound: &str) -> Sound {
        let repeat = self
            .puck
            .settings
            .get(&format!("{state}.repeat"))
            .and_then(|r| r.parse().ok())
            .unwrap_or(1u8)
            .clamp(1, 5);
        Sound {
            state: state.into(),
            sound: sound.into(),
            gain: self.level("volume") * self.level(&format!("{state}.volume")),
            repeat,
        }
    }

    /// Copies a puck's settings (`GET all`): keys this side doesn't know are skipped.
    pub fn mirror(&mut self, all: &Value) {
        let Some(m) = all.as_object() else { return };
        let mut any = false;
        for (k, v) in m {
            if let Some(v) = text(v) {
                if self.puck.settings.get(k) != Some(&v) {
                    any |= self.puck.handle(&format!("SET {k} {v}"), 0) == "OK";
                }
            }
        }
        if any {
            self.save();
        }
    }

    /// One setting copied from a puck `SET` (no reply needed).
    pub fn mirror_set(&mut self, line: &str) {
        if self.puck.handle(line, 0) == "OK" {
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plays_with_levels_and_repeat() {
        let mut h = Here::load(None);
        h.handle("SET volume 50", 0);
        h.handle("SET needs.volume 80", 0);
        h.handle("SET needs.repeat 2", 0);
        let (r, s) = h.handle("SNAP 30 0000000a:needs", 1);
        assert_eq!(r, "OK");
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].sound.as_str(), s[0].repeat), ("call", 2));
        assert!((s[0].gain - 0.4).abs() < 1e-6);
        // Muted: nothing.
        h.handle("MUTE on", 2);
        assert!(h.handle("SNAP 30 0000000b:needs", 3).1.is_empty());
    }

    #[test]
    fn settings_persist_and_mirror() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("here.json");
        let mut h = Here::load(Some(f.clone()));
        assert!(h.handle("GET needs.light", 0).0.contains("pulse"));
        h.handle("SET needs.color #123456", 0);
        h.mirror(&json!({"done.sound":"harp","brightness":42,"unknown.key":"x"}));
        h.mirror_set("SET working.light breathe");
        let back = Here::load(Some(f));
        let all = serde_json::to_value(&back.puck.settings).unwrap();
        assert_eq!(all["needs.color"], "#123456");
        assert_eq!(all["done.sound"], "harp");
        assert_eq!(all["brightness"], "42");
        assert_eq!(all["working.light"], "breathe");
    }
}
