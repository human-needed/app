// SPDX-License-Identifier: GPL-3.0-or-later
//! The puck's behaviour per `hn-usb/2`, as a pure state machine (time is passed in, ms).

use crate::machine::{Durations, Machine, St};
use hn_core::proto::{valid_id, SnapState, MAX_ENTRIES, MAX_LINE};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const STATES: [&str; 4] = ["needs", "done", "working", "idle"];
const LIGHTS: [&str; 8] = [
    "off",
    "solid",
    "breathe",
    "pulse",
    "blink",
    "heartbeat",
    "flicker",
    "spin",
];
/// Firmware 2.0.x's sounds, in its order (the melodies were added after "bloop").
pub const SOUNDS: [&str; 16] = [
    "none", "ding", "chime", "rising", "arpeggio", "bell", "beeps", "bloop", "bright", "clock",
    "doorbell", "fanfare", "ripple", "call", "lullaby", "harp",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mute {
    Off,
    On,
    Until(u64),
}

#[derive(Debug, Clone)]
pub struct SimPuck {
    pub fw: String,
    pub board: String,
    pub serial: String,
    pub settings: BTreeMap<String, String>,
    /// What's shown and when it changes: the firmware's own logic.
    pub machine: Machine,
    mute: Mute,
    /// Every sound played, oldest first (for tests).
    pub sounds: Vec<String>,
    /// Set by BOOTSEL / REBOOT; the transport acts on it after the reply is sent.
    pub pending: Option<Pending>,
    /// Firmware 2.0.x defaults (`RESET` goes back to them).
    fw2: bool,
    /// Light shown at power-up / reconnect only ("hello" light, silent).
    pub suspended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Reboot,
    Bootsel,
}

fn defaults(fw2: bool) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("brightness".into(), "60".into());
    m.insert("volume".into(), "70".into());
    // The simulator's own defaults (tests rely on them), or firmware 2.0.x's (`settings.cpp`).
    let styles = if fw2 {
        [
            ("needs", "#ffa62b", "pulse", "call", "0"),
            ("done", "#22e04a", "pulse", "bright", "3"),
            ("working", "#3366ff", "spin", "none", "0"),
            ("idle", "#ffd2a0", "solid", "none", "0"),
        ]
    } else {
        [
            ("needs", "#ff8800", "pulse", "chime", "0"),
            ("done", "#22cc55", "solid", "ding", "5"),
            ("working", "#3366ff", "breathe", "none", "0"),
            ("idle", "#ffaa66", "solid", "none", "0"),
        ]
    };
    for (st, color, light, sound, duration) in styles {
        m.insert(format!("{st}.color"), color.into());
        m.insert(format!("{st}.light"), light.into());
        m.insert(
            format!("{st}.brightness"),
            match (st, fw2) {
                ("idle", true) => "20",
                ("idle", false) => "10",
                _ => "100",
            }
            .into(),
        );
        if st != "idle" {
            m.insert(format!("{st}.duration"), duration.into());
            m.insert(format!("{st}.sound"), sound.into());
            m.insert(format!("{st}.volume"), "100".into());
            m.insert(format!("{st}.repeat"), "1".into());
        }
    }
    m
}

impl SimPuck {
    pub fn new(fw: &str) -> Self {
        Self::with_defaults(fw, false)
    }

    /// The same machine with firmware 2.0.x's defaults (the app's "this computer" output).
    pub fn with_fw2_defaults(fw: &str) -> Self {
        Self::with_defaults(fw, true)
    }

    fn with_defaults(fw: &str, fw2: bool) -> Self {
        Self {
            fw: fw.into(),
            board: "hn-v2".into(),
            serial: "HN00SIMULATOR000".into(),
            settings: defaults(fw2),
            fw2,
            machine: Machine::new(Default::default()),
            mute: Mute::Off,
            sounds: Vec::new(),
            pending: None,
            suspended: false,
        }
    }

    /// USB unmount, bus reset or power-up: table cleared, resting light, silent (§3).
    pub fn reset_link(&mut self) {
        self.machine.clear();
        self.machine.end_test();
    }

    /// The machine's durations follow the settings.
    fn durations(&mut self) {
        let get = |k: &str| {
            self.settings
                .get(k)
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
        };
        self.machine.durations = Durations {
            needs: get("needs.duration"),
            done: get("done.duration").max(1),
            working: get("working.duration"),
        };
    }

    pub fn tick(&mut self, now: u64) {
        self.durations();
        self.machine.tick(now);
        if matches!(self.mute, Mute::Until(t) if now >= t) {
            self.mute = Mute::Off;
        }
    }

