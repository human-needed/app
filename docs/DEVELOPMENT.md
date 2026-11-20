# HumanNeeded app: development notes

For people working on the app. The user-facing overview is the [README](../README.md).

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

## Releases

Releases on the [Releases](https://github.com/human-needed/app/releases) page (built by
`.github/workflows/release.yml` from a `v*` tag). They are **unsigned**, so the OS warns once.

**Channels.** A stable version (`v1.0.0`) is published as "latest": every installed app, betas
included, updates to it through `releases/latest/download/latest.json`. From 1.0 on, a beta
(`v1.1.0-beta.1`) is published as a pre-release, which is never "latest", so stable users don't
get it; testers install it by hand. Betas before 1.0 (`v0.9.0-beta.N`) stay "latest".

**Release notes** come from `CHANGELOG.md` (`scripts/release_notes.py`): the version's section,
or `[Unreleased]` for a beta without one, under a download table and the first-install steps.
Write changelog lines for users: what they'll notice, not how it's built.

**Releasing 1.0.0** (or any stable version):
1. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [1.0.0] - <date>` and add a new empty
   `## [Unreleased]` above it.
2. Set the version in `app/tauri.conf.json`, `Cargo.toml` and `Cargo.lock` (`cargo update -w`).
3. Merge, then tag `v1.0.0` on main and push the tag. The workflow checks the tag matches the
   version, builds, signs the updates and publishes.
4. After it's published: `python packaging/make.py v1.0.0` for winget and Homebrew
   (`packaging/README.md`).

### Installing a test build
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

### Tray panel (D-059 app-first, WS-DESIGN rounds 14–16)

No main window. Left-click the tray / menu-bar icon for the panel (it closes when you click
elsewhere); right-click for **Screen edge glow** (pause), **Mute for 1 hour** and **Quit**. The
icon is a ring in the shown state's colour, with a grey dot when muted and a red dot when
something needs fixing (port in use, proxy, puck firmware too old). No puck is not a problem.

- Main: one row per agent (logo, last signal such as "Needs you · 1 min ago", on/off switch, or
  **Set up**; Cowork's switch is the cloud link), four state cards, then **Puck** (Connected / Not
  connected) and **Settings**.
- State page: two tabs. **Computer** has Screen edges (light, colour, brightness, duration) and
  Sound; **Puck** has Light and Sound, and is disabled without a puck. Each section has a switch;
  a switched-off section stays visible, dimmed. Only Needs you and Done have sounds (the
  firmware never plays one for Working or Idle). **Default** and **Preview** at the bottom.
- Settings: Screen edges (all screens or the main one, thickness, skip full-screen apps), quiet
  hours, app, Advanced.
- Puck page: connected = Test, firmware check/update, Advanced (port, board, serial, restart,
  clear); not connected = "Plug in your Puck" and "Don't have one? See the Puck".

Settings files (in the data folder):
- `alerts.json`: the app's alert settings per state (`crates/engine/src/alerts.rs`). The app owns
  them: this computer's screen edges and sound follow them, and the puck gets its part pushed
  on every connect and change (only changed keys; master levels 100, the levels live on each
  state). The first time a puck connects, its own settings are imported instead (existing
  users), and where it chimes the computer stays quiet.
- `glow.json`: pause, screens, thickness, skip full-screen apps.
- `quiet.json`: between From and To (local time) sounds are muted, the screen edges dim to a
  quarter and the puck's master level too (put back after, unless you changed it).
- `agent-switches.json`: a switched-off agent is still shown but its events don't alert.
- The hub's `/v1/set` and `hn set` (puck-style keys, app-events unchanged) set that field for
  both the screen and the puck.
- Fonts (D-058): Geist for text and headings, IBM Plex Mono for numbers, values and the
  HUMAN_NEEDED wordmark; bundled (`app/ui/fonts`, SIL OFL 1.1). Agent logos in `app/ui/logos`
  (from `@lobehub/icons-static-svg`, MIT; the marks belong to their owners).

### Auto-update

From 0.9.0-beta.2 the app updates itself (Tauri updater). It checks
`releases/latest/download/latest.json` 10 s after start and every 6 h (setting: "Check for app
updates automatically" in the panel's Settings, on by default). A new version downloads in the background. With automatic updates on, it
installs at the next quiet moment (checked every minute: nothing shown on this computer and the
panel closed), and the app restarts in the background (an `updated` marker in the data folder
keeps the panel from popping up). **Restart to update** in Settings installs it at once. Offline or failed checks are logged and retried in 30 min.

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
