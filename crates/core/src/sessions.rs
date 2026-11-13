//! The session table (app-events §3, D-019, D-020, D-028).
//!
//! All agents' sessions are merged here; the puck only gets the resulting snapshot. A session is
//! shown (`working`/`needs`) only while a liveness source keeps confirming it:
//! - Claude local: its `~/.claude/sessions/<pid>.json` (busy/idle, file gone) + process alive.
//! - anything else: the event's `ttl`, otherwise 2 h without an event.
//!
//! Time is passed in (ms since the Unix epoch) so every rule is unit-testable.

use crate::claude::{ClaudeSessionFile, ClaudeStatus};
use crate::event::{AgentState, Event};
use crate::ids::puck_id;
use crate::proto::{SnapState, MAX_ENTRIES};
use serde::Serialize;
use std::collections::HashMap;

/// No news for this long → idle, for agents without a better liveness source (E2, D-041).
pub const FALLBACK_IDLE_MS: u64 = 2 * 60 * 60 * 1000;
/// Claude's file must say `idle` for this long (after our last event) before we believe it.
pub const CLAUDE_IDLE_GRACE_MS: u64 = 4_000;
/// Idle sessions are forgotten after this long without an event.
pub const FORGET_IDLE_MS: u64 = 24 * 60 * 60 * 1000;
/// Ended sessions are kept this long so a late event can't bring them back.
pub const FORGET_ENDED_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Working,
    Needs,
    Idle,
    Ended,
}

impl State {
    pub fn snap(self) -> Option<SnapState> {
        match self {
            State::Working => Some(SnapState::Working),
            State::Needs => Some(SnapState::Needs),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub agent: String,
    pub session: String,
    /// The 8-hex id the puck sees.
    pub id: String,
    pub state: State,
    /// Why the session is in its state (shown in `hn status`, useful for support).
    pub reason: String,
    /// Newest event timestamp seen (out-of-order guard).
    pub last_ts: u64,
    /// Local time the last accepted event arrived.
    pub last_seen: u64,
    /// Local time the state was last set by an event.
    pub set_at: u64,
    pub ttl: Option<u64>,
    /// The Claude process this session was found in.
    pub pid: Option<u32>,
}

/// What a `done` etc. asks the puck to do besides the snapshot.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    pub flash_done: bool,
}

/// One Claude session file as seen on this tick, plus whether its process is still alive.
#[derive(Debug, Clone)]
pub struct ClaudeObservation {
    pub file: ClaudeSessionFile,
    pub alive: bool,
}

/// What the liveness sources say on this tick.
#[derive(Debug, Clone, Default)]
pub struct Observations {
    /// `None` = Claude's sessions folder couldn't be read: fall back to the generic rule.
    pub claude: Option<Vec<ClaudeObservation>>,
}

#[derive(Debug, Default)]
pub struct Table {
    sessions: HashMap<(String, String), Session>,
}

impl Table {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Applies one event. Events older than the newest one for that session are ignored.
    pub fn apply(&mut self, ev: &Event, now: u64) -> Effects {
        let key = (ev.agent.clone(), ev.session.clone());
        let mut fx = Effects::default();
        if let Some(s) = self.sessions.get(&key) {
            if ev.ts < s.last_ts {
                return fx;
            }
        } else if ev.state == AgentState::End {
            return fx;
        }
        let s = self.sessions.entry(key).or_insert_with(|| Session {
            agent: ev.agent.clone(),
            session: ev.session.clone(),
            id: puck_id(&ev.agent, &ev.session),
            state: State::Idle,
            reason: String::new(),
            last_ts: 0,
            last_seen: now,
            set_at: now,
            ttl: None,
            pid: None,
        });
        s.last_ts = ev.ts;
        s.last_seen = now;
        s.set_at = now;
        s.ttl = ev.ttl;
        let (state, reason) = match ev.state {
            AgentState::Working => (State::Working, "event working"),
            AgentState::Needs => (State::Needs, "event needs"),
            AgentState::Done => {
                fx.flash_done = true;
                (State::Idle, "event done")
            }
            AgentState::Idle => (State::Idle, "event idle"),
            AgentState::End => (State::Ended, "event end"),
        };
        s.state = state;
        s.reason = reason.into();
        fx
    }

    /// Runs the liveness rules. Returns true if any session changed state.
    pub fn check(&mut self, obs: &Observations, now: u64) -> bool {
        let mut changed = false;
        for s in self.sessions.values_mut() {
            if !matches!(s.state, State::Working | State::Needs) {
                continue;
            }
            if let Some(reason) = drop_reason(s, obs, now) {
                s.state = State::Idle;
                s.reason = reason;
                changed = true;
            }
        }
        self.sessions.retain(|_, s| match s.state {
            State::Idle => now.saturating_sub(s.last_seen) < FORGET_IDLE_MS,
            State::Ended => now.saturating_sub(s.last_seen) < FORGET_ENDED_MS,
            _ => true,
        });
        changed
    }

