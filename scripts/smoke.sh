#!/usr/bin/env bash
# SPDX-License-Identifier: FSL-1.1-ALv2
# Startup smoke run: launches the built app with no puck and its own data folder, then checks
# that the main thread runs ("ui: ready" in app.log) and the hub answers. Catches a hang at
# startup (0.9.0-beta.7). Usage: scripts/smoke.sh <path to humanneeded binary>
set -u
bin="$1"
dir="$(mktemp -d)"
port=24899
export HN_DATA_DIR="$dir" HN_PUCK=none HN_HTTP_PORT=$port HN_DEV=1
"$bin" --background >"$dir/out.txt" 2>&1 &
pid=$!
log="$dir/logs/app.log"
ok=""
for _ in $(seq 1 40); do
  if grep -q "ui: ready" "$log" 2>/dev/null; then ok=1; break; fi
  if ! kill -0 "$pid" 2>/dev/null; then echo "the app exited"; break; fi
  sleep 0.5
done
hub="$(curl -s -m 3 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/v1/status" || true)"
kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null
echo "--- app.log"; cat "$log" 2>/dev/null; echo "--- output"; tail -20 "$dir/out.txt"
if [ -z "$ok" ]; then echo "FAIL: no 'ui: ready' within 20 s (main thread blocked?)"; exit 1; fi
if [ "$hub" != "200" ]; then echo "FAIL: hub answered $hub"; exit 1; fi
echo "OK: main thread ready and hub answers"
