// SPDX-License-Identifier: FSL-1.1-ALv2
//! Which agent process sent a hook (app-events 2.0 §2: Codex, Cursor → process alive).
//!
//! Codex and Cursor hooks run `curl` (via PowerShell on Windows) as a child of the agent. While
//! the connection is open, the TCP table names curl's pid; walking up its parents finds the first
//! process whose program name starts with the agent id (`codex.exe`, `Cursor.exe`). That process,
//! by pid + creation time, is then the session's liveness source.
//!
//! Windows only for now; elsewhere `None`, and such sessions use the generic ttl / 2 h rule.

use hn_core::event::AgentProc;

/// Parent chains are short (agent → shell → curl); stop well before any loop could matter.
const MAX_DEPTH: usize = 12;

/// The agent process behind the loopback connection from `client_port` to `server_port`.
pub fn agent_behind(client_port: u16, server_port: u16, agent: &str) -> Option<AgentProc> {
    let pid = imp::tcp_client_pid(client_port, server_port)?;
    find_agent_ancestor(pid, agent)
}

/// `pid` or its nearest ancestor whose program name starts with `agent`.
pub fn find_agent_ancestor(pid: u32, agent: &str) -> Option<AgentProc> {
    let table = imp::process_table()?;
    let mut cur = pid;
    let mut cur_start = imp::process_start(cur)?;
    for _ in 0..MAX_DEPTH {
        let (parent, exe) = table.iter().find(|p| p.0 == cur).map(|p| (p.1, &p.2))?;
        if program_matches(exe, agent) {
            return Some(AgentProc {
                pid: cur,
                start: cur_start,
            });
        }
        if parent == 0 || parent == cur {
            return None;
        }
        // A parent pid can be reused by a newer, unrelated process: it must be older.
        let parent_start = imp::process_start(parent)?;
        if parent_start > cur_start {
            return None;
        }
        cur = parent;
        cur_start = parent_start;
    }
    None
}

fn program_matches(exe: &str, agent: &str) -> bool {
    let base = exe
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(exe)
        .to_ascii_lowercase();
    !agent.is_empty() && base.starts_with(agent)
}

/// Creation time of a running process (Windows FILETIME), if it runs.
pub fn process_start(pid: u32) -> Option<u64> {
    imp::process_start(pid)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// The pid owning the client end (local port `client_port`, remote port `server_port`).
    pub fn tcp_client_pid(client_port: u16, server_port: u16) -> Option<u32> {
        let mut size = 0u32;
        unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                AF_INET as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
        }
        for _ in 0..3 {
            size += 4096;
            // u32 alignment for MIB_TCPTABLE_OWNER_PID.
            let mut buf = vec![0u32; size as usize / 4 + 1];
            let r = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr().cast(),
                    &mut size,
                    0,
                    AF_INET as u32,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                )
            };
            if r != 0 {
                continue; // grew in between: retry with the new size
            }
            let n = buf[0] as usize;
            let rows = unsafe {
                std::slice::from_raw_parts(buf.as_ptr().add(1).cast::<MIB_TCPROW_OWNER_PID>(), n)
            };
            // Ports are in network byte order in the low 16 bits.
            let port = |p: u32| u16::from_be(p as u16);
            return rows
                .iter()
                .find(|r| port(r.dwLocalPort) == client_port && port(r.dwRemotePort) == server_port)
                .map(|r| r.dwOwningPid);
        }
        None
    }

    /// (pid, parent pid, program name) of every process.
    pub fn process_table() -> Option<Vec<(u32, u32, String)>> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut out = Vec::new();
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut e) != 0;
            while ok {
                let len = e
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]);
                out.push((e.th32ProcessID, e.th32ParentProcessID, name));
                ok = Process32NextW(snap, &mut e) != 0;
            }
            CloseHandle(snap);
            Some(out)
        }
    }

    pub fn process_start(pid: u32) -> Option<u64> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let zero = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
            let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0;
            CloseHandle(h);
            ok.then_some(((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn tcp_client_pid(_client_port: u16, _server_port: u16) -> Option<u32> {
        None
    }
    pub fn process_table() -> Option<Vec<(u32, u32, String)>> {
        None
    }
    pub fn process_start(_pid: u32) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_names() {
        assert!(program_matches("codex.exe", "codex"));
        assert!(program_matches(r"C:\x\Cursor.exe", "cursor"));
        assert!(!program_matches("curl.exe", "cursor"));
        assert!(!program_matches("powershell.exe", ""));
    }

    #[cfg(windows)]
    #[test]
    fn finds_the_connecting_process_and_its_ancestors() {
        // This test process connects to a listener; the TCP table must name us, and the walk
        // must find us (or an ancestor) by our own program name.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = l.local_addr().unwrap().port();
        let c = std::net::TcpStream::connect(("127.0.0.1", server)).unwrap();
        let client = c.local_addr().unwrap().port();
        let me = std::process::id();
        assert_eq!(imp::tcp_client_pid(client, server), Some(me));
        let exe = std::env::current_exe().unwrap();
        let name = exe
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        let found = agent_behind(client, server, &name[..4]).unwrap();
        assert_eq!(found.pid, me);
        assert_eq!(Some(found.start), process_start(me));
        // No ancestor named like this.
        assert!(agent_behind(client, server, "no-such-agent").is_none());
    }
}
