//! Integration detect / install / remove (app-events 2.0 §6, D-037, D-045, D-047), driven by
//! WS-PLUG's manifest (`hn-integrations/0`), vendored in `integrations/` and embedded at build time.
//! Hooks POST to the app's local HTTP hub; the template files are written as they are.
//!
//! Supported manifest operations:
//! - detect `any`: `{dir, os?}`, `{command}`
//! - installed: `{json, pointer, equals}`, `{hooks_json, marker}`
//! - install: `{preferred: {run}, fallback: {json_merge, set}}`,
//!   `{hooks_json_merge, from (path or {os: path}), format, after?}`
//! - remove: `{run, fallback: {json_merge, delete}}`, `{hooks_json_remove, marker}`
//!
//! File edits keep every entry that is not ours (ours = a hook command containing the marker, or
//! a 1.0 hook that ran `hn event --agent <id>`, so upgrading replaces those) and are written
//! atomically, with a one-time `.hn-backup` copy of the original.

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = include_str!("../integrations/manifest.json");
const FILES: &[(&str, &str)] = &[
    (
        "plugins/codex/hooks.json",
        include_str!("../integrations/plugins/codex/hooks.json"),
    ),
    (
        "plugins/cursor/hooks.json",
        include_str!("../integrations/plugins/cursor/hooks.json"),
    ),
    (
        "plugins/cursor/hooks.windows.json",
        include_str!("../integrations/plugins/cursor/hooks.windows.json"),
    ),
];

#[derive(Debug, Clone, Serialize)]
pub struct Integration {
    pub id: String,
    pub name: String,
    /// The agent app is on this computer.
    pub detected: bool,
    /// The HumanNeeded integration is installed and enabled.
    pub installed: bool,
    /// The app can install it with one click.
    pub can_install: bool,
    pub manual: String,
    /// What the user must still do after installing (e.g. approve hooks in Codex).
    pub after: Option<String>,
    /// Enabled in the agent's settings but not really there yet (e.g. Claude recorded the plugin
    /// but never downloaded its files): what the user must do to finish.
    pub finish: Option<String>,
    pub restart_needed: Option<String>,
}

pub struct Env {
    pub home: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub path: Option<std::ffi::OsString>,
}

impl Env {
    pub fn current() -> Self {
        Self {
            home: dirs::home_dir(),
            appdata: std::env::var_os("APPDATA").map(PathBuf::from),
            path: std::env::var_os("PATH"),
        }
    }

    /// `~/x` and `%APPDATA%/x` → absolute paths.
    fn expand(&self, p: &str) -> Option<PathBuf> {
        if let Some(rest) = p.strip_prefix("~/") {
            return Some(self.home.as_ref()?.join(rest));
        }
        if p == "~" {
            return self.home.clone();
        }
        if let Some(rest) = p.strip_prefix("%APPDATA%/") {
            return Some(self.appdata.as_ref()?.join(rest));
        }
        Some(PathBuf::from(p))
    }
}

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).expect("embedded manifest is valid JSON")
}

