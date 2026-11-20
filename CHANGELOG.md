# Changelog

All notable changes to the HumanNeeded app. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Connect your agents** opens the setup guides on human-needed.com/start.
- When an update is ready, the panel says so at the top: **Update now** installs it and restarts.
  It still installs by itself at a quiet moment.
- Every state card says the same kind of thing: "Edges", "Sound", "Edges · Sound", "Puck only"
  or "Off" (no durations).
- Settings → Advanced → Uninstall HumanNeeded: removes the app, its login item, the cloud
  link's credential, log and cache, and your settings unless you keep them. The Windows
  uninstaller now cleans up the same way.
- Settings → Advanced → Copy diagnostics: versions, agents, the Puck and the last log lines,
  ready to paste into a bug report (no keys or session ids).
- Works without a puck: the screen edges glow on every monitor and the computer plays the
  chimes. Turn the glow on or off from the tray menu or Settings.

### Changed
- The cloud link checks itself with the relay every 30 minutes instead of posting a test
  message every 3 minutes, and pings every 9 minutes: less traffic, and a broken link is still
  noticed and reconnected.
- The panel's final design: a "Connect your agents" button opens the setup guides, one
  "Cloud agents" switch turns on Cowork and other cloud agents, then the four states, the Puck
  and Settings.
- New panel, built around your agents: each agent's last signal and switch, four state cards,
  and a Puck page. Each state has a Computer tab (screen edges, sound) and a Puck tab (light,
  sound). Settings gain screen-edge options: which screens, thickness, skip full-screen apps.
- The app now owns the light and sound settings and sends them to the puck. The first time
  your puck connects, its own settings are kept.
- The app is free, with its source code public under the Functional Source License
  (FSL-1.1-ALv2): any use except a competing product, and each release becomes Apache-2.0 after
  two years.
- New README, contributing guide (with DCO sign-off), security policy, and issue and pull
  request templates.

### Fixed
- A preview of a state that lasts "while it works" or "until you answer" now runs for 5 seconds
  (it ran for a minute), on the screen and the Puck. **Stop** ends a preview at once, and so does
  changing a setting.
- Automatic updates now install themselves: a downloaded update installs when nothing is
  showing and the panel is closed, and the app comes back in the background. Before, it waited
  for you to click "Restart to update".
- Windows: "Start when I sign in" is on after the first run, as intended, also when the app runs
  as an administrator. It's for your user only, and Task Manager's Startup apps switch is
  respected.
- macOS: if you drag the app to the Trash, its start-at-login entry removes itself at your next
  sign-in instead of failing every time.
- The screen-edge glow keeps each state's duration exactly, like the puck (a 3 s preview no
  longer lasts about 15 s).
- Dropdowns in the panel no longer close while you choose.
- A preview on this computer lasts exactly the duration you set, like the real alert.
- The screen-edge glow's animations (pulse, breathe, spin and the others) now play; before,
  the glow only faded in and out. The glow now shows in screenshots and screen shares (hiding
  it from capture is what stopped the animations on Windows).
- The glow animates even when Windows' "Animation effects" setting is off.
- Every light animation now takes exactly 1 second and repeats, so it finishes a whole cycle
  at the end of any duration you pick.
- A duration is now the light effect alone: the light fades in for 250 ms, runs that many
  seconds, then fades out for 250 ms.
- New 1 second and 2 second durations for the screen edges and the Puck.
- Breathe, Blink, Heartbeat and Candle flicker now start and end each cycle at full
  brightness, so a light fades in to full and fades out from full.
- When one state changes straight to another, the screen edges cross-fade in 250 ms, like the
  Puck, instead of switching at once.

### Removed
- The Pulse light. Anything set to Pulse now uses Breathe, which looked almost the same.

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