    /// The one state the whole ring shows (§3 priority); `test:<state>` during a preview.
    pub fn shown(&mut self, now: u64) -> String {
        self.tick(now);
        if self.suspended {
            return "dark".into();
        }
        match self.machine.shown(now) {
            (st, true) => format!("test:{}", st.name()),
            (st, false) => st.name().into(),
        }
    }

    /// When what's shown now started (ms; 0 for idle).
    pub fn shown_since(&mut self, now: u64) -> u64 {
        self.tick(now);
        self.machine.shown_since(now)
    }

    /// When what's shown may change by itself next (ms), if ever.
    pub fn next_change(&mut self, now: u64) -> Option<u64> {
        self.tick(now);
        self.machine.next_change(now)
    }

    pub fn lease_left_s(&self, now: u64) -> u64 {
        self.machine.lease_left_s(now)
    }

    fn play(&mut self, state: &str) {
        if self.mute != Mute::Off {
            return;
        }
        let sound = self
            .settings
            .get(&format!("{state}.sound"))
            .cloned()
            .unwrap_or_default();
        if sound != "none" && !sound.is_empty() {
            self.sounds.push(format!("{state}:{sound}"));
        }
    }

    pub fn hello_json(&self) -> String {
        json!({"name":"HumanNeeded","fw":self.fw,"board":self.board,"serial":self.serial,"proto":2})
            .to_string()
    }

    /// Handles one command line, returns exactly one reply line.
    pub fn handle(&mut self, line: &str, now: u64) -> String {
        self.tick(now);
        if line.len() > MAX_LINE {
            return "ERR line too long".into();
        }
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();
        match cmd {
            "HELLO" => self.hello_json(),
            "SNAP" => self.snap(&args, now),
            "FLASH" if args == ["done"] => {
                self.machine.flash_done(now);
                self.play("done");
                "OK".into()
            }
            "FLASH" => "ERR bad flash".into(),
            "GET" if args == ["all"] => json!(self.settings).to_string(),
            "GET" if args.len() == 1 => match self.settings.get(args[0]) {
                Some(v) => json!({ args[0]: v }).to_string(),
                None => "ERR unknown key".into(),
            },
            "SET" if args.len() == 2 => match validate(args[0], args[1]) {
                Ok(v) if self.settings.contains_key(args[0]) => {
                    self.settings.insert(args[0].into(), v);
                    "OK".into()
                }
                Ok(_) => "ERR unknown key".into(),
                Err(e) => format!("ERR {e}"),
            },
            "SCHEMA" => schema().to_string(),
            "RESET" => {
                self.settings = defaults(self.fw2);
                "OK".into()
            }
            "MUTE" if args.len() == 1 => match args[0] {
                "on" => {
                    self.mute = Mute::On;
                    "OK".into()
                }
                m => match m.parse::<u64>() {
                    Ok(0) => {
                        self.mute = Mute::Off;
                        "OK".into()
                    }
                    Ok(n) if n <= 1440 => {
                        self.mute = Mute::Until(now + n * 60_000);
                        "OK".into()
                    }
                    _ => "ERR bad minutes".into(),
                },
            },
            "TEST"
                if args.len() == 1
                    && ["needs", "done", "working", "idle", "hello"].contains(&args[0]) =>
            {
                // `hello` is the puck's own light-only hello: nothing to preview here.
                if let Some(st) = St::parse(args[0]) {
                    self.machine.start_test(st, now);
                }
                if args[0] == "needs" || args[0] == "done" {
                    self.play(args[0]);
                }
                "OK".into()
            }
            "STATUS" => {
                let shown = self.shown(now);
                let muted = match self.mute {
                    Mute::Off => "off".to_string(),
                    Mute::On => "on".to_string(),
                    Mute::Until(t) => t.saturating_sub(now).div_ceil(60_000).to_string(),
                };
                let entries: Vec<Value> = self
                    .machine
                    .entries
                    .iter()
                    .map(|(id, s)| json!({"id": id, "state": s.as_str()}))
                    .collect();
                json!({"proto":2,"lease_left":self.lease_left_s(now),"shown":shown,"muted":muted,"entries":entries})
                    .to_string()
            }
            "REBOOT" => {
                self.pending = Some(Pending::Reboot);
                "OK".into()
            }
            "BOOTSEL" => {
                self.pending = Some(Pending::Bootsel);
                "OK".into()
            }
            "SELFTEST" => {
                json!({"leds":"ok","speaker":"ok","serial":self.serial,"button":"up"}).to_string()
            }
            "" => "ERR empty".into(),
            _ => "ERR unknown command".into(),
        }
    }

