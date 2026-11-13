//! Liveness sources (app-events §3): Claude's session files + "is this process still the one".

use hn_core::claude::parse_session_file;
use hn_core::sessions::{ClaudeObservation, Observations};
use std::path::Path;

/// Reads `<claude_dir>/sessions/*.json`. `None` if the folder can't be read at all.
pub fn observe(claude_dir: Option<&Path>) -> Observations {
    let claude = claude_dir.and_then(|dir| {
        let rd = std::fs::read_dir(dir.join("sessions")).ok()?;
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Some(file) = parse_session_file(&text) else { continue };
            let alive = process_alive(file.pid, file.proc_start.as_deref());
            out.push(ClaudeObservation { file, alive });
        }
        Some(out)
    });
    Observations { claude }
}

/// True if `pid` runs and (when we can tell) started at `proc_start`, so a reused pid of a
/// different program doesn't count.
#[cfg(windows)]
pub fn process_alive(pid: u32, proc_start: Option<&str>) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let running = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE as u32;
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        let times = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0;
        CloseHandle(h);
        if !running {
            return false;
        }
        // Claude writes the creation FILETIME (100 ns since 1601) as procStart.
        match (times, proc_start.and_then(|s| s.parse::<u64>().ok())) {
            (true, Some(expected)) => {
                let created = ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64;
                created.abs_diff(expected) <= 10_000_000 // 1 s
            }
            _ => true,
        }
    }
}

#[cfg(unix)]
pub fn process_alive(pid: u32, _proc_start: Option<&str>) -> bool {
    // procStart's format on macOS/Linux is not known yet (OPEN): pid check only.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_alive_dead_pid_is_not() {
        assert!(process_alive(std::process::id(), None));
        assert!(!process_alive(0x7fff_fff0, None));
    }

    #[cfg(windows)]
    #[test]
    fn wrong_start_time_is_not_alive() {
        assert!(!process_alive(std::process::id(), Some("1")));
    }

    #[test]
    fn reads_session_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sessions")).unwrap();
        let me = std::process::id();
        std::fs::write(
            dir.path().join(format!("sessions/{me}.json")),
            format!(r#"{{"pid":{me},"sessionId":"abc","status":"busy","statusUpdatedAt":5}}"#),
        )
        .unwrap();
        std::fs::write(dir.path().join("sessions/x.key"), "junk").unwrap();
        let obs = observe(Some(dir.path()));
        let files = obs.claude.unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].alive);
        assert_eq!(files[0].file.session_id, "abc");
        assert!(observe(Some(&dir.path().join("missing"))).claude.is_none());
    }
}
