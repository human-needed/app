# SPDX-License-Identifier: FSL-1.1-ALv2
"""Writes the winget manifests and the Homebrew cask for a published release.

    python packaging/make.py v1.0.0        # needs the GitHub CLI (gh), signed in

Reads the release's SHA256SUMS.txt, then writes packaging/out/:
  winget/manifests/h/HumanNeeded/HumanNeeded/<version>/   the three winget files, for a pull request
                                                           to microsoft/winget-pkgs (or `wingetcreate`)
  homebrew/Casks/humanneeded.rb                            the cask, for our tap human-needed/homebrew-tap

Each points at the release's versioned files (HumanNeeded_<version>_x64-setup.exe,
HumanNeeded_<version>_universal.dmg), which never change once published.
"""
import subprocess, sys
from pathlib import Path

REPO = "human-needed/app"
OUT = Path(__file__).resolve().parent / "out"
PUBLISHER = "HumanNeeded"
DESC = "Screen-edge glow and a chime when your AI coding agent needs you or is done"
LONG = (
    "HumanNeeded listens to your coding agents (Claude Code, Codex, Cursor, Cowork) and tells you "
    "when one is working, needs you, or is done: the edges of your screen glow in the state's "
    "colour and a short chime plays. Start a task, look away, and come back when it's your turn."
)


def sums(tag):
    text = subprocess.run(["gh", "release", "download", tag, "-R", REPO, "-p", "SHA256SUMS.txt", "-O", "-"],
                          check=True, capture_output=True, text=True).stdout
    return {name: h for h, name in (line.split() for line in text.splitlines() if line.strip())}


def winget(version, sha, url):
    d = OUT / "winget" / "manifests" / "h" / "HumanNeeded" / "HumanNeeded" / version
    d.mkdir(parents=True, exist_ok=True)
    head = "# yaml-language-server: $schema=https://aka.ms/winget-manifest.{}.1.9.0.schema.json\n"
    (d / "HumanNeeded.HumanNeeded.yaml").write_text(head.format("version") + f"""
PackageIdentifier: HumanNeeded.HumanNeeded
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.9.0
""", encoding="utf8")
    (d / "HumanNeeded.HumanNeeded.installer.yaml").write_text(head.format("installer") + f"""
PackageIdentifier: HumanNeeded.HumanNeeded
PackageVersion: {version}
InstallerType: nullsoft
Scope: user
InstallModes:
  - interactive
  - silent
UpgradeBehavior: install
Installers:
  - Architecture: x64
    InstallerUrl: {url}
    InstallerSha256: {sha.upper()}
ManifestType: installer
ManifestVersion: 1.9.0
""", encoding="utf8")
    (d / "HumanNeeded.HumanNeeded.locale.en-US.yaml").write_text(head.format("defaultLocale") + f"""
PackageIdentifier: HumanNeeded.HumanNeeded
PackageVersion: {version}
PackageLocale: en-US
Publisher: {PUBLISHER}
PublisherUrl: https://github.com/human-needed
PublisherSupportUrl: https://github.com/human-needed/app/issues
PackageName: HumanNeeded
PackageUrl: https://github.com/{REPO}
License: FSL-1.1-ALv2
LicenseUrl: https://github.com/{REPO}/blob/main/LICENSE
ShortDescription: {DESC}
Description: {LONG}
Tags:
  - ai
  - claude-code
  - codex
  - cursor
  - notifications
  - productivity
ReleaseNotesUrl: https://github.com/{REPO}/releases/tag/v{version}
ManifestType: defaultLocale
ManifestVersion: 1.9.0
""", encoding="utf8")
    return d


def cask(version, sha):
    d = OUT / "homebrew" / "Casks"
    d.mkdir(parents=True, exist_ok=True)
    f = d / "humanneeded.rb"
    f.write_text(f'''cask "humanneeded" do
  version "{version}"
  sha256 "{sha}"

  url "https://github.com/{REPO}/releases/download/v#{{version}}/HumanNeeded_#{{version}}_universal.dmg"
  name "HumanNeeded"
  desc "{DESC}"
  homepage "https://github.com/{REPO}"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on macos: ">= :catalina"

  app "HumanNeeded.app"

  uninstall quit:      "com.human-needed.app",
            launchctl: "HumanNeeded",
            delete:    "~/Library/LaunchAgents/HumanNeeded.plist"

  zap trash: [
    "~/Library/Application Support/HumanNeeded",
    "~/Library/Application Support/com.human-needed.app",
    "~/Library/Caches/com.human-needed.app",
    "~/Library/HTTPStorages/com.human-needed.app",
    "~/Library/Saved Application State/com.human-needed.app.savedState",
    "~/Library/WebKit/com.human-needed.app",
  ]

  caveats <<~EOS
    HumanNeeded isn't code-signed yet. The first time, open it from Applications, then allow it in
    System Settings → Privacy & Security → Open Anyway.
  EOS
end
''', encoding="utf8")
    return f


def main():
    if len(sys.argv) != 2 or not sys.argv[1].startswith("v"):
        sys.exit("usage: python packaging/make.py v1.0.0")
    tag = sys.argv[1]
    version = tag[1:]
    s = sums(tag)
    exe, dmg = f"HumanNeeded_{version}_x64-setup.exe", f"HumanNeeded_{version}_universal.dmg"
    missing = [n for n in (exe, dmg) if n not in s]
    if missing:
        sys.exit(f"{tag} has no {', '.join(missing)} in SHA256SUMS.txt")
    w = winget(version, s[exe], f"https://github.com/{REPO}/releases/download/{tag}/{exe}")
    c = cask(version, s[dmg])
    print("wrote", w, "and", c)


if __name__ == "__main__":
    main()
