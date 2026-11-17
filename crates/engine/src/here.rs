// SPDX-License-Identifier: GPL-3.0-or-later
//! "This computer" (D-059): the screen edges and the computer's sound, driven exactly like a
//! puck. It is the puck's own state machine (`hn_sim::SimPuck`, firmware 2.0.x sounds) run inside
//! the app: it gets every snapshot, `FLASH`, `MUTE` and `TEST`, works out what to show and
//! when a sound starts. Its settings come from the app's alert settings
//! ([`crate::alerts::Alerts::computer_settings`]); it always runs, with or without a puck.

use hn_sim::SimPuck;
use std::collections::BTreeMap;

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
}

impl Default for Here {
    fn default() -> Self {
        Self::new()
    }
}

impl Here {
    pub fn new() -> Self {
        Self {
            puck: SimPuck::with_fw2_defaults("app"),
        }
    }

    /// Takes these settings (`key → value`, as on a puck); bad ones are skipped.
    pub fn configure(&mut self, settings: &BTreeMap<String, String>) {
        for (k, v) in settings {
            if self.puck.settings.get(k) != Some(v) {
                self.puck.handle(&format!("SET {k} {v}"), 0);
            }
        }
    }

    /// Runs one `hn-usb/2` command. Returns the reply line and the sounds it started.
    pub fn handle(&mut self, line: &str, now: u64) -> (String, Vec<Sound>) {
        let reply = self.puck.handle(line, now);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::Alerts;

    #[test]
    fn plays_with_levels_and_mute() {
        let mut h = Here::new();
        let mut a = Alerts::default();
        a.states
            .get_mut("needs")
            .unwrap()
            .sound
            .as_mut()
            .unwrap()
            .volume = 40;
        h.configure(&a.computer_settings());
        let (r, s) = h.handle("SNAP 30 0000000a:needs", 1);
        assert_eq!(r, "OK");
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].sound.as_str(), s[0].repeat), ("call", 1));
        assert!((s[0].gain - 0.4).abs() < 1e-6);
        h.handle("MUTE on", 2);
        assert!(h.handle("SNAP 30 0000000b:needs", 3).1.is_empty());
    }

    #[test]
    fn a_switched_off_sound_doesnt_play_but_the_state_shows() {
        let mut h = Here::new();
        let mut a = Alerts::default();
        a.states.get_mut("done").unwrap().sound.as_mut().unwrap().on = false;
        h.configure(&a.computer_settings());
        assert!(h.handle("FLASH done", 0).1.is_empty());
        assert!(h.handle("STATUS", 1).0.contains("\"shown\":\"done\""));
        // Done shows for the screen's duration (10 s by default).
        assert!(h.handle("STATUS", 10_500).0.contains("\"shown\":\"idle\""));
    }
}
