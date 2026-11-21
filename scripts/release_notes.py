# SPDX-License-Identifier: FSL-1.1-ALv2
"""Release notes for normal users, from CHANGELOG.md (used by .github/workflows/release.yml).

    python3 scripts/release_notes.py 0.10.0 > notes.md

The "What's new" part is the version's CHANGELOG section (`## [0.10.0] - …`), or `[Unreleased]`
if it has none. Prints the notes; the title is `--title`.
"""
import re, sys
from pathlib import Path

REPO = "human-needed/app"
ROOT = Path(__file__).resolve().parents[1]


def title(version):
    return f"HumanNeeded {version}"


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