fn this_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Finds an executable on PATH (Windows: .exe, .cmd, .bat too).
pub fn which(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    for dir in std::env::split_paths(path?) {
        for ext in exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// `claude` on PATH, else its usual install places (GUI apps often lack the shell's PATH: native
/// installer `~/.local/bin`, npm global, Homebrew), else the copy inside the Claude desktop app
/// (newest version folder). The Store (MSIX) Claude Desktop has none of these.
fn find_command(name: &str, env: &Env) -> Option<PathBuf> {
    if let Some(p) = which(name, env.path.as_deref()) {
        return Some(p);
    }
    if name != "claude" {
        return None;
    }
    let mut known: Vec<PathBuf> = Vec::new();
    if let Some(home) = &env.home {
        known.push(home.join(".local/bin"));
        known.push(home.join(".claude/local"));
    }
    if cfg!(windows) {
        if let Some(a) = &env.appdata {
            known.push(a.join("npm"));
        }
    } else {
        known.push(PathBuf::from("/opt/homebrew/bin"));
        known.push(PathBuf::from("/usr/local/bin"));
    }
    if let Ok(dirs) = std::env::join_paths(known.iter().filter(|d| d.is_dir())) {
        if let Some(p) = which(name, Some(&dirs)) {
            return Some(p);
        }
    }
    let base = if cfg!(windows) {
        env.appdata.as_ref()?.join("Claude/claude-code")
    } else {
        env.home
            .as_ref()?
            .join("Library/Application Support/Claude/claude-code")
    };
    let exe = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let mut found: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    for ver in std::fs::read_dir(&base).ok()?.flatten() {
        let key: Vec<u32> = ver
            .file_name()
            .to_string_lossy()
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        for build in std::fs::read_dir(ver.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let p = build.path().join(exe);
            if p.is_file() {
                found.push((key.clone(), p));
            }
        }
    }
    found.sort();
    found.pop().map(|(_, p)| p)
}

fn detected(spec: &Value, env: &Env) -> bool {
    spec["any"].as_array().into_iter().flatten().any(|c| {
        if let Some(os) = c["os"].as_str() {
            if os != this_os() {
                return false;
            }
        }
        if let Some(d) = c["dir"].as_str() {
            return env.expand(d).is_some_and(|p| p.is_dir());
        }
        if let Some(cmd) = c["command"].as_str() {
            return find_command(cmd, env).is_some();
        }
        false
    })
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let backup = path.with_extension("json.hn-backup");
    if path.exists() && !backup.exists() {
        let _ = std::fs::copy(path, &backup);
    }
    let tmp = path.with_extension("json.hn-tmp");
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())? + "\n";
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn unescape(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

fn pointer_set(root: &mut Value, pointer: &str, value: Value) {
    let tokens: Vec<String> = pointer.split('/').skip(1).map(unescape).collect();
    let mut cur = root;
    for (i, t) in tokens.iter().enumerate() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let obj = cur.as_object_mut().unwrap();
        if i == tokens.len() - 1 {
            obj.insert(t.clone(), value);
            return;
        }
        cur = obj
            .entry(t.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }
}

fn pointer_delete(root: &mut Value, pointer: &str) {
    let (parent, last) = pointer.rsplit_once('/').unwrap_or(("", pointer));
    if let Some(Value::Object(m)) = root.pointer_mut(parent) {
        m.remove(&unescape(last));
    }
}

/// Splits a hook command into its program and the rest. Handles `prog`, `"prog"`, `'prog'`
/// and PowerShell's `& "prog"`.
fn split_command(cmd: &str) -> (&str, &str) {
    let mut c = cmd.trim_start();
    if let Some(rest) = c.strip_prefix("& ") {
        c = rest.trim_start();
    }
    for q in ['"', '\''] {
        if let Some(inner) = c.strip_prefix(q) {
            if let Some(end) = inner.find(q) {
                return (&inner[..end], inner[end + 1..].trim_start());
            }
        }
    }
    match c.split_once(char::is_whitespace) {
        Some((prog, rest)) => (prog, rest.trim_start()),
        None => (c, ""),
    }
}

/// A 1.0 hook (`hn event --agent <id> …`, `hn` by name, `{hn}` or any path to `hn`/`hn.exe`).
fn is_legacy_hn_hook(cmd: &str, agent: &str) -> bool {
    let (prog, rest) = split_command(cmd);
    let base = prog
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(prog)
        .to_ascii_lowercase();
    let hn = prog == "{hn}" || base == "hn" || base == "hn.exe";
    let agent_arg = format!("event --agent {agent}");
    hn && (rest == agent_arg || rest.starts_with(&format!("{agent_arg} ")))
}

/// Ours = the command contains the marker (the hub URL for this agent), or is a 1.0 `hn` hook.
fn is_ours(hook: &Value, marker: &str, agent: &str) -> bool {
    let Some(cmd) = hook["command"].as_str() else {
        return false;
    };
    cmd.contains(marker) || is_legacy_hn_hook(cmd, agent)
}

/// Every hook command in a hooks file (Claude/Codex groups or Cursor flat entries).
fn commands(file: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    for entries in file["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        for e in entries.as_array().into_iter().flatten() {
            if e.get("command").is_some() {
                out.push(e);
            }
            out.extend(e["hooks"].as_array().into_iter().flatten());
        }
    }
    out
}

/// Removes our hooks; groups left empty are dropped, as are empty event lists.
fn remove_ours(file: &mut Value, marker: &str, agent: &str) {
    let Some(events) = file.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return;
    };
    for entries in events.values_mut() {
        let Some(list) = entries.as_array_mut() else {
            continue;
        };
        list.retain_mut(|e| {
            if e.get("command").is_some() {
                return !is_ours(e, marker, agent);
            }
            if let Some(hooks) = e.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                hooks.retain(|h| !is_ours(h, marker, agent));
                return !hooks.is_empty();
            }
            true
        });
    }
    events.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
}

