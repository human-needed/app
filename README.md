# HumanNeeded app

[![tests](https://github.com/human-needed/app/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/human-needed/app/actions/workflows/ci.yml)

The per-user tray app that owns the HumanNeeded puck: it merges the sessions of every agent
(Claude Code, Codex, Cursor, …), checks they are still alive, and sends the puck one snapshot
under a lease, so the light always clears itself if anything goes wrong.

| Part | What |
|---|---|
| `crates/core` | `hn-events/1` (agent → app), host side of `hn-usb/2`, session table + liveness rules. No I/O |
| `crates/engine` | Local HTTP hub (agents' hooks post here), puck link (discovery, reconnect), SNAP every 10 s / on change, liveness, firmware update, per-agent status, proxy check. `hnd` = headless app |
| `crates/cli` | `hn`: **dev and test tool only** (talks to the app over a named pipe / Unix socket; used by the e2e tests). Agents no longer use it (app-events 2.0) and it isn't shipped |
| `crates/sim` | Puck simulator speaking `hn-usb/2` (TCP or in-process), incl. a fake RPI-RP2 drive for firmware updates |
| `app` | Tauri tray app: tray / menu-bar icon and one panel (D-055); theme tokens in `app/ui/tokens.css` |

## Test builds

Releases on the [Releases](https://github.com/human-needed/app/releases) page (built by
`.github/workflows/release.yml` from a `v*` tag). They are **unsigned**, so the OS warns once.
Check a download against `SHA256SUMS.txt` (`certutil -hashfile <file> SHA256` on Windows,
`shasum -a 256 <file>` on macOS).

- **Windows:** run `HumanNeeded_<version>_x64-setup.exe`. If SmartScreen says "Windows protected
  your PC": **More info → Run anyway**. Installs per user (no admin).
- **macOS** (Apple silicon and Intel): open the `.dmg`, drag HumanNeeded to Applications, then open
  it. If macOS says it can't check it for malware: **System Settings → Privacy & Security → Open
  Anyway** (at the bottom), then open it again. On older macOS, right-click the app → **Open** also
  works.

Not yet on macOS: liveness for Codex/Cursor uses the
2 h fallback (the process lookup is Windows-only). Start at login and the puck's USB port are
untested there.

### Tray panel (D-055)

No main window. Left-click the tray / menu-bar icon for the panel (it closes when you click
elsewhere); right-click for **Mute for 1 hour** and **Quit**. The icon is a ring in the colour the
puck shows, with a grey dot when muted and a red dot when something needs fixing.

- Home: the puck's line ("Puck · Connected", or the problem with its fix), master brightness and
  volume with one mute button, and four state tiles (light, colour, sound, "stay on for" for
  Done, Default, Preview on puck). Unplugged: only "Plug in your puck".
- Settings: agents that light the puck (per-agent switch; Cowork = the cloud link), quiet hours,
  firmware check/update, start at sign-in, auto-update, Advanced.
- Per-state brightness/volume are not in the UI: the app keeps them at 100 (a share of the master
  level, hn-usb/2 §5), so the two master sliders are the only levels.
- Quiet hours: between From and To (local time) the app mutes the puck and dims it to a quarter of
  its brightness, then puts the level back (unless you changed it). Kept in `quiet.json`.
- Agent switches are kept in `agent-switches.json`; a switched-off agent is still shown as
  connected but its events don't light the puck.
- Fonts: Geist and Geist Mono, bundled (`app/ui/fonts`, SIL OFL 1.1).

### Auto-update

From 0.9.0-beta.2 the app updates itself (Tauri updater). It checks
`releases/latest/download/latest.json` 10 s after start and every 6 h (setting: "Check for app
updates automatically" in the panel's Settings, on by default). A new version downloads in the background and is only installed when you click
**Restart to update**. Offline or failed checks are logged and retried in 30 min.

Updates are signed with the project's updater key; the app refuses any package whose signature
doesn't match the public key in `app/tauri.conf.json`. The release workflow signs with the
`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repo secrets and publishes
`latest.json`. **If that private key is lost, installed apps can never update again**; keep a backup.

Testing locally (debug builds only): `HN_UPDATE_URL=<url of a latest.json>` overrides the endpoint,
and `HN_UPDATE_TEST_INSTALL=1` installs as soon as the update is ready.

## Cloud chats (relay-api §3)

Cowork and Claude on the web run their hooks in a cloud VM that can't reach this PC. With
**Light the puck for cloud chats** on (off by default), the app keeps a WebSocket to
`wss://relay-ws.human-needed.com/v1` and shows those chats as `claude-cloud` sessions (dropped after
120 s without a message; the cloud hook sends a heartbeat every 45 s).

- Route key `K` = first 32 hex of `sha256("humanneeded-account:" + accountUuid)`. Not secret.
  The app listens on every account it finds (D-051), read again every minute: Claude Desktop's
  `config.json` `lastKnownAccountUuid` (`%APPDATA%\Claude`, the Store build's
  `%LOCALAPPDATA%\Claude-Data\*`, macOS `~/Library/Application Support/Claude`) and
  `~/.claude.json`. One connection per account, same install id.
- This install's id and secret live in the OS credential store (Windows Credential Manager entry
  `cloud-link.HumanNeeded`, macOS Keychain), never in a file. Turning the link off unbinds this PC
  and deletes them.
- Ping every 4 min; reconnect with backoff 5 → 120 s; every 3 min a probe is POSTed to `K` and must
  come back ("Working" in the window). Up to 3 PCs per account (`key_full` is shown).
- Dev overrides (only with `HN_DEV=1`): `HN_RELAY_WS`, `HN_RELAY_POST`, `HN_CLAUDE_JSON`, `HN_CLAUDE_DESKTOP_CONFIG`,
  `HN_CRED_SERVICE`.

## Build and test

```
cargo test --workspace
cargo build
```

## Run against the simulator

```
target/debug/hn-sim --listen 127.0.0.1:47800 --uf2-dir <folder>
HN_DEV=1 HN_PUCK=tcp:127.0.0.1:47800 target/debug/humanneeded
curl -H 'Content-Type: application/json' -d '{"session_id":"s1"}' http://127.0.0.1:24816/v1/event/claude/working
curl http://127.0.0.1:24816/v1/status
```

Environment: `HN_PUCK` (`auto` | `serial:COM7` | `tcp:host:port` | `sim` | `none`), `HN_HTTP_PORT`
(default 24816, 0 = off), `HN_IPC` (pipe for the `hn` dev tool), `HN_LATEST_URL`, `HN_UF2_DRIVE`
(fake drive folder), `HN_DATA_DIR`, `HN_DEV=1` (don't register start-at-login or clean up PATH).

## For agent integrations (app-events 2.0)

The user installs each agent's plugin (guides: https://human-needed.com/start/, D-054). The
panel only shows **Connected** for an agent that sent an event in the last 7 days (kept in
`agents.json` in the data folder), else **Not set up**.

Post each hook's JSON to the app (D-047), with `Content-Type: application/json`:

```
POST http://127.0.0.1:24816/v1/event/<agent>/<working|needs|done|idle|end>
```

Reply `204`. Skill endpoints: `GET /v1/status`, `POST /v1/test/<state>`, `/v1/mute/<min>`,
`/v1/clear`, `/v1/set` `{"key","value"}`. Requests with an `Origin` header or a foreign `Host`
are refused (403); a POST without a JSON content type gets 405.

Sessions of agents other than Claude live as long as the agent process that sent the hook (found
from the connection, Windows), else the `ttl` / 2 h rule. Claude events carrying `cursor_version`
are dropped (Cursor running imported Claude hooks). With `HTTP(S)_PROXY` set and no `127.0.0.1`
in `NO_PROXY`, the app shows a warning with a one-line fix (D-049).
