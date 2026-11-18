# Changelog

All notable changes to the HumanNeeded app. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Works without a puck: the screen edges glow on every monitor and the computer plays the
  chimes. Turn the glow on or off from the tray menu or Settings.

### Changed
- New panel, built around your agents: each agent's last signal and switch, four state cards,
  and a Puck page. Each state has a Computer tab (screen edges, sound) and a Puck tab (light,
  sound). Settings gain screen-edge options: which screens, thickness, skip full-screen apps.
- The app now owns the light and sound settings and sends them to the puck. The first time
  your puck connects, its own settings are kept.
- The app is free and open source under the GNU GPL v3.0 or later.
- New README, contributing guide (with DCO sign-off), security policy, and issue and pull
  request templates.

### Fixed
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
