// SPDX-License-Identifier: GPL-3.0-or-later
//! What the ring shows and when that changes: a line-for-line port of the firmware's `Engine`
//! (`firmware/humanneeded/core.cpp`, hn-usb/2 §3). The simulator and the app's own output
//! ("this computer": screen edges, sound) both run it, so they behave exactly like a puck:
//!
//! - shown, highest first: `TEST` preview, `FLASH done`, `needs`, `working`, `idle`;
//! - each state's `duration` runs from when that state started (0 = as long as it lasts); a new
//!   id entering `needs` starts `needs` again; once it runs out, the next state shows;
//! - `TEST <state>` previews for that state's duration, capped at 10 s, and ends at once when
//!   what would be shown under it changes;
//! - the lease running out clears the table.
//!
//! Time is passed in (ms). [`Machine::next_change`] says exactly when what's shown can change
//! next, so a caller can wake right then instead of polling.

use hn_core::proto::SnapState;

/// Longest `TEST` preview (firmware `TEST_MAX_MS`).
pub const TEST_MAX_MS: u64 = 10_000;

/// The states, as the firmware orders them (a higher one wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum St {
    Idle,
    Working,
    Done,
    Needs,
}

impl St {
    pub const ALL: [St; 4] = [St::Needs, St::Done, St::Working, St::Idle];

    pub fn name(self) -> &'static str {
        match self {
            St::Idle => "idle",
            St::Working => "working",
            St::Done => "done",
            St::Needs => "needs",
        }
    }

    pub fn parse(s: &str) -> Option<St> {
        St::ALL.into_iter().find(|st| st.name() == s)
    }
}

/// Seconds each state shows for (0 = as long as it lasts; `done` is at least 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Durations {
    pub needs: u32,
    pub done: u32,
    pub working: u32,
}

impl Durations {
    fn of(&self, st: St) -> u32 {
        match st {
            St::Needs => self.needs,
            St::Done => self.done.max(1),
            St::Working => self.working,
            St::Idle => 0,
        }
    }
}

/// What a call changed, for the caller to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Events {
    /// An id entered `needs`: play the needs sound.
    pub needs_sound: bool,
    /// A `TEST` preview was cut short by a real change: stop its sound.
    pub test_ended: bool,
    /// The lease ran out and the table was cleared.
    pub cleared: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Machine {
    /// The last snapshot (while leased).
    pub entries: Vec<(String, SnapState)>,
    leased: bool,
    lease_end: u64,
    needs_on: bool,
    working_on: bool,
    needs_since: u64,
    working_since: u64,
    done_on: bool,
    done_end: u64,
    test: Option<(St, u64)>,
    test_base: Option<St>,
    pub durations: Durations,
}

impl Machine {
    pub fn new(durations: Durations) -> Self {
        Self {
            durations,
            ..Default::default()
        }
    }

    fn within(&self, st: St, since: u64, now: u64) -> bool {
        let d = self.durations.of(st) as u64;
        d == 0 || now.saturating_sub(since) < d * 1000
    }

    /// What's shown without the `TEST` preview.
    pub fn base(&self, now: u64) -> St {
        if self.done_on {
            St::Done
        } else if self.needs_on && self.within(St::Needs, self.needs_since, now) {
            St::Needs
        } else if self.working_on && self.within(St::Working, self.working_since, now) {
            St::Working
        } else {
            St::Idle
        }
    }

    /// What's shown, and whether it is a preview. Call [`Machine::tick`] first.
    pub fn shown(&self, now: u64) -> (St, bool) {
        match self.test {
            Some((st, _)) => (st, true),
            None => (self.base(now), false),
        }
    }

    pub fn lease_left_s(&self, now: u64) -> u64 {
        if !self.leased || now >= self.lease_end {
            0
        } else {
            (self.lease_end - now).div_ceil(1000)
        }
    }

    fn check_test(&mut self, now: u64) -> bool {
        if self.test.is_some() && Some(self.base(now)) != self.test_base {
            self.test = None;
            return true;
        }
        false
    }

    /// A validated snapshot (`lease_s` seconds).
    pub fn apply_snap(
        &mut self,
        entries: Vec<(String, SnapState)>,
        lease_s: u64,
        now: u64,
    ) -> Events {
        let was_needs_ids = |id: &str| {
            self.leased
                && self
                    .entries
                    .iter()
                    .any(|(i, s)| *s == SnapState::Needs && i == id)
        };
        let new_needs = entries
            .iter()
            .any(|(id, s)| *s == SnapState::Needs && !was_needs_ids(id));
        let (was_needs, was_working) = (self.needs_on, self.working_on);
        self.needs_on = entries.iter().any(|(_, s)| *s == SnapState::Needs);
        self.working_on = entries.iter().any(|(_, s)| *s != SnapState::Needs);
        self.entries = entries;
        self.leased = true;
        self.lease_end = now + lease_s * 1000;
        if self.needs_on && (!was_needs || new_needs) {
            self.needs_since = now;
        }
        if self.working_on && !was_working {
            self.working_since = now;
        }
        Events {
            needs_sound: new_needs,
            test_ended: self.check_test(now),
            cleared: false,
        }
    }

    /// `FLASH done`: the done light for its duration.
    pub fn flash_done(&mut self, now: u64) -> Events {
        self.done_on = true;
        self.done_end = now + self.durations.of(St::Done) as u64 * 1000;
        Events {
            test_ended: self.check_test(now),
            ..Default::default()
        }
    }

    /// `TEST <state>`: a preview for the state's duration, at most 10 s.
    pub fn start_test(&mut self, st: St, now: u64) {
        let d = self.durations.of(st) as u64 * 1000;
        let ms = if d == 0 || d > TEST_MAX_MS {
            TEST_MAX_MS
        } else {
            d
        };
        self.test = Some((st, now + ms));
        self.test_base = Some(self.base(now));
    }

