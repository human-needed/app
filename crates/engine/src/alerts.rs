// SPDX-License-Identifier: GPL-3.0-or-later
//! The app's alert settings (D-059 redesign): for each state, how this computer shows it
//! (screen edges, sound) and how the puck shows it (light, sound). The app owns them, kept in
//! `alerts.json`; the computer's outputs follow them, and the puck gets its part pushed whenever
//! it connects (only changed keys are written, the puck saves to flash on change).
//!
//! Existing users: the first time a puck connects, its settings are imported into the puck
//! part (its master levels folded into each state's), so nothing changes on the desk.
//!
//! Only `needs` and `done` have sounds: the firmware plays a sound when an agent starts
//! needing you and when one finishes, never for working or idle.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const STATES: [&str; 4] = ["needs", "done", "working", "idle"];
/// States that make a sound.
pub const SOUND_STATES: [&str; 2] = ["needs", "done"];
/// Light animations (the firmware's, without "off": each output has its own switch).
pub const LIGHTS: [&str; 6] = ["solid", "breathe", "blink", "heartbeat", "flicker", "spin"];
/// Sounds (the firmware's, without "none").
pub const SOUNDS: [&str; 15] = [
    "ding", "chime", "rising", "arpeggio", "bell", "beeps", "bloop", "bright", "clock", "doorbell",
    "fanfare", "ripple", "call", "lullaby", "harp",
];
/// How long a light shows, in seconds (0 = as long as the state lasts; not for `done`).
pub const DURATIONS: [u32; 10] = [0, 1, 2, 3, 5, 10, 30, 60, 300, 600];

