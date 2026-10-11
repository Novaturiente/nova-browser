#!/usr/bin/env bash
# Smoke test over the IPC socket: start, open/close tabs, :set, quit cleanly.
# Needs a running Wayland or X11 session. Uses throwaway profile dirs, no network.
# Usage: tests/smoke.sh [path/to/novabrowser]   (default: target/release/novabrowser)
set -u
bin=$(realpath "${1:-target/release/novabrowser}")
tmp=$(mktemp -d /tmp/nova-smoke.XXXXXX)
export XDG_CONFIG_HOME=$tmp/config XDG_DATA_HOME=$tmp/data XDG_STATE_HOME=$tmp/state XDG_CACHE_HOME=$tmp/cache
mkdir -p $XDG_DATA_HOME/novabrowser && touch $XDG_DATA_HOME/novabrowser/welcomed   # returning user: no welcome tab
fails=0
ok()   { echo "ok   $1"; }
fail() { echo "FAIL $1"; fails=$((fails + 1)); }
q()    { "$bin" --query state 2>/dev/null; }
page() { echo "data:text/html,<title>$1</title><p>$1"; }
# wait up to $2 s for `q` output to contain $1
wait_for() { for _ in $(seq $(( ${2:-10} * 5 ))); do q | grep -q -- "$1" && return 0; sleep 0.2; done; return 1; }
ntabs() { q | grep -o '"title"' | wc -l; }

"$bin" "$(page one)" 2>"$tmp/stderr" &
pid=$!
trap 'kill $pid 2>/dev/null; rm -rf "$tmp"' EXIT

wait_for '"title":"one"' 30 && ok "starts, first tab loads" || fail "first tab (stderr: $tmp/stderr)"

"$bin" --cmd "open -t $(page two)" >/dev/null
wait_for '"title":"two"' && [ "$(ntabs)" = 2 ] && ok "open -t over IPC" || fail "open -t: $(q)"

"$bin" "$(page three)"   # plain launch while running hands over to the running instance
wait_for '"title":"three"' && ok "second launch hands over" || fail "handover: $(q)"

"$bin" --cmd "tab-close" >/dev/null
sleep 1
n=$(ntabs); [ "$n" -ge 1 ] && ! q | grep -q '"active":null' && ok "tab-close ($n tabs left)" || fail "tab-close: $(q)"

"$bin" --cmd "set theme light" >/dev/null
grep -q '"theme": *"light"' "$XDG_CONFIG_HOME/novabrowser/autoconfig.json" 2>/dev/null \
    && ok ":set persists to autoconfig.json" || fail ":set persistence"

"$bin" --cmd "quit -s" >/dev/null   # -s: save the session even with auto_save.session off
for _ in $(seq 50); do kill -0 $pid 2>/dev/null || break; sleep 0.2; done
if kill -0 $pid 2>/dev/null; then fail "quit: still running after 10 s"
else wait $pid; rc=$?; [ $rc = 0 ] && ok "quit exits 0" || fail "quit exit code $rc"; fi
grep -q "Check failed" "$tmp/stderr" && fail "CEF CHECK failure in stderr" || ok "no CEF CHECK failures"
[ -s "$XDG_DATA_HOME/novabrowser/sessions/default.json" ] && ok "session saved" || fail "quit -s saved no session"

echo; [ $fails = 0 ] && echo "smoke: all passed" || echo "smoke: $fails failed"
exit $fails