    /// Table cleared (lease end, unplug, power-up).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.leased = false;
        self.needs_on = false;
        self.working_on = false;
    }

    /// Ends the preview now (e.g. the user changed a setting).
    pub fn end_test(&mut self) {
        self.test = None;
    }

    /// Lease, `FLASH` and `TEST` timeouts.
    pub fn tick(&mut self, now: u64) -> Events {
        let mut ev = Events::default();
        if self.leased && now >= self.lease_end {
            self.clear();
            ev.cleared = true;
        }
        if self.done_on && now >= self.done_end {
            self.done_on = false;
        }
        if matches!(self.test, Some((_, end)) if now >= end) {
            self.test = None;
        }
        ev.test_ended = self.check_test(now);
        ev
    }

    /// The earliest time (ms) at which what's shown may change by itself: a preview or the
    /// done light ending, a state's duration running out, the lease lapsing. `None` = never.
    pub fn next_change(&self, now: u64) -> Option<u64> {
        let mut t: Vec<u64> = vec![];
        if let Some((_, end)) = self.test {
            t.push(end);
        }
        if self.done_on {
            t.push(self.done_end);
        }
        if self.leased {
            t.push(self.lease_end);
        }
        for (on, st, since) in [
            (self.needs_on, St::Needs, self.needs_since),
            (self.working_on, St::Working, self.working_since),
        ] {
            let d = self.durations.of(st) as u64;
            if on && d > 0 && since + d * 1000 > now {
                t.push(since + d * 1000);
            }
        }
        t.into_iter().filter(|&x| x > now).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(needs: u32, done: u32, working: u32) -> Durations {
        Durations {
            needs,
            done,
            working,
        }
    }
    fn e(id: &str, s: SnapState) -> (String, SnapState) {
        (id.into(), s)
    }

    // The firmware's own cases (firmware/test/core_test.cpp).
    #[test]
    fn flash_over_needs_then_needs_again() {
        let mut m = Machine::new(d(0, 3, 0));
        m.apply_snap(vec![e("0000000a", SnapState::Needs)], 30, 0);
        m.flash_done(1000);
        assert_eq!(m.shown(1000), (St::Done, false));
        assert_eq!(m.next_change(1000), Some(4000));
        m.tick(4000);
        assert_eq!(m.shown(4000), (St::Needs, false));
    }

    #[test]
    fn needs_duration_then_working_underneath() {
        let mut m = Machine::new(d(10, 3, 0));
        let ev = m.apply_snap(
            vec![
                e("0000000a", SnapState::Needs),
                e("0000000b", SnapState::Working),
            ],
            30,
            0,
        );
        assert!(ev.needs_sound);
        assert_eq!(m.base(9_999), St::Needs);
        assert_eq!(m.next_change(0), Some(10_000));
        assert_eq!(m.base(10_000), St::Working);
        // The same needs again: no sound, no restart.
        let ev = m.apply_snap(
            vec![
                e("0000000a", SnapState::Needs),
                e("0000000b", SnapState::Working),
            ],
            30,
            11_000,
        );
        assert!(!ev.needs_sound);
        assert_eq!(m.base(11_000), St::Working);
        // A new id in needs starts needs again.
        let ev = m.apply_snap(vec![e("0000000c", SnapState::Needs)], 30, 12_000);
        assert!(ev.needs_sound);
        assert_eq!(m.base(12_000), St::Needs);
    }

    #[test]
    fn test_lasts_the_state_duration_capped_at_10s() {
        let mut m = Machine::new(d(3, 5, 0));
        m.start_test(St::Needs, 0);
        assert_eq!(m.shown(0), (St::Needs, true));
        assert_eq!(m.next_change(0), Some(3_000));
        m.tick(2_999);
        assert_eq!(m.shown(2_999), (St::Needs, true));
        m.tick(3_000);
        assert_eq!(m.shown(3_000), (St::Idle, false));
        // Duration 0 (as long as it lasts): capped at 10 s.
        m.start_test(St::Working, 5_000);
        assert_eq!(m.next_change(5_000), Some(15_000));
        // Over 10 s: capped too.
        m.durations.done = 30;
        m.start_test(St::Done, 20_000);
        assert_eq!(m.next_change(20_000), Some(30_000));
    }

    #[test]
    fn test_ends_when_the_state_underneath_changes() {
        let mut m = Machine::new(d(0, 3, 0));
        m.start_test(St::Done, 0);
        // A snapshot that changes nothing keeps it.
        assert!(!m.apply_snap(vec![], 30, 100).test_ended);
        assert_eq!(m.shown(100), (St::Done, true));
        assert!(
            m.apply_snap(vec![e("0000000a", SnapState::Working)], 30, 200)
                .test_ended
        );
        assert_eq!(m.shown(200), (St::Working, false));
    }

    #[test]
    fn lease_lapses() {
        let mut m = Machine::new(d(0, 3, 0));
        m.apply_snap(vec![e("0000000a", SnapState::Needs)], 30, 0);
        assert_eq!(m.lease_left_s(1), 30);
        assert_eq!(m.next_change(0), Some(30_000));
        assert!(m.tick(30_000).cleared);
        assert_eq!(m.shown(30_000), (St::Idle, false));
        // A lapsed lease counts as empty: the same needs sounds again.
        assert!(
            m.apply_snap(vec![e("0000000a", SnapState::Needs)], 30, 30_001)
                .needs_sound
        );
    }

    #[test]
    fn nothing_pending_means_no_wakeup() {
        let m = Machine::new(d(0, 3, 0));
        assert_eq!(m.next_change(0), None);
    }
}