/// A light: on the screen edges or on the puck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Look {
    pub on: bool,
    pub light: String,
    pub color: String,
    /// 0..100.
    pub brightness: u8,
    /// Seconds; 0 = as long as the state lasts.
    pub duration: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sound {
    pub on: bool,
    pub sound: String,
    /// 0..100.
    pub volume: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateAlerts {
    /// Screen edges on this computer.
    pub screen: Look,
    /// Sound on this computer (`needs`, `done` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<Sound>,
    /// The puck's light.
    pub puck: Look,
    /// The puck's sound (`needs`, `done` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub puck_sound: Option<Sound>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alerts {
    pub version: u32,
    pub states: BTreeMap<String, StateAlerts>,
    /// A puck's own settings were imported once (existing users), so from now on the app's
    /// settings are pushed to it.
    #[serde(default)]
    pub puck_imported: bool,
}

fn look(on: bool, light: &str, color: &str, brightness: u8, duration: u32) -> Look {
    Look {
        on,
        light: light.into(),
        color: color.into(),
        brightness,
        duration,
    }
}

fn sound(on: bool, s: &str, volume: u8) -> Option<Sound> {
    Some(Sound {
        on,
        sound: s.into(),
        volume,
    })
}

/// One state's defaults: colours, lights and sounds are firmware 2.0's; "working" doesn't glow
/// on the screen and "idle" only shows on the puck (WS-DESIGN round 13).
pub fn default_state(st: &str) -> StateAlerts {
    match st {
        "needs" => StateAlerts {
            screen: look(true, "breathe", "#ffa62b", 100, 0),
            sound: sound(true, "call", 70),
            puck: look(true, "breathe", "#ffa62b", 100, 0),
            puck_sound: sound(true, "call", 70),
        },
        "done" => StateAlerts {
            screen: look(true, "breathe", "#22e04a", 100, 10),
            sound: sound(true, "bright", 70),
            puck: look(true, "breathe", "#22e04a", 100, 3),
            puck_sound: sound(true, "bright", 70),
        },
        "working" => StateAlerts {
            screen: look(false, "spin", "#3366ff", 60, 0),
            sound: None,
            puck: look(true, "spin", "#3366ff", 60, 0),
            puck_sound: None,
        },
        _ => StateAlerts {
            screen: look(false, "solid", "#ffd2a0", 20, 0),
            sound: None,
            puck: look(true, "solid", "#ffd2a0", 12, 0),
            puck_sound: None,
        },
    }
}

impl Default for Alerts {
    fn default() -> Self {
        Self {
            version: 1,
            states: STATES
                .iter()
                .map(|s| (s.to_string(), default_state(s)))
                .collect(),
            puck_imported: false,
        }
    }
}

/// Pulse was removed: saved settings, older pucks and `hn set` get Breathe in its place.
fn current_light(light: &str) -> &str {
    if light == "pulse" {
        "breathe"
    } else {
        light
    }
}

/// A saved look, with a removed light replaced.
fn current_look(file: &Value) -> Value {
    let mut look = file.clone();
    if let Some(l) = look.get_mut("light") {
        if *l == "pulse" {
            *l = Value::from("breathe");
        }
    }
    look
}

fn valid_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn check_look(st: &str, l: &Look) -> Result<(), String> {
    if !LIGHTS.contains(&l.light.as_str()) {
        return Err(format!("{st}: unknown light {}", l.light));
    }
    if !valid_color(&l.color) {
        return Err(format!("{st}: bad colour {}", l.color));
    }
    if l.brightness > 100 {
        return Err(format!("{st}: brightness over 100"));
    }
    if l.duration > 3600 || (st == "done" && l.duration == 0) {
        return Err(format!("{st}: bad duration {}", l.duration));
    }
    Ok(())
}

fn check_sound(st: &str, s: &Option<Sound>) -> Result<(), String> {
    let has = SOUND_STATES.contains(&st);
    match s {
        None if has => Err(format!("{st}: sound missing")),
        Some(_) if !has => Err(format!("{st} has no sound")),
        Some(s) if !SOUNDS.contains(&s.sound.as_str()) => {
            Err(format!("{st}: unknown sound {}", s.sound))
        }
        Some(s) if s.volume > 100 => Err(format!("{st}: volume over 100")),
        _ => Ok(()),
    }
}

impl Alerts {
    /// Every state present and every value in range.
    pub fn check(&self) -> Result<(), String> {
        for st in STATES {
            let a = self.states.get(st).ok_or_else(|| format!("{st} missing"))?;
            check_look(st, &a.screen)?;
            check_look(st, &a.puck)?;
            check_sound(st, &a.sound)?;
            check_sound(st, &a.puck_sound)?;
        }
        if self.states.len() != STATES.len() {
            return Err("unknown state".into());
        }
        Ok(())
    }

    /// Loads `alerts.json`. Each output of each state (screen, sound, puck, puck sound) keeps
    /// the fields it has that are valid; anything missing, unknown or bad falls back to that
    /// state's defaults. So a file from an older version (fewer fields), a hand edit or a
    /// newer version (extra fields) never loses the rest of the user's settings.
    pub fn load(file: &Option<PathBuf>) -> Self {
        let Some(v) = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        else {
            return Self::default();
        };
        let mut a = Self {
            puck_imported: v["puck_imported"].as_bool().unwrap_or(false),
            ..Self::default()
        };
        for st in STATES {
            let file = &v["states"][st];
            let mut s = default_state(st);
            s.screen = merged(&s.screen, &current_look(&file["screen"]), |l| {
                check_look(st, l).is_ok()
            });
            s.puck = merged(&s.puck, &current_look(&file["puck"]), |l| {
                check_look(st, l).is_ok()
            });
            if let Some(d) = s.sound.clone() {
                s.sound = Some(merged(&d, &file["sound"], |x| {
                    check_sound(st, &Some(x.clone())).is_ok()
                }));
            }
            if let Some(d) = s.puck_sound.clone() {
                s.puck_sound = Some(merged(&d, &file["puck_sound"], |x| {
                    check_sound(st, &Some(x.clone())).is_ok()
                }));
            }
            a.states.insert(st.into(), s);
        }
        a
    }

    pub fn save(&self, file: &Option<PathBuf>) -> Result<(), String> {
        let Some(f) = file else { return Ok(()) };
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = f.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, f).map_err(|e| e.to_string())
    }

    fn state(&self, st: &str) -> StateAlerts {
        self.states
            .get(st)
            .cloned()
            .unwrap_or_else(|| default_state(st))
    }

    /// Settings for an `hn-usb/2` device (the puck, or this computer's own state machine).
    /// Levels live on each state, so the master levels are 100.
    fn device(&self, puck: bool) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("brightness".into(), "100".into());
        m.insert("volume".into(), "100".into());
        for st in STATES {
            let a = self.state(st);
            let (l, s) = if puck {
                (a.puck, a.puck_sound)
            } else {
                (a.screen, a.sound)
            };
            let light = if l.on { l.light.as_str() } else { "off" };
            m.insert(format!("{st}.light"), light.into());
            m.insert(format!("{st}.color"), l.color.clone());
            m.insert(format!("{st}.brightness"), l.brightness.to_string());
            if st != "idle" {
                let dur = if st == "done" {
                    l.duration.max(1)
                } else {
                    l.duration
                };
                m.insert(format!("{st}.duration"), dur.to_string());
            }
            if let Some(s) = s {
                let name = if s.on { s.sound.as_str() } else { "none" };
                m.insert(format!("{st}.sound"), name.into());
                m.insert(format!("{st}.volume"), s.volume.to_string());
            }
        }
        m
    }

    /// What the puck should hold.
    pub fn puck_settings(&self) -> BTreeMap<String, String> {
        self.device(true)
    }

    /// What this computer's state machine should hold (screen edges and sound).
    pub fn computer_settings(&self) -> BTreeMap<String, String> {
        self.device(false)
    }

    /// The `SET` lines that bring a puck holding `have` (`GET all`) to [`Self::puck_settings`],
    /// with `master` as its master brightness (100, or dimmed in quiet hours).
    pub fn puck_changes(&self, have: &Value, master: u8) -> Vec<String> {
        let mut want = self.puck_settings();
        want.insert("brightness".into(), master.min(100).to_string());
        want.into_iter()
            .filter(|(k, v)| text(&have[k]).as_deref() != Some(v.as_str()))
            .filter(|(k, _)| !have[k].is_null() || k == "brightness" || k == "volume")
            .map(|(k, v)| format!("SET {k} {v}"))
            .collect()
    }

    /// Existing users (first connect): takes the puck's look and sounds as the puck part, and
    /// its colours and lights for the screen too. Where the puck chimes, this computer stays
    /// quiet, as before.
    pub fn import_puck(&mut self, all: &Value) {
        let num = |k: &str| text(&all[k]).and_then(|v| v.parse::<u32>().ok());
        let master_b = num("brightness").unwrap_or(100).min(100);
        let master_v = num("volume").unwrap_or(100).min(100);
        for st in STATES {
            let mut a = self.state(st);
            if let Some(light) = text(&all[format!("{st}.light")]) {
                let light = current_light(&light);
                if light == "off" {
                    a.puck.on = false;
                } else if LIGHTS.contains(&light) {
                    a.puck.on = true;
                    a.puck.light = light.into();
                    a.screen.light = light.into();
                }
            }
            if let Some(c) = text(&all[format!("{st}.color")]).filter(|c| valid_color(c)) {
                a.puck.color = c.to_ascii_lowercase();
                a.screen.color = a.puck.color.clone();
            }
            if let Some(b) = num(&format!("{st}.brightness")) {
                a.puck.brightness = (b.min(100) * master_b / 100) as u8;
            }
            if let Some(d) = num(&format!("{st}.duration")) {
                if d <= 3600 && !(st == "done" && d == 0) {
                    a.puck.duration = d;
                }
            }
            if let (Some(ps), Some(name)) =
                (a.puck_sound.as_mut(), text(&all[format!("{st}.sound")]))
            {
                if name == "none" {
                    ps.on = false;
                } else if SOUNDS.contains(&name.as_str()) {
                    ps.on = true;
                    ps.sound = name;
                }
                if let Some(v) = num(&format!("{st}.volume")) {
                    ps.volume = (v.min(100) * master_v / 100) as u8;
                }
                if ps.on {
                    if let Some(s) = a.sound.as_mut() {
                        s.on = false;
                    }
                }
            }
            self.states.insert(st.into(), a);
        }
        self.puck_imported = true;
    }

    /// `/v1/set` and `hn set` (app-events, unchanged): a puck-style key sets that field for
    /// the screen and the puck alike. The master `brightness` / `volume` set every state's.
    pub fn apply_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        let (states, field): (Vec<&str>, &str) = match key.split_once('.') {
            Some((st, f)) if STATES.contains(&st) => (vec![st], f),
            None if key == "brightness" || key == "volume" => (STATES.to_vec(), key),
            _ => return Err("unknown key".into()),
        };
        let mut next = self.clone();
        for st in states {
            let a = next
                .states
                .entry(st.into())
                .or_insert_with(|| default_state(st));
            let level = || value.parse::<u8>().ok().filter(|v| *v <= 100);
            match field {
                "light" if value == "off" => {
                    a.screen.on = false;
                    a.puck.on = false;
                }
                "light" => {
                    for l in [&mut a.screen, &mut a.puck] {
                        l.on = true;
                        l.light = current_light(value).into();
                    }
                }
                "color" => {
                    for l in [&mut a.screen, &mut a.puck] {
                        l.color = value.to_ascii_lowercase();
                    }
                }
                "brightness" => {
                    let b = level().ok_or("out of range")?;
                    a.screen.brightness = b;
                    a.puck.brightness = b;
                }
                "duration" => {
                    let d: u32 = value.parse().map_err(|_| "out of range")?;
                    a.screen.duration = d;
                    a.puck.duration = d;
                }
                "sound" | "volume" => {
                    for s in [&mut a.sound, &mut a.puck_sound].into_iter().flatten() {
                        if field == "volume" {
                            s.volume = level().ok_or("out of range")?;
                        } else if value == "none" {
                            s.on = false;
                        } else {
                            s.on = true;
                            s.sound = value.into();
                        }
                    }
                }
                "repeat" => {}
                _ => return Err("unknown key".into()),
            }
        }
        next.check()?;
        *self = next;
        Ok(())
    }
}