    /// Removes everything (e.g. "clear the puck").
    pub fn clear(&mut self) {
        for s in self.sessions.values_mut() {
            if matches!(s.state, State::Working | State::Needs) {
                s.state = State::Idle;
                s.reason = "cleared".into();
            }
        }
    }

    /// The snapshot for the puck: needs before working, most recent first, ≤ 32, then by id.
    pub fn snapshot(&self) -> Vec<(String, SnapState)> {
        let mut active: Vec<&Session> = self
            .sessions
            .values()
            .filter(|s| s.state.snap().is_some())
            .collect();
        active.sort_by(|a, b| {
            (b.state == State::Needs)
                .cmp(&(a.state == State::Needs))
                .then(b.last_seen.cmp(&a.last_seen))
        });
        active.truncate(MAX_ENTRIES);
        let mut out: Vec<(String, SnapState)> = active
            .into_iter()
            .map(|s| (s.id.clone(), s.state.snap().unwrap()))
            .collect();
        out.sort();
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }

    pub fn list(&self) -> Vec<Session> {
        let mut v: Vec<Session> = self.sessions.values().cloned().collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.last_seen));
        v
    }
}

fn drop_reason(s: &mut Session, obs: &Observations, now: u64) -> Option<String> {
    if s.agent == "claude" {
        if let Some(files) = &obs.claude {
            match files.iter().find(|o| o.file.session_id == s.session) {
                Some(o) => {
                    s.pid = Some(o.file.pid);
                    if !o.alive {
                        return Some("claude process gone".into());
                    }
                    if o.file.status == ClaudeStatus::Idle {
                        let changed_at = o.file.status_updated_at.unwrap_or(0);
                        let since = changed_at.max(s.set_at);
                        let idle_long_enough = now.saturating_sub(since) > CLAUDE_IDLE_GRACE_MS;
                        let stopped = match s.state {
                            // Stop button (no hook) or a late `working` after the turn ended.
                            State::Working => idle_long_enough,
                            // Claude also sits idle while asking (idle_prompt); only a status
                            // change *after* the question was raised means it was stopped.
                            State::Needs => changed_at > s.set_at + 1_000 && idle_long_enough,
                            _ => false,
                        };
                        if stopped {
                            return Some("claude says idle (stopped)".into());
                        }
                    }
                    // Busy, or not long enough: stays as long as Claude says so (D-020).
                    return None;
                }
                None if s.pid.is_some() => return Some("claude session file gone".into()),
                None => {} // never seen locally (cloud, or file format changed): generic rule
            }
        }
    }
    let silent = now.saturating_sub(s.last_seen);
    match s.ttl {
        Some(ttl) if silent > ttl * 1000 => Some("ttl expired".into()),
        Some(_) => None,
        None if silent > FALLBACK_IDLE_MS => Some("no news for 2 h".into()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(agent: &str, session: &str, state: AgentState, ts: u64) -> Event {
        Event {
            v: 1,
            agent: agent.into(),
            session: session.into(),
            state,
            ts,
            ttl: None,
        }
    }

    fn claude_file(
        session: &str,
        pid: u32,
        status: ClaudeStatus,
        updated: u64,
        alive: bool,
    ) -> Observations {
        Observations {
            claude: Some(vec![ClaudeObservation {
                file: ClaudeSessionFile {
                    pid,
                    session_id: session.into(),
                    proc_start: None,
                    status,
                    status_updated_at: Some(updated),
                },
                alive,
            }]),
        }
    }

    #[test]
    fn merges_agents_needs_first() {
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Working, 1), 1);
        t.apply(&ev("codex", "b", AgentState::Needs, 2), 2);
        t.apply(&ev("cursor", "c", AgentState::Working, 3), 3);
        let snap = t.snapshot();
        assert_eq!(snap.len(), 3);
        assert!(snap.contains(&(puck_id("codex", "b"), SnapState::Needs)));
    }

    #[test]
    fn done_flashes_and_drops() {
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Working, 1), 1);
        let fx = t.apply(&ev("claude", "a", AgentState::Done, 2), 2);
        assert!(fx.flash_done);
        assert!(t.snapshot().is_empty());
    }

    #[test]
    fn late_event_ignored() {
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Done, 10), 10);
        t.apply(&ev("claude", "a", AgentState::Working, 9), 11);
        assert!(t.snapshot().is_empty());
    }

    #[test]
    fn end_removes_and_blocks_late_events() {
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Needs, 1), 1);
        t.apply(&ev("claude", "a", AgentState::End, 5), 5);
        assert!(t.snapshot().is_empty());
        t.apply(&ev("claude", "a", AgentState::Working, 4), 6);
        assert!(t.snapshot().is_empty());
        // End for an unknown session creates nothing.
        t.apply(&ev("claude", "zz", AgentState::End, 5), 5);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn idle_kept_not_shown() {
        let mut t = Table::new();
        t.apply(&ev("codex", "a", AgentState::Idle, 1), 1);
        assert_eq!(t.len(), 1);
        assert!(t.snapshot().is_empty());
    }

    #[test]
    fn fallback_two_hours() {
        let mut t = Table::new();
        t.apply(&ev("codex", "a", AgentState::Working, 1), 0);
        assert!(!t.check(&Observations::default(), FALLBACK_IDLE_MS));
        assert_eq!(t.snapshot().len(), 1);
        assert!(t.check(&Observations::default(), FALLBACK_IDLE_MS + 1));
        assert!(t.snapshot().is_empty());
    }

    #[test]
    fn ttl() {
        let mut t = Table::new();
        let mut e = ev("thirdparty", "a", AgentState::Needs, 1);
        e.ttl = Some(60);
        t.apply(&e, 0);
        assert!(!t.check(&Observations::default(), 60_000));
        assert!(t.check(&Observations::default(), 60_001));
    }

    #[test]
    fn claude_busy_keeps_working_past_two_hours() {
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Working, 1), 0);
        let now = FALLBACK_IDLE_MS * 3;
        assert!(!t.check(&claude_file("a", 7, ClaudeStatus::Busy, 0, true), now));
        assert_eq!(t.snapshot().len(), 1);
    }

    #[test]
    fn claude_stop_button_clears_working() {
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Working, 1000), 1000);
        // Stop pressed at 5000: idle, no hook.
        let obs = claude_file("a", 7, ClaudeStatus::Idle, 5000, true);
        assert!(!t.check(&obs, 6000)); // within grace
        assert!(t.check(&obs, 9001));
        assert!(t.snapshot().is_empty());
    }

    #[test]
    fn claude_late_working_after_turn_end_clears() {
        let mut t = Table::new();
        // File went idle at 1000, a late `working` hook lands at 2000.
        t.apply(&ev("claude", "a", AgentState::Working, 900), 2000);
        let obs = claude_file("a", 7, ClaudeStatus::Idle, 1000, true);
        assert!(!t.check(&obs, 5000));
        assert!(t.check(&obs, 6001));
    }

    #[test]
    fn claude_idle_prompt_needs_stays() {
        let mut t = Table::new();
        // Claude went idle at 1000 (turn ended); 60 s later it asks for input (idle_prompt).
        t.apply(&ev("claude", "a", AgentState::Needs, 61_000), 61_000);
        let obs = claude_file("a", 7, ClaudeStatus::Idle, 1000, true);
        assert!(!t.check(&obs, 500_000));
        assert_eq!(t.snapshot().len(), 1);
    }

    #[test]
    fn claude_stop_during_permission_prompt_clears_needs() {
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Needs, 1000), 1000);
        let obs = claude_file("a", 7, ClaudeStatus::Idle, 10_000, true);
        assert!(!t.check(&obs, 12_000));
        assert!(t.check(&obs, 14_001));
    }

    #[test]
    fn claude_process_gone_or_file_gone() {
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Working, 1), 1);
        assert!(t.check(&claude_file("a", 7, ClaudeStatus::Busy, 0, false), 2));
        assert!(t.snapshot().is_empty());

        t.apply(&ev("claude", "b", AgentState::Working, 1), 1);
        assert!(!t.check(&claude_file("b", 8, ClaudeStatus::Busy, 0, true), 2));
        // App quit: file removed.
        assert!(t.check(
            &Observations {
                claude: Some(vec![])
            },
            3
        ));
        assert!(t.snapshot().is_empty());
    }

    #[test]
    fn claude_without_file_uses_fallback() {
        let mut t = Table::new();
        t.apply(&ev("claude", "cloud1", AgentState::Working, 1), 0);
        assert!(!t.check(
            &Observations {
                claude: Some(vec![])
            },
            1000
        ));
        assert!(t.check(
            &Observations {
                claude: Some(vec![])
            },
            FALLBACK_IDLE_MS + 1
        ));
    }

    #[test]
    fn snapshot_caps_at_32_keeping_needs() {
        let mut t = Table::new();
        for i in 0..40u64 {
            t.apply(
                &ev("codex", &format!("w{i}"), AgentState::Working, i),
                100 + i,
            );
        }
        t.apply(&ev("codex", "n", AgentState::Needs, 1), 1);
        let snap = t.snapshot();
        assert_eq!(snap.len(), MAX_ENTRIES);
        assert!(snap.iter().any(|(_, s)| *s == SnapState::Needs));
    }

    #[test]
    fn forgets_old_idle() {
        let mut t = Table::new();
        t.apply(&ev("codex", "a", AgentState::Idle, 1), 0);
        t.check(&Observations::default(), FORGET_IDLE_MS + 1);
        assert!(t.is_empty());
    }

    #[test]
    fn unknown_claude_status_keeps_state() {
        // e.g. "waiting" (seen in Claude Code 2.1.321): keep whatever the hooks said (D-045).
        let mut t = Table::new();
        t.apply(&ev("claude", "a", AgentState::Needs, 1000), 1000);
        let obs = claude_file("a", 7, ClaudeStatus::Unknown, 50_000, true);
        assert!(!t.check(&obs, 10 * 60_000));
        assert_eq!(t.snapshot().len(), 1);
    }
}
