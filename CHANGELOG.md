# Changelog

All notable changes to the HumanNeeded app. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.0] - 2026-11-20

The first stable release. HumanNeeded tells you when your AI coding agent needs you or is done,
so you can look away while it works.

### New
- **Screen-edge glow** on every monitor, in each state's colour and animation (Breathe, Blink,
  Heartbeat, Candle flicker, Spin or Solid). It never takes focus or clicks.
- **Chimes** for "Needs you" and "Done", with 15 sounds and melodies, each with its own volume.
- **Every agent, one signal:** Claude Code, Codex, Cursor, and Claude Cowork and other cloud
  agents (turn on **Cloud agents**). "Needs you" always wins.
- **A welcome screen** on first run: see the glow and chime, find the agents on your computer,
  and start the app when you sign in.
- **Your look per state:** screen edges and sound on this computer, and light and sound on the
  optional Puck desk light. Durations from 1 second to "until you answer".
- **Quiet hours:** no sounds and a softer glow overnight. Mute for an hour from the tray.
- **Updates itself:** signed updates download in the background and install when you're not
  using it.
- **Copy diagnostics** for bug reports, and a clean **Uninstall** (keep your settings or not),
  both in Settings → Advanced.
- Free, with its source code public under the Functional Source License (FSL-1.1-ALv2).

### Since the last beta
- The panel's final design: **Connect your agents** opens the setup guides, and one **Cloud
  agents** switch replaces the list of agents.
- Updates now install themselves at a quiet moment instead of waiting for "Restart to update".
- Windows: "Start when I sign in" now turns on after the first run, also when the app runs as an
  administrator. macOS: its login item removes itself if you drag the app to the Trash.
- The Pulse light is gone: anything set to Pulse now uses Breathe, which looked almost the same.
- Every light runs in whole 1-second cycles that start and end at full brightness, with 250 ms
  fades, and the screen edges cross-fade from one state to the next, like the Puck.

## [0.9.0-beta.6] - 2026-11-17

### Changed
- New brand: the HUMAN_NEEDED wordmark, IBM Plex Mono for numbers and values, and an
  8-segment ring as the tray icon.

## [0.9.0-beta.5] - 2026-11-16

### Added
- Every light animation is offered for each state: Blink, Heartbeat, Candle flicker and Spin
  are back.

## [0.9.0-beta.4] - 2026-11-15

### Added
- A tray-only app: one panel opens from the tray or menu-bar icon.
- A switch per agent, and quiet hours (sounds off and the light dimmed overnight).

### Changed
- Brightness and volume are set once for the whole puck.

## [0.9.0-beta.3] - 2026-11-15

### Added
- Cloud link: Cowork and Claude on the web light the puck too, through the HumanNeeded relay
  (off by default).
- Each agent's status (connected or not set up) in the panel.

## [0.9.0-beta.2] - 2026-11-14

### Added
- The app updates itself. Updates are signed and only installed when you restart.

## [0.9.0-beta.1] - 2026-11-14

### Added
- First test builds: a Windows installer and a macOS universal app.

[Unreleased]: https://github.com/human-needed/app/compare/v0.9.0-beta.6...HEAD
[0.9.0-beta.6]: https://github.com/human-needed/app/compare/v0.9.0-beta.5...v0.9.0-beta.6
[0.9.0-beta.5]: https://github.com/human-needed/app/compare/v0.9.0-beta.4...v0.9.0-beta.5
[0.9.0-beta.4]: https://github.com/human-needed/app/compare/v0.9.0-beta.3...v0.9.0-beta.4
[0.9.0-beta.3]: https://github.com/human-needed/app/compare/v0.9.0-beta.2...v0.9.0-beta.3
[0.9.0-beta.2]: https://github.com/human-needed/app/compare/v0.9.0-beta.1...v0.9.0-beta.2
[0.9.0-beta.1]: https://github.com/human-needed/app/releases/tag/v0.9.0-beta.1
