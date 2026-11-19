#!/usr/bin/env bash
# UI screenshot helper: start a throwaway instance on a few colourful local pages, run
# each command over IPC ("sleep N" waits), then save the window as a PNG (Hyprland:
# focus it, read its geometry with hyprctl, grab it with grim).
# Usage: tests/shot.sh <binary> <out.png> [command ...]
#   env: SHOT_TABS (default 4), SHOT_WAIT (seconds before the shot, default 1),
#        SHOT_THEME (e.g. light), SHOT_PAGE (URL of the first tab),
#        SHOT_STDERR (copy the browser's stderr there)
#   commands: "sleep N", "type TEXT" (wtype), "burst N" (N frames right away)
set -u
bin=$(realpath "$1")
out=$(realpath -m "$2")
shift 2
tmp=$(mktemp -d /tmp/nova-shot.XXXXXX)
export XDG_CONFIG_HOME=$tmp/config XDG_DATA_HOME=$tmp/data XDG_STATE_HOME=$tmp/state XDG_CACHE_HOME=$tmp/cache
mkdir -p $XDG_DATA_HOME/novabrowser && touch $XDG_DATA_HOME/novabrowser/welcomed
q() { "$bin" --query state 2>/dev/null; }
wait_for() { for _ in $(seq $(( ${2:-10} * 5 ))); do q | grep -q -- "$1" && return 0; sleep 0.2; done; return 1; }
page() {
    echo "data:text/html,<title>$1</title><body style='margin:0;font:18px sans-serif;background:linear-gradient(135deg,%23ff9a8b,%23a18cd1 50%,%2384fab0)'><div style='padding:60px'><h1 style='font-size:64px;margin:0 0 20px'>$1</h1><p style='max-width:700px;line-height:1.6'>Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris.</p><div style='display:flex;gap:20px;margin-top:30px'><div style='width:220px;height:160px;border-radius:16px;background:%23264653'></div><div style='width:220px;height:160px;border-radius:16px;background:%23e9c46a'></div><div style='width:220px;height:160px;border-radius:16px;background:%23e76f51'></div></div></div>"
}
"$bin" "${SHOT_PAGE:-$(page one)}" >/dev/null 2>"$tmp/stderr" &
pid=$!
trap 'kill $pid 2>/dev/null; rm -rf "$tmp"' EXIT
wait_for '"tabs":\[{' 30 || { echo "did not start"; cat "$tmp/stderr"; exit 1; }
# the newest Hyprland instance (an old signature can linger in the environment);
# focused, so a scrolling layout brings it on screen
export HYPRLAND_INSTANCE_SIGNATURE=$(ls -t /run/user/$(id -u)/hypr | head -1)
hyprctl dispatch focuswindow pid:$pid >/dev/null
[ -n "${SHOT_THEME:-}" ] && "$bin" --cmd "set theme $SHOT_THEME" >/dev/null
for t in $(seq 2 "${SHOT_TABS:-4}"); do "$bin" --cmd "open -t $(page "tab$t")" >/dev/null; done
"$bin" --cmd "tab-focus 1" >/dev/null
sleep 1.5
geo=$(hyprctl -j clients | jq -r --argjson p $pid '.[] | select(.pid == $p) | "\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"' | head -1)
for c in "$@"; do
    case "$c" in
        sleep\ *) sleep "${c#sleep }" ;;
        type\ *) wtype "${c#type }" ;;
        # burst N: N back-to-back frames as <out>-1.png ... (catches flashes)
        burst\ *) for i in $(seq "${c#burst }"); do grim -g "$geo" "${out%.png}-$i.png"; done ;;
        *) "$bin" --cmd "$c" >/dev/null ;;
    esac
done
sleep "${SHOT_WAIT:-1}"
grim -g "$geo" "$out"
[ -n "${SHOT_STDERR:-}" ] && cp "$tmp/stderr" "$SHOT_STDERR"
"$bin" --cmd quit >/dev/null
sleep 1
echo "$out"
