#!/usr/bin/env bash
# S1 leftovers: start cefsimple (Wayland), run s1-test.mjs, stop, restore clipboard.
# usage: spikes/s1-wayland/run.sh [outdir]
set -u
cd "$(dirname "$0")" || exit 1
OUT=${1:-/tmp/s1}
mkdir -p "$OUT"
export CEF_PATH=$HOME/.local/share/cef LD_LIBRARY_PATH=$HOME/.local/share/cef
BIN=../../../cef-rs/target/release/cefsimple
PORT=9301

wl-paste -n > "$OUT/clip-saved.txt" 2>/dev/null; SAVED_RC=$?
wl-paste -p -n > "$OUT/primary-saved.txt" 2>/dev/null; PSAVED_RC=$?

timeout 240 env ${S1_ENV:-} "$BIN" --use-alloy-style --ozone-platform=wayland \
  --remote-debugging-port=$PORT --root-cache-path=/tmp/novabrowser-s1 \
  --url=about:blank > "$OUT/cef.log" 2>&1 &
TPID=$!
for _ in $(seq 50); do curl -s "localhost:$PORT/json/version" >/dev/null && break; sleep 0.2; done
CEF_PID=$(pgrep -P $TPID | head -1)
sleep 1.5  # let the window map
echo "timeout pid=$TPID cef pid=$CEF_PID"
CDP_PORT=$PORT CEF_PID=$CEF_PID timeout 120 node s1-test.mjs "$OUT"
echo "node rc=$?"

kill $TPID 2>/dev/null; pkill -P $TPID 2>/dev/null
wait $TPID 2>/dev/null
pgrep -f "remote-debugging-port=$PORT" && pkill -9 -f "remote-debugging-port=$PORT"
# wl-copy forks a server: keep it off our stdout so callers' pipes can close
if [ $SAVED_RC -eq 0 ]; then wl-copy < "$OUT/clip-saved.txt" >/dev/null 2>&1; echo "clipboard restored"; fi
if [ $PSAVED_RC -eq 0 ]; then wl-copy -p < "$OUT/primary-saved.txt" >/dev/null 2>&1; echo "primary restored"; fi