    fn snap(&mut self, args: &[&str], now: u64) -> String {
        let Some(lease) = args.first().and_then(|l| l.parse::<u64>().ok()) else {
            return "ERR bad lease".into();
        };
        if !(5..=600).contains(&lease) {
            return "ERR bad lease".into();
        }
        if args.len() - 1 > MAX_ENTRIES {
            return "ERR too many entries".into();
        }
        let mut entries = Vec::new();
        for a in &args[1..] {
            let Some((id, st)) = a.split_once(':') else {
                return "ERR bad entry".into();
            };
            let Some(st) = SnapState::parse(st) else {
                return "ERR bad state".into();
            };
            if !valid_id(id) {
                return "ERR bad id".into();
            }
            entries.push((id.to_string(), st));
        }
        // Whole line validated: now apply.
        if self.machine.apply_snap(entries, lease, now).needs_sound {
            self.play("needs");
        }
        "OK".into()
    }
}

fn validate(key: &str, value: &str) -> Result<String, &'static str> {
    let field = key.rsplit('.').next().unwrap_or(key);
    let num = |lo: i64, hi: i64| -> Result<String, &'static str> {
        match value.parse::<i64>() {
            Ok(n) if (lo..=hi).contains(&n) => Ok(n.to_string()),
            _ => Err("out of range"),
        }
    };
    match field {
        "brightness" | "volume" => num(0, 100),
        "repeat" => num(1, 5),
        "duration" if key == "done.duration" => num(1, 3600),
        "duration" => num(0, 3600),
        "color" => {
            let ok = value.len() == 7
                && value.starts_with('#')
                && value[1..].chars().all(|c| c.is_ascii_hexdigit());
            if ok {
                Ok(value.to_ascii_lowercase())
            } else {
                Err("bad color")
            }
        }
        "light" if LIGHTS.contains(&value) => Ok(value.into()),
        "sound" if SOUNDS.contains(&value) => Ok(value.into()),
        "light" | "sound" => Err("bad option"),
        _ => Err("unknown key"),
    }
}