fn merge_hooks(file: &mut Value, from: &Value, marker: &str, agent: &str, format: &str) {
    if !file.is_object() {
        *file = json!({});
    }
    if format == "cursor" && file.get("version").is_none() {
        file["version"] = json!(1);
    }
    remove_ours(file, marker, agent);
    if !file["hooks"].is_object() {
        file["hooks"] = json!({});
    }
    for (event, entries) in from["hooks"].as_object().into_iter().flatten() {
        let list = file["hooks"]
            .as_object_mut()
            .unwrap()
            .entry(event.clone())
            .or_insert_with(|| json!([]));
        if let (Some(dst), Some(src)) = (list.as_array_mut(), entries.as_array()) {
            dst.extend(src.iter().cloned());
        }
    }
}

/// `/enabledPlugins/<plugin>` → `<plugin>`.
fn enabled_plugin(pointer: &str) -> Option<String> {
    pointer.strip_prefix("/enabledPlugins/").map(unescape)
}

/// Claude really has the plugin: `plugins/installed_plugins.json` lists it with an `installPath`
/// that holds `hooks/hooks.json` (D-PC2: enabling it in settings.json alone records it but may
/// never download the files).
fn claude_plugin_files(settings: &Path, plugin: &str) -> bool {
    let Some(dir) = settings.parent() else {
        return false;
    };
    let Some(v) = read_json(&dir.join("plugins/installed_plugins.json")) else {
        return false;
    };
    let entry = &v["plugins"][plugin];
    // v2: a list of installs (one per scope); older: one object.
    let installs: Vec<&Value> = match entry {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![entry],
        _ => vec![],
    };
    installs.iter().any(|i| {
        i["installPath"].as_str().is_some_and(|p| {
            let p = PathBuf::from(p);
            let p = if p.is_absolute() { p } else { dir.join(p) };
            p.join("hooks/hooks.json").is_file()
        })
    })
}

/// (installed, needs finishing in the agent app).
fn installed_state(a: &Value, env: &Env) -> (bool, bool) {
    let spec = &a["installed"];
    if let (Some(file), Some(ptr)) = (spec["json"].as_str(), spec["pointer"].as_str()) {
        let Some(path) = env.expand(file) else {
            return (false, false);
        };
        let enabled =
            read_json(&path).and_then(|v| v.pointer(ptr).cloned()) == Some(spec["equals"].clone());
        if !enabled {
            return (false, false);
        }
        return match enabled_plugin(ptr) {
            Some(plugin) if claude_plugin_files(&path, &plugin) => (true, false),
            Some(_) => (false, true),
            None => (true, false),
        };
    }
    (installed(a, env), false)
}

