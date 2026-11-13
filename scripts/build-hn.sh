#!/usr/bin/env sh
# Builds the `hn` command for one or more targets, for bundling into agent plugins (D-046).
#
#   scripts/build-hn.sh                      # host target
#   scripts/build-hn.sh aarch64-apple-darwin x86_64-apple-darwin
#
# Output: dist/hn/<target>/hn[.exe] and dist/hn/hn-build.json
#   { "hn": "<version>", "contract": "hn-events/1", "files": [{ "target", "path", "sha256" }] }
# Cross targets need their toolchain (`rustup target add <t>`; Linux arm/musl: `cross`).
set -eu
cd "$(dirname "$0")/.."
CARGO="${CARGO:-cargo}"
[ $# -eq 0 ] && set -- "$(rustc -vV | sed -n 's/^host: //p')"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
CONTRACT=$(sed -n 's/^pub const CONTRACT: &str = "\(.*\)";/\1/p' crates/core/src/event.rs)
OUT=dist/hn
mkdir -p "$OUT"
FILES=""
for T in "$@"; do
  "$CARGO" build -p hn --release --target "$T"
  EXE=hn; case "$T" in *windows*) EXE=hn.exe ;; esac
  mkdir -p "$OUT/$T"
  cp "target/$T/release/$EXE" "$OUT/$T/$EXE"
  if command -v sha256sum >/dev/null; then SUM=$(sha256sum "$OUT/$T/$EXE" | cut -d' ' -f1)
  else SUM=$(shasum -a 256 "$OUT/$T/$EXE" | cut -d' ' -f1); fi
  [ -n "$FILES" ] && FILES="$FILES,"
  FILES="$FILES{\"target\":\"$T\",\"path\":\"$T/$EXE\",\"sha256\":\"$SUM\"}"
  echo "built $OUT/$T/$EXE"
done
printf '{"hn":"%s","contract":"%s","files":[%s]}\n' "$VERSION" "$CONTRACT" "$FILES" > "$OUT/hn-build.json"
cat "$OUT/hn-build.json"