/// Same shape as firmware 0.3.0's SCHEMA (schema 1).
pub fn schema() -> Value {
    let opts =
        |v: &[&str], l: &[&str]| -> Value { v.iter().zip(l).map(|(a, b)| json!([a, b])).collect() };
    let light = opts(
        &LIGHTS,
        &[
            "Off",
            "Solid",
            "Breathe (slow)",
            "Pulse",
            "Blink",
            "Heartbeat",
            "Candle flicker",
            "Spin (LED ring)",
        ],
    );
    let sound = opts(
        &SOUNDS,
        &[
            "No sound",
            "Ding",
            "Chime (falling)",
            "Rising",
            "Arpeggio",
            "Bell",
            "Beeps",
            "Bloop",
            "Bright (melody)",
            "Clock (melody)",
            "Doorbell (melody)",
            "Fanfare (melody)",
            "Ripple (melody)",
            "Call (melody)",
            "Lullaby (melody)",
            "Harp (melody)",
        ],
    );
    let duration = opts(
        &["0", "3", "5", "10", "30", "60", "300", "600"],
        &[
            "As long as the state lasts",
            "3 seconds",
            "5 seconds",
            "10 seconds",
            "30 seconds",
            "1 minute",
            "5 minutes",
            "10 minutes",
        ],
    );
    let repeat = opts(
        &["1", "2", "3", "4", "5"],
        &["Once", "2 times", "3 times", "4 times", "5 times"],
    );
    let state = |id: &str, title: &str, desc: &str| -> Value {
        let mut fields = vec![
            json!({"key":format!("{id}.color"),"label":"Colour","type":"color"}),
            json!({"key":format!("{id}.light"),"label":"Light","type":"select","options":"light"}),
            json!({"key":format!("{id}.brightness"),"label":"Brightness","type":"range","min":0,"max":100,"unit":"%"}),
        ];
        if id != "idle" {
            fields.push(json!({"key":format!("{id}.duration"),"label":"Show light for","type":"select","options":"duration","unit":"seconds"}));
            fields.push(json!({"key":format!("{id}.sound"),"label":"Sound","type":"select","options":"sound"}));
            fields.push(json!({"key":format!("{id}.volume"),"label":"Volume","type":"range","min":0,"max":100,"unit":"%"}));
            fields.push(json!({"key":format!("{id}.repeat"),"label":"Play sound","type":"select","options":"repeat"}));
        }
        json!({"id":id,"title":title,"desc":desc,"preview":id,"swatch":format!("{id}.color"),"fields":fields})
    };
    json!({
        "schema": 1,
        "lists": {"light": light, "sound": sound, "duration": duration, "repeat": repeat},
        "groups": [
            {"id":"overall","title":"Overall","desc":"Master controls for the whole puck.","fields":[
                {"key":"brightness","label":"Brightness","type":"range","min":0,"max":100,"unit":"%"},
                {"key":"volume","label":"Volume","type":"range","min":0,"max":100,"unit":"%"}]},
            state("needs", "Needs you", "An agent is waiting for your approval or an answer."),
            state("done", "Done", "An agent finished its task."),
            state("working", "Working", "An agent is busy on your request."),
            state("idle", "Idle", "No agent is working or waiting."),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello() {
        let mut p = SimPuck::new("2.0.0");
        let h = hn_core::proto::Hello::parse(&p.handle("HELLO", 0)).unwrap();
        assert_eq!(h.proto, 2);
    }

    #[test]
    fn snap_lease_and_priority() {
        let mut p = SimPuck::new("2.0.0");
        assert_eq!(p.handle("SNAP 30 0000000a:working", 0), "OK");
        assert_eq!(p.shown(0), "working");
        assert_eq!(p.handle("SNAP 30 0000000a:working 0000000b:needs", 1), "OK");
        assert_eq!(p.shown(1), "needs");
        assert_eq!(p.sounds, vec!["needs:chime"]);
        // Same needs again: no new sound.
        p.handle("SNAP 30 0000000b:needs", 2);
        assert_eq!(p.sounds.len(), 1);
        // Lease runs out.
        assert_eq!(p.shown(30_002), "idle");
        // Lapsed lease counts as empty: the same needs sounds again.
        p.handle("SNAP 30 0000000b:needs", 30_003);
        assert_eq!(p.sounds.len(), 2);
    }

    #[test]
    fn snap_validation_is_all_or_nothing() {
        let mut p = SimPuck::new("2.0.0");
        p.handle("SNAP 30 0000000a:working", 0);
        assert!(p
            .handle("SNAP 30 0000000b:needs ZZ:working", 1)
            .starts_with("ERR"));
        assert_eq!(p.shown(1), "working");
        assert!(p.handle("SNAP 4", 1).starts_with("ERR"));
        assert!(p.handle("SNAP 601", 1).starts_with("ERR"));
        assert!(p.handle("SNAP 30 0000000a:done", 1).starts_with("ERR"));
        assert_eq!(p.handle("SNAP 30", 2), "OK");
        assert_eq!(p.shown(2), "idle");
    }

    #[test]
    fn flash_over_everything() {
        let mut p = SimPuck::new("2.0.0");
        p.handle("SNAP 30 0000000b:needs", 0);
        assert_eq!(p.handle("FLASH done", 0), "OK");
        assert_eq!(p.shown(1), "done");
        assert_eq!(p.shown(5_001), "needs");
        assert!(p.sounds.contains(&"done:ding".to_string()));
    }

    #[test]
    fn mute() {
        let mut p = SimPuck::new("2.0.0");
        p.handle("MUTE 1", 0);
        p.handle("SNAP 30 0000000b:needs", 0);
        assert!(p.sounds.is_empty());
        p.handle("SNAP 30", 1);
        p.handle("SNAP 30 0000000c:needs", 60_001);
        assert_eq!(p.sounds.len(), 1);
        assert!(p.handle("MUTE 1441", 0).starts_with("ERR"));
    }

    #[test]
    fn settings() {
        let mut p = SimPuck::new("2.0.0");
        assert_eq!(p.handle("SET needs.color #AABBCC", 0), "OK");
        assert!(p.handle("GET needs.color", 0).contains("#aabbcc"));
        assert!(p.handle("SET done.duration 0", 0).starts_with("ERR"));
        assert!(p.handle("SET idle.sound ding", 0).starts_with("ERR"));
        assert!(p.handle("SET brightness 101", 0).starts_with("ERR"));
        assert_eq!(p.handle("RESET", 0), "OK");
        assert!(p.handle("GET needs.color", 0).contains("#ff8800"));
        let s: Value = serde_json::from_str(&p.handle("SCHEMA", 0)).unwrap();
        assert_eq!(s["schema"], 1);
    }

    #[test]
    fn test_preview_ends_on_change() {
        let mut p = SimPuck::new("2.0.0");
        p.handle("TEST needs", 0);
        assert_eq!(p.shown(1), "test:needs");
        p.handle("SNAP 30", 2); // nothing changes
        assert_eq!(p.shown(3), "test:needs");
        p.handle("SNAP 30 0000000a:working", 4);
        assert_eq!(p.shown(5), "working");
        p.handle("TEST done", 6);
        assert_eq!(p.shown(10_007), "working");
    }

    #[test]
    fn status() {
        let mut p = SimPuck::new("2.0.0");
        p.handle("SNAP 30 0000000a:working", 0);
        let s: Value = serde_json::from_str(&p.handle("STATUS", 1000)).unwrap();
        assert_eq!(s["shown"], "working");
        assert_eq!(s["lease_left"], 29);
        assert_eq!(s["entries"][0]["id"], "0000000a");
    }
}