fn installed(a: &Value, env: &Env) -> bool {
    let spec = &a["installed"];
    if spec["json"].is_string() {
        return installed_state(a, env).0;
    }
    if let Some(file) = spec["hooks_json"].as_str() {
        // Installed = every hook of the current template is there, exactly. Older hooks (1.0
        // `hn`, or the `$input | curl.exe` form that re-encoded JSON on Windows) need a
        // reinstall, which replaces them.
        let Ok(template) = template_for(&a["install"]["from"]) else {
            return false;
        };
        let Some(v) = env.expand(file).and_then(|p| read_json(&p)) else {
            return false;
        };
        let have = commands(&v);
        let same = |x: &Value, y: &Value| {
            x["command"] == y["command"] && x["commandWindows"] == y["commandWindows"]
        };
        let want = commands(&template);
        return !want.is_empty() && want.iter().all(|t| have.iter().any(|h| same(t, h)));
    }
    false
}

fn run_all(cmds: &Value, env: &Env) -> Result<(), String> {
    for argv in cmds.as_array().into_iter().flatten() {
        let argv: Vec<&str> = argv
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a.as_str())
            .collect();
        let (name, args) = argv.split_first().ok_or("empty command")?;
        let exe = find_command(name, env).ok_or(format!("`{name}` wasn't found"))?;
        let mut cmd = Command::new(&exe);
        cmd.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let out = cmd
            .output()
            .map_err(|e| format!("{}: {e}", exe.display()))?;
        if !out.status.success() {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            // Adding an already-known marketplace is not a failure.
            if args.contains(&"marketplace") && text.to_lowercase().contains("already") {
                continue;
            }
            return Err(format!("`{}` failed: {}", argv.join(" "), text.trim()));
        }
    }
    Ok(())
}

fn json_edit(spec: &Value, env: &Env) -> Result<(), String> {
    let path = env
        .expand(spec["json_merge"].as_str().ok_or("bad manifest")?)
        .ok_or("no home folder")?;
    let mut v = read_json(&path).unwrap_or_else(|| json!({}));
    for (ptr, val) in spec["set"].as_object().into_iter().flatten() {
        pointer_set(&mut v, ptr, val.clone());
    }
    for ptr in spec["delete"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str())
    {
        pointer_delete(&mut v, ptr);
    }
    write_json(&path, &v)
}

fn agent(id: &str) -> Result<Value, String> {
    let m = manifest();
    m["agents"]
        .get(id)
        .cloned()
        .ok_or(format!("unknown agent {id}"))
}

fn marker_of(id: &str, a: &Value) -> String {
    a["remove"]["marker"]
        .as_str()
        .or(a["installed"]["marker"].as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("127.0.0.1:24816/v1/event/{id}/"))
}

/// `from` is a path or `{"windows": path, "macos": path, "linux": path}`.
fn template_for(from: &Value) -> Result<Value, String> {
    let path = match from {
        Value::String(p) => p.as_str(),
        Value::Object(m) => m
            .get(this_os())
            .and_then(|p| p.as_str())
            .ok_or(format!("no hooks file for {}", this_os()))?,
        _ => return Err("bad manifest".into()),
    };
    let text = FILES
        .iter()
        .find(|(p, _)| *p == path)
        .ok_or(format!("missing {path}"))?
        .1;
    serde_json::from_str(text).map_err(|e| e.to_string())
}

pub fn list(env: &Env) -> Vec<Integration> {
    let m = manifest();
    let Some(agents) = m["agents"].as_object() else {
        return vec![];
    };
    agents
        .iter()
        .map(|(id, a)| {
            let install = &a["install"];
            let can_install = install["preferred"]["run"].is_array()
                || install["fallback"]["json_merge"].is_string()
                || install["hooks_json_merge"].is_string();
            let (installed, unfinished) = installed_state(a, env);
            Integration {
                id: id.clone(),
                name: a["name"].as_str().unwrap_or(id).to_string(),
                detected: detected(&a["detect"], env),
                installed,
                finish: unfinished.then(|| FINISH_IN_CLAUDE.to_string()),
                can_install,
                manual: a["manual"].as_str().unwrap_or_default().to_string(),
                after: install["after"].as_str().map(String::from),
                restart_needed: a["restart_needed"].as_str().map(String::from),
            }
        })
        .collect()
}

