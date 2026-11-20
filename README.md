<div align="center">

<img src="docs/images/hero.gif" alt="An agent works, then needs you: the screen edges glow orange and a chime plays. Then it's done: a green glow. The HumanNeeded panel shows each agent's state." width="800">

# HUMAN_NEEDED

**Know when your AI agent needs you, without watching it.**

The edges of your screen glow and a chime plays when Claude Code, Codex, Cursor or Cowork needs
you or is done. Start a task, look away, come back when it's your turn.

<a href="https://github.com/human-needed/app/releases/latest/download/HumanNeeded-windows-x64-setup.exe"><img src="https://img.shields.io/badge/Download_for_Windows-0078D4?style=for-the-badge&logo=windows&logoColor=white" alt="Download for Windows" height="44"></a>
&nbsp;
<a href="https://github.com/human-needed/app/releases/latest/download/HumanNeeded-macos-universal.dmg"><img src="https://img.shields.io/badge/Download_for_macOS-111111?style=for-the-badge&logo=apple&logoColor=white" alt="Download for macOS" height="44"></a>

Free · Windows 10 and 11, macOS (Apple silicon and Intel) · No account · Runs on your computer

[Set up your agent](#set-up-your-agent) · [FAQ](#faq) · [All releases](https://github.com/human-needed/app/releases) · [Report a bug](https://github.com/human-needed/app/issues/new/choose)

[![Release](https://img.shields.io/github/v/release/human-needed/app?include_prereleases&label=release)](https://github.com/human-needed/app/releases)
[![License: FSL-1.1-ALv2](https://img.shields.io/badge/license-FSL--1.1--ALv2-blue)](LICENSE)

</div>

## Features

- 🟠 **Screen-edge glow** on every monitor, in the state's colour and animation. It never takes
  focus or clicks.
- 🔔 **Chimes** for "needs you" and "done", with 15 sounds and melodies to choose from.
- 🤖 **Every agent, one signal:** Claude Code, Codex, Cursor and Cowork / Claude on the web.
  "Needs you" always wins.
- 🎨 **Your look:** colour, animation and sound for each state.
- 🌙 **Quiet hours:** sounds off and the glow dimmed overnight. Mute for an hour from the tray.
- 🧹 **Clears itself:** if an agent crashes or you close it, its signal goes away on its own.
- 🔒 **Local first:** everything runs on your computer (see [Privacy](#privacy)).
- 💡 **Optional desk light:** add the [HumanNeeded puck](#the-puck-optional) to see it across
  the room.

## Install

1. **Download** the installer for your computer with the buttons above. Each
   [release](https://github.com/human-needed/app/releases/latest) has:

   | Your computer | File |
   |---|---|
   | Windows 10 / 11 (x64) | `HumanNeeded-windows-x64-setup.exe`: installs for your user, no admin needed |
   | Mac (Apple silicon and Intel) | `HumanNeeded-macos-universal.dmg`: drag HumanNeeded to Applications |

2. **Open it.** The app isn't code-signed yet, so Windows or macOS warns you once:
   [here's how to get past it](#installing-an-unsigned-app).
3. **Follow the welcome screen.** It shows you the glow and chime, finds the agents on your
   computer, and starts the app when you sign in. Then [set up your agent](#set-up-your-agent).

The app lives in the tray (Windows) or the menu bar (macOS). Click the ring icon for the panel;
right-click it for **Screen edge glow**, **Mute for 1 hour** and **Quit**. It updates itself:
new versions are checked and signed, and installed when you choose to restart.

<details>
<summary><b>Installing an unsigned app</b> (the Windows or macOS warning)</summary>

### Installing an unsigned app

HumanNeeded isn't code-signed yet, so Windows and macOS don't recognise the publisher and warn
you once. Its code is public, and each release is built on GitHub from that code. Same steps on
the site: [human-needed.com/help](https://human-needed.com/help/#unsigned).

**Windows**
1. If your browser says the file isn't commonly downloaded, choose **Keep**.
2. Open `HumanNeeded-windows-x64-setup.exe`.
3. On the blue "Windows protected your PC" screen, click **More info**, then **Run anyway**.
4. The installer runs for your user only; it doesn't ask for admin rights.

**macOS** (Apple silicon and Intel)
1. Open `HumanNeeded-macos-universal.dmg` and drag HumanNeeded to **Applications**.
2. Open HumanNeeded. macOS says it can't check it for malware: click **Done** (or **OK**).
3. Open **System Settings → Privacy & Security**, scroll down, and click **Open Anyway** next to
   HumanNeeded.
4. Confirm with **Open Anyway** and your password. You only do this once.

Only do this for files from [human-needed.com](https://human-needed.com/start/#app) or
[GitHub Releases](https://github.com/human-needed/app/releases). To check a file, compare it with
`SHA256SUMS.txt` on the release:

```powershell
certutil -hashfile HumanNeeded-windows-x64-setup.exe SHA256
```

```bash
shasum -a 256 HumanNeeded-macos-universal.dmg
```

</details>

## Set up your agent

Install the app first, then add the agents you use. The panel shows each one as **Connected**
once it has sent its first signal. Full guides: **[human-needed.com/start](https://human-needed.com/start/)**.

**Claude Code** (terminal, desktop app, VS Code): run these two commands, then start a new
session.

```bash
claude plugin marketplace add human-needed/plugins
```

```bash
claude plugin install humanneeded@humanneeded
```

To keep it up to date: in Claude Code type `/plugin` → **Marketplaces** → **humanneeded** →
**Enable auto-update**.

<details>
<summary><b>Codex</b></summary>

Add our hooks file to `~/.codex/hooks.json`:
[guide and one-line installer](https://github.com/human-needed/plugins#codex). Then restart Codex
and **approve the hooks** (`/hooks` in the CLI, or **Settings → Hooks** in the app).
</details>

<details>
<summary><b>Cursor</b></summary>

Add our hooks file to `~/.cursor/hooks.json`:
[guide and one-line installer](https://github.com/human-needed/plugins#cursor). Cursor picks it up
at once. It shows working and done (Cursor has no "needs you" hook).
</details>

<details>
<summary><b>Cowork and Claude on the web</b> (cloud)</summary>

Cloud tasks run in a cloud machine that can't reach your computer. Install the Claude Code
plugin above, then in the app turn on **Cowork** in the agents list. The app links to the
HumanNeeded relay with your Claude account (signed in to Claude on this computer).
</details>

## How it works

```mermaid
flowchart LR
    A["Claude Code · Codex · Cursor<br/>(hooks)"] -- "state events<br/>127.0.0.1:24816" --> H
    C["Cowork / Claude on the web"] -- "via the HumanNeeded relay<br/>(only if you turn it on)" --> H
    H["HumanNeeded app<br/>local hub: merges sessions,<br/>checks they're still alive"]
    H --> G["Screen-edge glow"]
    H --> S["Chime"]
    H -. "USB, optional" .-> P["Puck"]
```

1. Each agent's **hooks** post its state (working, needs you, done) to the app on `127.0.0.1`.
2. The **local hub** merges every session into one state, with "needs you" first, and clears
   sessions whose agent has gone away.
3. The app shows that state: **glow and chime** on this computer, or on the **puck** when one
   is plugged in (the glow stays on too, if you like).

## Privacy

- **What the app reads, on your computer:**
  - the state events your agents' hooks send;
  - Claude Code's session files, to tell whether a session is still alive;
  - on Windows, the agent's process, for the same reason;
  - with the cloud link on, the account ID in Claude's config files.
- **What leaves your computer:** nothing, unless you turn on the **cloud link** (off by default).
  Then the app keeps a connection to the HumanNeeded relay to receive your cloud chats' states.
  It's addressed by a hash of your Claude account ID, never by your prompts, code or files.
- **No telemetry, no accounts, no ads.**
- **Update checks:** the app downloads its update file from GitHub Releases, and, when you ask,
  the puck's firmware list from human-needed.com. Neither sends anything about you.

## The puck (optional)

The **HumanNeeded puck** is a small USB desk light with a chime, for when you're away from the
screen: across the room, on a call, making coffee. Plug it in and the app finds it. The panel then
sets its colours, animations and sounds, and updates its firmware. Everything else works without
it. More at **[human-needed.com](https://human-needed.com)**.

## FAQ

<details>
<summary><b>Is it free?</b></summary>

Yes. No account, no subscription, no ads. The code is public under the
[Functional Source License](#license).
</details>

<details>
<summary><b>Can it see my code or my prompts?</b></summary>

No. Your agents only tell it their state (working, needs you, done), and everything stays on your
computer. The optional cloud link for Cowork sends only those states. See [Privacy](#privacy).
</details>

<details>
<summary><b>Do I need the puck?</b></summary>

No. The app works on its own with the screen-edge glow and chimes. The puck is an extra.
</details>

<details>
<summary><b>Will the glow get in my way?</b></summary>

It never takes focus or clicks. It does show in screenshots and screen shares; pause it from
the tray menu before you share your screen.
By default only "Needs you" and "Done" glow, briefly for "Done". Change each state in the panel,
or turn the glow off any time from the tray menu.
</details>

<details>
<summary><b>Does it slow my computer down?</b></summary>

Barely. It idles when nothing happens; the glow animates on the GPU compositor and stops when
there's nothing to show. The audio device is only opened while a chime plays.
</details>

<details>
<summary><b>Why does Windows or macOS warn me when I install it?</b></summary>

The app isn't code-signed yet. [These steps](#installing-an-unsigned-app) get you past it once.
Signing is planned.
</details>

<details>
<summary><b>Which agents work?</b></summary>

Claude Code, Codex, Cursor, and Cowork / Claude on the web. Anything that can run a hook can post
to the app's [event API](docs/DEVELOPMENT.md#for-agent-integrations-app-events-20).
</details>

## Troubleshooting

| Problem | Fix |
|---|---|
| An agent shows **Not set up** | Install its plugin or hooks ([set up your agent](#set-up-your-agent)), then start a new session. Codex: approve the hooks. |
| Nothing lights up and you use a proxy | Add `127.0.0.1` to `NO_PROXY`. The panel shows the exact line. |
| "Agents can't reach the app" | Another program uses port 24816. Close it or restart your computer. |
| No sound | Open the state ("Needs you", "Done") in the panel and check its sound and volume; check quiet hours in Settings and "Mute for 1 hour" in the tray menu. |
| Cursor never shows "needs you" | Cursor has no hook for it yet: only working and done. |
| Windows or macOS won't open the installer or app | See [Installing an unsigned app](#installing-an-unsigned-app). |

Still stuck? In the panel, open **Settings → Advanced → Copy diagnostics**, then
[open an issue](https://github.com/human-needed/app/issues/new/choose) and paste them in.

## Uninstall

In the app, open **Settings → Advanced → Uninstall HumanNeeded…**. Choose whether to keep your
settings for a later install. It removes the app, its start-at-login entry, the cloud link's saved
credential, its log and its cache, and nothing else.

- **Windows:** you can also use **Settings → Apps → Installed apps → HumanNeeded → Uninstall**.
  Tick **Delete the application data** to remove your settings too.
- **macOS:** if you already dragged the app to the Trash, its start-at-login entry removes
  itself at your next sign-in. To remove the rest: `~/Library/Application Support/HumanNeeded`
  (settings and log), the `com.human-needed.app` folders in `~/Library/Caches` and
  `~/Library/WebKit`, and the **HumanNeeded** item in Keychain Access.

Your agents' plugins and hooks belong to the agents: remove the Claude Code plugin with
`claude plugin uninstall humanneeded@humanneeded`, and our entries from `~/.codex/hooks.json`
and `~/.cursor/hooks.json`.

## Build from source

You need [Rust](https://rustup.rs) (stable). On Linux, also Tauri's
[system packages](https://v2.tauri.app/start/prerequisites/#linux), `libudev-dev` and `libasound2-dev`.

```bash
git clone https://github.com/human-needed/app && cd app
```

```bash
cargo test --workspace
```

```bash
cargo run -p humanneeded
```

The code layout, the event API for agent integrations and the puck simulator are in
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Contributing

Bug reports, ideas and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first:
commits need a DCO sign-off (`git commit -s`). Security problems go privately through
[SECURITY.md](SECURITY.md). Release notes are in [CHANGELOG.md](CHANGELOG.md).

## License

HumanNeeded is source-available under the [Functional Source License, Version 1.1, ALv2 Future
License](LICENSE) (FSL-1.1-ALv2). You may use, copy, change and share it for any purpose except a
competing product or service. Two years after each release, that release's code also becomes
available under the Apache License 2.0. The bundled fonts (Geist, IBM Plex Mono) are under the
SIL Open Font License 1.1 (`app/ui/fonts`).

© 2026 HumanNeeded · [human-needed.com](https://human-needed.com)