/// `default` with each of `file`'s fields that keeps it valid (checked one field at a time).
fn merged<T>(default: &T, file: &Value, ok: impl Fn(&T) -> bool) -> T
where
    T: Clone + Serialize + serde::de::DeserializeOwned,
{
    let mut cur = default.clone();
    let Some(fields) = file.as_object() else {
        return cur;
    };
    for (k, val) in fields {
        let mut v = serde_json::to_value(&cur).unwrap_or(Value::Null);
        if v.get(k).is_none() {
            continue; // unknown field (a newer version)
        }
        v[k] = val.clone();
        if let Ok(next) = serde_json::from_value::<T>(v) {
            if ok(&next) {
                cur = next;
            }
        }
    }
    cur
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_valid_and_follow_the_design() {
        let a = Alerts::default();
        a.check().unwrap();
        assert!(!a.states["working"].screen.on, "working doesn't glow");
        assert!(!a.states["idle"].screen.on && a.states["idle"].puck.on);
        assert!(a.states["needs"].sound.as_ref().unwrap().on);
        assert!(a.states["working"].sound.is_none());
    }

    #[test]
    fn device_settings_map_switches_to_off_and_none() {
        let mut a = Alerts::default();
        a.states
            .get_mut("needs")
            .unwrap()
            .puck_sound
            .as_mut()
            .unwrap()
            .on = false;
        let p = a.puck_settings();
        assert_eq!(p["needs.sound"], "none");
        assert_eq!(p["needs.light"], "breathe");
        assert_eq!(p["brightness"], "100");
        assert_eq!(p["done.duration"], "3");
        assert!(!p.contains_key("idle.sound") && !p.contains_key("idle.duration"));
        let c = a.computer_settings();
        assert_eq!(c["working.light"], "off", "screen off for working");
        assert_eq!(c["done.duration"], "10");
        assert_eq!(c["needs.sound"], "call");
    }

    #[test]
    fn only_changed_keys_are_pushed() {
        let a = Alerts::default();
        let mut have = serde_json::to_value(a.puck_settings()).unwrap();
        assert!(a.puck_changes(&have, 100).is_empty());
        have["needs.color"] = json!("#000000");
        have["brightness"] = json!(60);
        let ch = a.puck_changes(&have, 100);
        assert_eq!(ch, vec!["SET brightness 100", "SET needs.color #ffa62b"]);
        // Keys the puck doesn't have (older firmware) aren't pushed.
        have.as_object_mut().unwrap().remove("idle.brightness");
        assert!(!a
            .puck_changes(&have, 100)
            .iter()
            .any(|l| l.contains("idle.brightness")));
    }

    #[test]
    fn importing_a_puck_keeps_its_look_and_one_chime() {
        let mut a = Alerts::default();
        a.import_puck(&json!({"brightness":"60","volume":"50","needs.light":"blink","needs.color":"#AA00FF",
            "needs.brightness":"100","needs.sound":"harp","needs.volume":"80","needs.duration":"0",
            "done.light":"off","done.sound":"none","working.light":"breathe","working.brightness":"50"}));
        let n = &a.states["needs"];
        assert_eq!(
            (n.puck.light.as_str(), n.puck.color.as_str()),
            ("blink", "#aa00ff")
        );
        assert_eq!(n.puck.brightness, 60, "master folded in");
        assert_eq!(n.screen.color, "#aa00ff");
        let ps = n.puck_sound.as_ref().unwrap();
        assert_eq!((ps.sound.as_str(), ps.volume), ("harp", 40));
        assert!(
            !n.sound.as_ref().unwrap().on,
            "the puck chimes, the computer doesn't"
        );
        let d = &a.states["done"];
        assert!(!d.puck.on && !d.puck_sound.as_ref().unwrap().on);
        assert!(
            d.sound.as_ref().unwrap().on,
            "no puck chime: the computer keeps its"
        );
        assert_eq!(a.states["working"].puck.brightness, 30);
        assert!(a.puck_imported);
        a.check().unwrap();
    }

    #[test]
    fn legacy_keys_set_both_outputs() {
        let mut a = Alerts::default();
        a.apply_key("needs.color", "#123456").unwrap();
        assert_eq!(a.states["needs"].screen.color, "#123456");
        assert_eq!(a.states["needs"].puck.color, "#123456");
        a.apply_key("done.sound", "none").unwrap();
        assert!(!a.states["done"].puck_sound.as_ref().unwrap().on);
        a.apply_key("volume", "40").unwrap();
        assert_eq!(a.states["needs"].sound.as_ref().unwrap().volume, 40);
        assert!(a.apply_key("needs.light", "disco").is_err());
        assert!(a.apply_key("done.duration", "0").is_err());
        assert!(a.apply_key("nope.color", "#000000").is_err());
        assert_eq!(
            a.states["needs"].screen.light, "breathe",
            "a failed set changes nothing"
        );
    }

    #[test]
    fn load_keeps_good_states_and_survives_bad_files() {
        let d = tempfile::tempdir().unwrap();
        let f = Some(d.path().join("alerts.json"));
        let mut a = Alerts::default();
        a.states.get_mut("done").unwrap().screen.color = "#010203".into();
        a.save(&f).unwrap();
        assert_eq!(Alerts::load(&f), a);
        let mut v = serde_json::to_value(&a).unwrap();
        v["states"]["needs"]["screen"]["light"] = json!("disco");
        std::fs::write(f.as_ref().unwrap(), v.to_string()).unwrap();
        let b = Alerts::load(&f);
        // Only the bad field falls back.
        assert_eq!(b.states["needs"].screen.light, "breathe");
        assert_eq!(b.states["done"].screen.color, "#010203");
        std::fs::write(f.as_ref().unwrap(), "nope").unwrap();
        assert_eq!(Alerts::load(&f), Alerts::default());
    }

    #[test]
    fn pulse_becomes_breathe_everywhere() {
        let d = tempfile::tempdir().unwrap();
        let f = Some(d.path().join("alerts.json"));
        let old =
            json!({"states":{"working":{"screen":{"light":"pulse"},"puck":{"light":"pulse"}}}});
        std::fs::write(f.as_ref().unwrap(), old.to_string()).unwrap();
        let mut a = Alerts::load(&f);
        assert_eq!(a.states["working"].screen.light, "breathe");
        assert_eq!(a.states["working"].puck.light, "breathe");

        a.import_puck(&json!({"idle.light":"pulse"}));
        assert_eq!(a.states["idle"].puck.light, "breathe");
        assert_eq!(a.states["idle"].screen.light, "breathe");

        a.apply_key("done.light", "blink").unwrap();
        a.apply_key("done.light", "pulse").unwrap();
        assert_eq!(a.states["done"].screen.light, "breathe");
        assert_eq!(a.states["done"].puck.light, "breathe");

        let mut bad = Alerts::default();
        bad.states.get_mut("needs").unwrap().screen.light = "pulse".into();
        assert!(bad.check().is_err(), "no longer a light to choose");
    }

    #[test]
    fn older_or_partial_files_keep_what_they_have() {
        let d = tempfile::tempdir().unwrap();
        let f = Some(d.path().join("alerts.json"));
        // Missing fields and outputs (an older or hand-written file), an unknown field and a
        // bad value: everything valid is kept, the rest is the state's defaults.
        let old = json!({"version":1,"puck_imported":true,"states":{
            "needs":{"screen":{"light":"blink","color":"#123456"},"puck":{"on":false,"future":1}},
            "done":{"screen":{"duration":0,"color":"#abcdef"},"sound":{"volume":200,"sound":"harp"}},
            "working":"garbage"}});
        std::fs::write(f.as_ref().unwrap(), old.to_string()).unwrap();
        let a = Alerts::load(&f);
        assert!(a.check().is_ok());
        assert!(a.puck_imported);
        let n = &a.states["needs"];
        assert_eq!(
            (n.screen.light.as_str(), n.screen.color.as_str()),
            ("blink", "#123456")
        );
        assert_eq!(
            n.screen.brightness,
            default_state("needs").screen.brightness
        );
        assert!(!n.puck.on);
        assert_eq!(n.sound, default_state("needs").sound);
        let dn = &a.states["done"];
        assert_eq!(dn.screen.color, "#abcdef");
        assert_eq!(dn.screen.duration, 10, "done can't last 0 s: default kept");
        let snd = dn.sound.as_ref().unwrap();
        assert_eq!((snd.sound.as_str(), snd.volume), ("harp", 70));
        assert_eq!(a.states["working"], default_state("working"));
        assert_eq!(a.states["idle"], default_state("idle"));
    }
}