/// When Claude only got the settings.json entries (no `claude` command), it may record the
/// plugin without downloading it; the user finishes in Claude's own Plugins menu.
pub const FINISH_IN_CLAUDE: &str =
    "Finish in Claude: click + next to the prompt box → Plugins → install humanneeded.";

/// One-click install. Returns a message for the user (incl. anything they still must do).
pub fn install(id: &str, env: &Env) -> Result<String, String> {
    let a = agent(id)?;
    let spec = &a["install"];
    if spec["preferred"].is_object() || spec["fallback"].is_object() {
        let preferred = match spec["preferred"]["run"].is_array() {
            true => run_all(&spec["preferred"]["run"], env),
            false => Err("no command".into()),
        };
        if let Err(e) = preferred {
            if !spec["fallback"].is_object() {
                return Err(e);
            }
            json_edit(&spec["fallback"], env)?;
            // Proven unreliable (PC2): Claude may never fetch the files. Say what's left.
            if !installed(&a, env) {
                return Ok(FINISH_IN_CLAUDE.into());
            }
        }
    } else if let Some(file) = spec["hooks_json_merge"].as_str() {
        let src = template_for(&spec["from"])?;
        let format = spec["format"].as_str().unwrap_or("");
        let path = env.expand(file).ok_or("no home folder")?;
        let mut v = read_json(&path).unwrap_or_else(|| json!({}));
        merge_hooks(&mut v, &src, &marker_of(id, &a), id, format);
        write_json(&path, &v)?;
    } else {
        return Err("not available yet".into());
    }
    let mut msg = "Installed.".to_string();
    if let Some(after) = spec["after"].as_str() {
        msg = format!("{msg} {after}");
    } else if let Some(r) = a["restart_needed"].as_str() {
        if r != "no" {
            msg = format!("{msg} Takes effect for {r}.");
        }
    }
    Ok(msg)
}

