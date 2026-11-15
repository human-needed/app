//! Quiet hours (D-055): every day between `from` and `to` (local time) the puck's sounds are
//! muted and its light dimmed; every state still lights. Done with the puck's own commands
//! (`MUTE <min>`, `SET brightness`), so no firmware change. The level from before is kept in
//! `quiet.json` so it's put back even after a restart, unless the user changed it meanwhile.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub enabled: bool,
    /// `HH:MM`, local time.
    pub from: String,
    pub to: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            from: "22:00".into(),
            to: "08:00".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(flatten)]
    settings: Settings,
    /// Set while quiet hours are applied: (brightness before, dimmed level we set).
    #[serde(default)]
    applied: Option<(u8, u8)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuietStatus {
    #[serde(flatten)]
    pub settings: Settings,
    /// Quiet hours are on right now.
    pub active: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Dim and mute for this many minutes (until `to`).
    Enter {
        mute_min: u32,
    },
    /// Undo: put this brightness back if the puck still shows the dimmed level.
    Leave {
        restore: Option<(u8, u8)>,
    },
}

pub struct Quiet {
    file: Option<PathBuf>,
    s: Stored,
}

/// `HH:MM` → minutes after midnight.
pub fn parse_hm(t: &str) -> Option<u32> {
    let (h, m) = t.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && t.len() == 5).then_some(h * 60 + m)
}

/// Now, in minutes after local midnight.
pub fn local_minutes() -> u32 {
    use chrono::Timelike;
    let t = chrono::Local::now();
    t.hour() * 60 + t.minute()
}

/// Inside `[from, to)`, across midnight when `from > to`. `from == to` = never.
pub fn inside(now: u32, from: u32, to: u32) -> bool {
    if from <= to {
        now >= from && now < to
    } else {
        now >= from || now < to
    }
}

/// Minutes from `now` until `to` (1..=1440).
pub fn until(now: u32, to: u32) -> u32 {
    ((to + 1440 - now) % 1440).max(1)
}

/// The quiet light: a quarter of the normal level, at least 5%.
pub fn dimmed(b: u8) -> u8 {
    (b / 4).max(5).min(b)
}

/// `{"brightness":60}` (or a bare number) → 60.
pub fn level_of(reply: &str, key: &str) -> Option<u8> {
    let v: Value = serde_json::from_str(reply).ok()?;
    let x = if v.is_object() { &v[key] } else { &v };
    match x {
        Value::Number(n) => n.as_u64().map(|n| n.min(100) as u8),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

impl Quiet {
    pub fn load(file: Option<PathBuf>) -> Self {
        let s = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { file, s }
    }

    fn save(&self) -> Result<(), String> {
        let Some(f) = &self.file else { return Ok(()) };
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(f, serde_json::to_string(&self.s).unwrap_or_default())
            .map_err(|e| e.to_string())
    }

    pub fn settings(&self) -> Settings {
        self.s.settings.clone()
    }

    pub fn status(&self) -> QuietStatus {
        QuietStatus {
            settings: self.settings(),
            active: self.s.applied.is_some(),
        }
    }

    pub fn set(&mut self, q: Settings) -> Result<(), String> {
        if parse_hm(&q.from).is_none() || parse_hm(&q.to).is_none() {
            return Err("times must be HH:MM".into());
        }
        self.s.settings = q;
        self.save()
    }

    /// What to do now (`now` = minutes after local midnight).
    pub fn due(&self, now: u32) -> Action {
        let q = &self.s.settings;
        let (from, to) = (parse_hm(&q.from), parse_hm(&q.to));
        let want = q.enabled && matches!((from, to), (Some(f), Some(t)) if inside(now, f, t));
        match (want, self.s.applied) {
            (true, None) => Action::Enter {
                mute_min: until(now, to.unwrap_or(0)),
            },
            (false, Some(r)) => Action::Leave { restore: Some(r) },
            _ => Action::None,
        }
    }

    pub fn entered(&mut self, before: u8, dim: u8) {
        self.s.applied = Some((before, dim));
        let _ = self.save();
    }

    pub fn left(&mut self) {
        self.s.applied = None;
        let _ = self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(enabled: bool, from: &str, to: &str) -> Quiet {
        let mut x = Quiet::load(None);
        x.set(Settings {
            enabled,
            from: from.into(),
            to: to.into(),
        })
        .unwrap();
        x
    }

    #[test]
    fn window_across_midnight() {
        let (f, t) = (parse_hm("22:00").unwrap(), parse_hm("08:00").unwrap());
        assert!(inside(23 * 60, f, t) && inside(60, f, t) && !inside(12 * 60, f, t));
        assert!(!inside(8 * 60, f, t) && inside(22 * 60, f, t));
        assert!(inside(9 * 60, 8 * 60, 17 * 60) && !inside(18 * 60, 8 * 60, 17 * 60));
        assert!(!inside(5, 300, 300));
        assert_eq!(until(23 * 60, t), 9 * 60);
        assert_eq!(until(7 * 60 + 59, t), 1);
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("7:00"), None);
    }

    #[test]
    fn enter_then_leave() {
        let mut x = q(true, "22:00", "08:00");
        assert_eq!(x.due(12 * 60), Action::None);
        assert_eq!(x.due(23 * 60), Action::Enter { mute_min: 540 });
        x.entered(60, dimmed(60));
        assert!(x.status().active);
        assert_eq!(x.due(23 * 60 + 30), Action::None);
        assert_eq!(
            x.due(8 * 60),
            Action::Leave {
                restore: Some((60, 15))
            }
        );
        x.left();
        assert_eq!(x.due(8 * 60), Action::None);
    }

    #[test]
    fn turning_it_off_leaves_at_once_and_bad_times_are_refused() {
        let mut x = q(true, "00:00", "23:59");
        x.entered(80, 20);
        let mut s = x.settings();
        s.enabled = false;
        x.set(s).unwrap();
        assert!(matches!(x.due(600), Action::Leave { .. }));
        assert!(x
            .set(Settings {
                enabled: true,
                from: "25:00".into(),
                to: "08:00".into()
            })
            .is_err());
    }

    #[test]
    fn survives_restart_and_parses_levels() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("quiet.json");
        let mut x = Quiet::load(Some(f.clone()));
        x.set(Settings {
            enabled: true,
            from: "21:30".into(),
            to: "07:00".into(),
        })
        .unwrap();
        x.entered(70, 17);
        let y = Quiet::load(Some(f));
        assert_eq!(y.settings().from, "21:30");
        assert!(y.status().active);
        assert_eq!(level_of(r#"{"brightness":60}"#, "brightness"), Some(60));
        assert_eq!(level_of("42", "brightness"), Some(42));
        assert_eq!(dimmed(60), 15);
        assert_eq!(dimmed(10), 5);
        assert_eq!(dimmed(3), 3);
    }
}
