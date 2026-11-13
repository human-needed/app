//! Where `hn` finds the app (app-events §1). `HN_IPC` overrides it (tests, several instances).
//!
//! - Windows: named pipe `\\.\pipe\humanneeded-<user SID>`.
//! - macOS: `~/Library/Application Support/HumanNeeded/hn.sock`.
//! - Linux: `$XDG_RUNTIME_DIR/humanneeded/hn.sock`.

use std::path::PathBuf;

pub fn ipc_endpoint() -> PathBuf {
    if let Some(v) = std::env::var_os("HN_IPC") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    default_endpoint()
}

#[cfg(windows)]
fn default_endpoint() -> PathBuf {
    let sid = win::current_user_sid().unwrap_or_else(|| "unknown".into());
    PathBuf::from(format!(r"\\.\pipe\humanneeded-{sid}"))
}

#[cfg(target_os = "macos")]
fn default_endpoint() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    home.join("Library/Application Support/HumanNeeded/hn.sock")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_endpoint() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/humanneeded-{}", unsafe_uid())));
    dir.join("humanneeded/hn.sock")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unsafe_uid() -> String {
    std::env::var("UID")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".into())
}

#[cfg(windows)]
pub mod win {
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// The current user's SID as `S-1-5-21-…`.
    pub fn current_user_sid() -> Option<String> {
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
            let mut buf = vec![0u8; len as usize];
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
            CloseHandle(token);
            if ok == 0 {
                return None;
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut wide: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut wide) == 0 {
                return None;
            }
            let mut n = 0;
            while *wide.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(wide, n));
            LocalFree(wide.cast());
            Some(s)
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn sid_looks_right() {
        let sid = super::win::current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
    }
}