pub fn uninstall(id: &str, env: &Env) -> Result<String, String> {
    let a = agent(id)?;
    let spec = &a["remove"];
    if spec["run"].is_array() {
        if let Err(e) = run_all(&spec["run"], env) {
            if !spec["fallback"].is_object() {
                return Err(e);
            }
            json_edit(&spec["fallback"], env)?;
        }
    } else if let Some(file) = spec["hooks_json_remove"].as_str() {
        let path = env.expand(file).ok_or("no home folder")?;
        if let Some(mut v) = read_json(&path) {
            remove_ours(&mut v, &marker_of(id, &a), id);
            write_json(&path, &v)?;
        }
    } else {
        return Err("not available".into());
    }
    Ok("Removed.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(home: &Path) -> Env {
        Env {
            home: Some(home.into()),
            appdata: Some(home.join("AppData")),
            path: None,
        }
    }

    const CODEX: &str = "127.0.0.1:24816/v1/event/codex/";

    #[test]
    fn recognises_our_hooks_new_and_legacy() {
        for cmd in [
            "curl -s --data-binary @- http://127.0.0.1:24816/v1/event/codex/done || true",
            "$input | curl.exe -s http://127.0.0.1:24816/v1/event/codex/working; exit 0",
            // 1.0 hooks, removed on upgrade:
            "hn event --agent codex --state done --stdin-json",
            "{hn} event --agent codex --state done",
            r#""C:\Users\A B\AppData\Local\HumanNeeded\bin\hn.exe" event --agent codex --state done"#,
            r#"& "C:\x y\hn.exe" event --agent codex --state done"#,
            "/usr/local/bin/hn event --agent codex --state end",
        ] {
            assert!(is_ours(&json!({ "command": cmd }), CODEX, "codex"), "{cmd}");
        }
        for cmd in [
            "curl http://127.0.0.1:24816/v1/event/cursor/done",
            "hn event --agent cursor --state done",
            "hn event --agent codexx --state done",
            "other-tool done",
            "nohn event --agent codex",
        ] {
            assert!(
                !is_ours(&json!({ "command": cmd }), CODEX, "codex"),
                "{cmd}"
            );
        }
    }

    #[test]
    fn installed_hooks_post_to_the_hub() {
        let d = tempfile::tempdir().unwrap();
        let e = env(d.path());
        install("cursor", &e).unwrap();
        install("codex", &e).unwrap();
        for (f, agent) in [
            (".cursor/hooks.json", "cursor"),
            (".codex/hooks.json", "codex"),
        ] {
            let v = read_json(&d.path().join(f)).unwrap();
            let cmds = commands(&v);
            assert!(!cmds.is_empty());
            for h in cmds {
                let c = h["command"].as_str().unwrap();
                assert!(
                    c.contains(&format!("http://127.0.0.1:24816/v1/event/{agent}/")),
                    "{c}"
                );
                assert!(c.contains("--noproxy"), "{c}");
            }
        }
        let v = read_json(&d.path().join(".cursor/hooks.json")).unwrap();
        let c = v["hooks"]["stop"][0]["command"].as_str().unwrap();
        // Windows: curl.exe reads stdin itself (no `$input |`, which re-encoded the JSON).
        assert_eq!(c.starts_with("curl.exe "), cfg!(windows), "{c}");
        assert!(!c.contains("$input"), "{c}");
        assert!(c.contains("/v1/event/cursor/done"), "{c}");
    }

    #[test]
    fn upgrade_replaces_input_pipe_hooks() {
        // plugins 1279db4 piped `$input | curl.exe`: PowerShell re-encoded the JSON (BOM, CRLF).
        let d = tempfile::tempdir().unwrap();
        let cdir = d.path().join(".cursor");
        std::fs::create_dir(&cdir).unwrap();
        std::fs::write(
            cdir.join("hooks.json"),
            r#"{"version":1,"hooks":{"stop":[{"command":"$input | curl.exe -s --data-binary '@-' http://127.0.0.1:24816/v1/event/cursor/done; exit 0"}]}}"#,
        )
        .unwrap();
        let e = env(d.path());
        let is_installed = || {
            list(&e)
                .into_iter()
                .find(|i| i.id == "cursor")
                .unwrap()
                .installed
        };
        assert!(!is_installed(), "old form needs a reinstall");
        install("cursor", &e).unwrap();
        assert!(is_installed());
        let text = std::fs::read_to_string(cdir.join("hooks.json")).unwrap();
        assert!(!text.contains("$input"), "{text}");
        assert_eq!(text.matches("/v1/event/cursor/done").count(), 1, "{text}");
    }

    #[test]
    fn upgrade_replaces_hn_hooks() {
        let d = tempfile::tempdir().unwrap();
        let cdir = d.path().join(".cursor");
        std::fs::create_dir(&cdir).unwrap();
        std::fs::write(
            cdir.join("hooks.json"),
            r#"{"version":1,"hooks":{"stop":[{"command":"hn event --agent cursor --state done --stdin-json"},{"command":"mine"}]}}"#,
        )
        .unwrap();
        let e = env(d.path());
        let c = list(&e).into_iter().find(|i| i.id == "cursor").unwrap();
        assert!(!c.installed, "1.0 hooks need a reinstall");
        install("cursor", &e).unwrap();
        let v = read_json(&cdir.join("hooks.json")).unwrap();
        let stop: Vec<&str> = v["hooks"]["stop"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["command"].as_str().unwrap())
            .collect();
        assert_eq!(stop.len(), 2, "{stop:?}");
        assert!(stop.contains(&"mine"));
        assert!(stop.iter().all(|c| !c.starts_with("hn ")));
    }

    #[test]
    fn manifest_has_three_agents() {
        let d = tempfile::tempdir().unwrap();
        let l = list(&env(d.path()));
        let ids: Vec<&str> = l.iter().map(|i| i.id.as_str()).collect();
        for id in ["claude", "codex", "cursor"] {
            assert!(ids.contains(&id));
        }
        assert!(l
            .iter()
            .all(|i| !i.detected && !i.installed && i.can_install));
        assert!(l
            .iter()
            .find(|i| i.id == "codex")
            .unwrap()
            .after
            .as_deref()
            .unwrap()
            .contains("trust"));
    }

    #[test]
    fn claude_fallback_json_install_and_remove() {
        let d = tempfile::tempdir().unwrap();
        let cdir = d.path().join(".claude");
        std::fs::create_dir(&cdir).unwrap();
        std::fs::write(
            cdir.join("settings.json"),
            r#"{"model":"x","enabledPlugins":{"other@m":true,"humanneeded@humanneeded":false}}"#,
        )
        .unwrap();
        let e = env(d.path());
        let c = list(&e).into_iter().find(|i| i.id == "claude").unwrap();
        assert!(c.detected && !c.installed);
        // No `claude` command here: falls back to editing settings.json, which Claude may never
        // act on: not "installed" until the plugin files exist.
        let msg = install("claude", &e).unwrap();
        assert_eq!(msg, FINISH_IN_CLAUDE);
        let c = list(&e).into_iter().find(|i| i.id == "claude").unwrap();
        assert!(!c.installed && c.finish.as_deref() == Some(FINISH_IN_CLAUDE));
        // Claude downloads it (what its Plugins menu does): now installed.
        let ip = cdir.join("plugins/cache/humanneeded/humanneeded/2.0.0");
        std::fs::create_dir_all(ip.join("hooks")).unwrap();
        std::fs::write(ip.join("hooks/hooks.json"), "{}").unwrap();
        std::fs::write(
            cdir.join("plugins/installed_plugins.json"),
            json!({"version": 2, "plugins": {"humanneeded@humanneeded": [
                {"scope": "user", "installPath": ip, "version": "2.0.0"}]}})
            .to_string(),
        )
        .unwrap();
        let v = read_json(&cdir.join("settings.json")).unwrap();
        assert_eq!(v["model"], "x");
        assert_eq!(v["enabledPlugins"]["other@m"], true);
        assert_eq!(
            v["extraKnownMarketplaces"]["humanneeded"]["source"]["repo"],
            "human-needed/plugins"
        );
        let c = list(&e).into_iter().find(|i| i.id == "claude").unwrap();
        assert!(c.installed && c.finish.is_none());
        assert!(cdir.join("settings.json.hn-backup").exists());
        uninstall("claude", &e).unwrap();
        let v = read_json(&cdir.join("settings.json")).unwrap();
        assert!(v["enabledPlugins"].get("humanneeded@humanneeded").is_none());
        assert_eq!(v["enabledPlugins"]["other@m"], true);
    }

    #[test]
    fn codex_hooks_merge_keeps_others_and_is_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let cdir = d.path().join(".codex");
        std::fs::create_dir(&cdir).unwrap();
        std::fs::write(
            cdir.join("hooks.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other-tool done"}]}]}}"#,
        )
        .unwrap();
        let e = env(d.path());
        assert!(
            list(&e)
                .into_iter()
                .find(|i| i.id == "codex")
                .unwrap()
                .detected
        );
        let msg = install("codex", &e).unwrap();
        assert!(msg.contains("trust"), "{msg}");
        install("codex", &e).unwrap(); // twice: no duplicates
        let v = read_json(&cdir.join("hooks.json")).unwrap();
        let cmds: Vec<String> = commands(&v)
            .iter()
            .filter_map(|h| h["command"].as_str().map(String::from))
            .collect();
        assert_eq!(
            cmds.iter()
                .filter(|c| c.contains("/v1/event/codex/done"))
                .count(),
            1
        );
        assert!(cmds.contains(&"other-tool done".to_string()));
        assert!(
            list(&e)
                .into_iter()
                .find(|i| i.id == "codex")
                .unwrap()
                .installed
        );
        uninstall("codex", &e).unwrap();
        let v = read_json(&cdir.join("hooks.json")).unwrap();
        let cmds: Vec<&Value> = commands(&v);
        assert_eq!(cmds.len(), 1);
        assert!(
            !list(&e)
                .into_iter()
                .find(|i| i.id == "codex")
                .unwrap()
                .installed
        );
    }

    #[test]
    fn cursor_hooks_created_when_missing() {
        let d = tempfile::tempdir().unwrap();
        let e = env(d.path());
        install("cursor", &e).unwrap();
        let v = read_json(&d.path().join(".cursor/hooks.json")).unwrap();
        assert_eq!(v["version"], 1);
        assert!(v["hooks"]["stop"][0]["command"]
            .as_str()
            .unwrap()
            .contains("/v1/event/cursor/done"));
        assert!(
            list(&e)
                .into_iter()
                .find(|i| i.id == "cursor")
                .unwrap()
                .installed
        );
        uninstall("cursor", &e).unwrap();
        assert!(
            !list(&e)
                .into_iter()
                .find(|i| i.id == "cursor")
                .unwrap()
                .installed
        );
    }

    #[test]
    fn claude_recorded_without_files_is_not_installed() {
        // PC2: installed_plugins.json names an installPath that doesn't exist.
        let d = tempfile::tempdir().unwrap();
        let cdir = d.path().join(".claude");
        std::fs::create_dir_all(cdir.join("plugins")).unwrap();
        std::fs::write(
            cdir.join("settings.json"),
            r#"{"enabledPlugins":{"humanneeded@humanneeded":true}}"#,
        )
        .unwrap();
        std::fs::write(
            cdir.join("plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"humanneeded@humanneeded":[{"scope":"user","installPath":"plugins\\cache\\humanneeded\\humanneeded\\2.0.0-dev.2"}]}}"#,
        )
        .unwrap();
        let c = list(&env(d.path()))
            .into_iter()
            .find(|i| i.id == "claude")
            .unwrap();
        assert!(!c.installed);
        assert_eq!(c.finish.as_deref(), Some(FINISH_IN_CLAUDE));
        // Relative installPath that does exist (with hooks) counts.
        let ip = cdir.join("plugins/cache/humanneeded/humanneeded/2.0.0-dev.2/hooks");
        std::fs::create_dir_all(&ip).unwrap();
        std::fs::write(ip.join("hooks.json"), "{}").unwrap();
        if cfg!(windows) {
            assert!(
                list(&env(d.path()))
                    .into_iter()
                    .find(|i| i.id == "claude")
                    .unwrap()
                    .installed
            );
        }
    }

    #[test]
    fn finds_claude_in_known_places() {
        let d = tempfile::tempdir().unwrap();
        let e = env(d.path());
        let exe = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        std::fs::create_dir_all(d.path().join(".local/bin")).unwrap();
        std::fs::write(d.path().join(".local/bin").join(exe), "").unwrap();
        let found = find_command("claude", &e).unwrap();
        assert!(
            found.starts_with(d.path().join(".local/bin")),
            "{}",
            found.display()
        );
    }

    #[test]
    fn finds_claude_inside_desktop_app() {
        let d = tempfile::tempdir().unwrap();
        let e = env(d.path());
        let exe = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let base = if cfg!(windows) {
            d.path().join("AppData/Claude/claude-code")
        } else {
            d.path()
                .join("Library/Application Support/Claude/claude-code")
        };
        for v in ["2.1.9", "2.1.321"] {
            std::fs::create_dir_all(base.join(v).join("abc")).unwrap();
            std::fs::write(base.join(v).join("abc").join(exe), "").unwrap();
        }
        let found = find_command("claude", &e).unwrap();
        assert!(
            found.to_string_lossy().contains("2.1.321"),
            "{}",
            found.display()
        );
    }

    #[test]
    fn which_finds_executables() {
        let d = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "claude.cmd"
        } else {
            "claude"
        };
        std::fs::write(d.path().join(name), "").unwrap();
        let path = std::env::join_paths([d.path()]).unwrap();
        assert!(which("claude", Some(&path)).is_some());
        assert!(which("codex", Some(&path)).is_none());
    }
}
