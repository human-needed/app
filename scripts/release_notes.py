# SPDX-License-Identifier: FSL-1.1-ALv2
"""Release notes for normal users, from CHANGELOG.md (used by .github/workflows/release.yml).

    python3 scripts/release_notes.py 1.0.0 > notes.md

The "What's new" part is the version's CHANGELOG section (`## [1.0.0] - …`), or `[Unreleased]`
for a beta that has none. Prints the notes; the title is `--title`.
"""
import re, sys
from pathlib import Path

REPO = "human-needed/app"
ROOT = Path(__file__).resolve().parents[1]


def is_beta(version):
    return "-" in version


def title(version):
    return f"HumanNeeded {version}" + (" (beta)" if is_beta(version) else "")


def section(changelog, version):
    """The body of `## [version]`, else of `## [Unreleased]`."""
    parts = re.split(r"^## \[", changelog, flags=re.M)
    by_name = {}
    for p in parts[1:]:
        name, _, rest = p.partition("]")
        by_name[name] = rest.split("\n", 1)[1] if "\n" in rest else ""
    body = by_name.get(version) or by_name.get("Unreleased") or ""
    return body.strip()


def notes(version, changelog):
    base = f"https://github.com/{REPO}/releases/download/v{version}"
    lines = []
    if is_beta(version):
        lines += ["> **Beta**, for testing. It may have rough edges: please report what you find "
                  f"with **Settings → Advanced → Copy diagnostics** ([new issue](https://github.com/{REPO}/issues/new/choose)).", ""]
    lines += [
        "**Know when your AI agent needs you, without watching it.** The edges of your screen glow and "
        "a chime plays when Claude Code, Codex, Cursor or Cowork needs you or is done.",
        "",
        "## Download",
        "",
        "| Your computer | Installer |",
        "|---|---|",
        f"| Windows 10 / 11 | [HumanNeeded-windows-x64-setup.exe]({base}/HumanNeeded-windows-x64-setup.exe) |",
        f"| Mac (Apple silicon and Intel) | [HumanNeeded-macos-universal.dmg]({base}/HumanNeeded-macos-universal.dmg) |",
        "",
        "- **Already installed?** The app updates itself: when the update is ready, click "
        "**Restart to update** in the panel's Settings.",
        "- **First install:** the app isn't code-signed yet, so Windows or macOS warns you once. "
        f"[Here's how to open it](https://github.com/{REPO}#installing-an-unsigned-app).",
        f"- **Then set up your agent:** [Claude Code, Codex, Cursor or Cowork](https://human-needed.com/start/).",
        "",
    ]
    new = section(changelog, version)
    if new:
        # "### Added" reads better to users as "New".
        new = re.sub(r"^### Added$", "### New", new, flags=re.M)
        new = re.sub(r"^### Changed$", "### Improved", new, flags=re.M)
        lines += ["## What's new", "", new, ""]
    lines += ["---", "Each file's SHA-256 is in `SHA256SUMS.txt`. The source code is public under "
              f"the [Functional Source License](https://github.com/{REPO}/blob/main/LICENSE)."]
    return "\n".join(lines) + "\n"


def main():
    if len(sys.argv) < 2:
        sys.exit("usage: release_notes.py <version> [--title]")
    version = sys.argv[1].removeprefix("v")
    if "--title" in sys.argv:
        print(title(version))
        return
    sys.stdout.buffer.write(notes(version, (ROOT / "CHANGELOG.md").read_text(encoding="utf8")).encode("utf8"))


if __name__ == "__main__":
    main()
