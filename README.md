# HumanNeeded app

The per-user tray app that owns the HumanNeeded puck: it merges the sessions of every agent
(Claude Code, Codex, Cursor, …), checks they are still alive, and sends the puck one snapshot
under a lease, so the light always clears itself if anything goes wrong.

| Part | What |
|---|---|
| `crates/core` | `hn-events/1` (agent → app), host side of `hn-usb/2`, session table + liveness rules. No I/O |
| `crates/engine` | IPC server, puck link (discovery, reconnect), SNAP every 10 s / on change, firmware update, integration manifests. `hnd` = headless app |
| `crates/cli` | `hn`: the command agent hooks call. Always exits 0 within 1 s, prints nothing for events |
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
echo '{"session_id":"s1"}' | target/debug/hn event --agent claude --state working --stdin-json
target/debug/hn status
```

Environment: `HN_PUCK` (`auto` | `serial:COM7` | `tcp:host:port` | `sim` | `none`), `HN_IPC`
(IPC endpoint), `HN_LATEST_URL`, `HN_UF2_DRIVE` (fake drive folder), `HN_DATA_DIR`,
`HN_DEV=1` (don't add `hn` to PATH or register start-at-login).

## For agent integrations

Call `hn event --agent <id> --state <working|needs|done|idle|end> --stdin-json` from each hook.
Contract: `hn-events/1` (published here when frozen).
