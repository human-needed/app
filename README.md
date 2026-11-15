# HumanNeeded app

[![tests](https://github.com/human-needed/app/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/human-needed/app/actions/workflows/ci.yml)

The per-user tray app that owns the HumanNeeded puck: it merges the sessions of every agent
(Claude Code, Codex, Cursor, …), checks they are still alive, and sends the puck one snapshot
under a lease, so the light always clears itself if anything goes wrong.

| Part | What |
|---|---|
| `crates/core` | `hn-events/1` (agent → app), host side of `hn-usb/2`, session table + liveness rules. No I/O |
| `crates/engine` | Local HTTP hub (agents' hooks post here), puck link (discovery, reconnect), SNAP every 10 s / on change, liveness, firmware update, integration manifests, proxy check. `hnd` = headless app |
| `crates/cli` | `hn`: **dev and test tool only** (talks to the app over a named pipe / Unix socket; used by the e2e tests). Agents no longer use it (app-events 2.0) and it isn't shipped |
| `crates/sim` | Puck simulator speaking `hn-usb/2` (TCP or in-process), incl. a fake RPI-RP2 drive for firmware updates |
| `app` | Tauri tray app + settings window (form built from the puck's `SCHEMA`) |

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
