#!/usr/bin/env bash
# Overlay flash + focus test. Starts a nested niri (tests/niri.kdl: its own output, so
# nothing of the live desktop is recorded and no key reaches it), runs the browser in it
# on a throwaway profile, records at 60 fps while overlays open and close, and prints
# the mean brightness (luma 0-255) of each frame per step. A step FLASHes when a frame
# leaves the range of its first and last frame by more than 12: a plain fade never does.
# Then checks the URL box takes typed keys and closes on Escape.
# Needs niri, wf-recorder, wtype, ffmpeg, python3. Exit status = number of failures.
# Usage: tests/flash.sh [binary] [out-dir]   (default target/debug/novabrowser)
set -u
cd "$(dirname "$0")/.."
bin=$(realpath "${1:-target/debug/novabrowser}")
out=$(realpath -m "${2:-/tmp/nova-flash/$(date +%H%M%S)}")
mkdir -p "$out"
tmp=$(mktemp -d /tmp/nova-flash.XXXXXX)
fails=0
ok()   { echo "ok    $1"; }
fail() { echo "FAIL  $1"; fails=$((fails + 1)); }

# nested compositor: its socket is the wayland-* that appears after it starts
before=$(ls "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' | sort)
niri -c tests/niri.kdl >"$tmp/niri.log" 2>&1 &
niri_pid=$!
nested=
for _ in $(seq 50); do
    nested=$(comm -13 <(echo "$before") <(ls "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' | sort) | head -1)
    [ -n "$nested" ] && break
    sleep 0.2
done
[ -n "$nested" ] || { echo "nested niri did not start"; cat "$tmp/niri.log"; exit 1; }

# local http pages (dark-toggle needs a host; data: pages never go dark)
mkdir -p "$tmp/www"
echo "<title>light</title><body style='background:#fff;margin:0'><h1 style='font:40px sans-serif;padding:40px'>Light page</h1>" >"$tmp/www/light.html"
echo "<title>ask</title><body style='background:#fff'><script>setTimeout(()=>confirm('Proceed?'),1000)</script>" >"$tmp/www/ask.html"
python3 -u -m http.server 0 --bind 127.0.0.1 --directory "$tmp/www" >"$tmp/http.log" 2>&1 &
http_pid=$!
for _ in $(seq 50); do port=$(grep -oE 'port [0-9]+' "$tmp/http.log" | grep -oE '[0-9]+'); [ -n "$port" ] && break; sleep 0.1; done
www="http://127.0.0.1:$port"

export WAYLAND_DISPLAY=$nested
unset DISPLAY
export XDG_CONFIG_HOME=$tmp/config XDG_DATA_HOME=$tmp/data XDG_STATE_HOME=$tmp/state XDG_CACHE_HOME=$tmp/cache
mkdir -p "$XDG_DATA_HOME/novabrowser" && touch "$XDG_DATA_HOME/novabrowser/welcomed"
q() { "$bin" --query state 2>/dev/null; }
wait_for() { for _ in $(seq $(( ${2:-10} * 5 ))); do q | grep -q -- "$1" && return 0; sleep 0.2; done; return 1; }
"$bin" "$www/light.html" >/dev/null 2>"$out/stderr" &
pid=$!
trap 'kill $rec $pid $http_pid $niri_pid 2>/dev/null; rm -rf "$tmp"' EXIT
rec=
wait_for '"title":"light"' 30 || { echo "browser did not start"; cat "$out/stderr"; exit 1; }
sleep 2.5

# focus first, on a fresh page: keys typed right after opening the URL box land in it;
# Escape closes it
"$bin" --cmd "cmd-set-text :open " >/dev/null; sleep 0.6
# leading space: the IPC trims the trailing one of ":open "
wtype " $www/light.html#typed"; sleep 0.2; grim "$out/typed.png"; wtype -k Return
wait_for 'light.html#typed' 5 && ok "URL box takes typed keys" || fail "URL box takes typed keys: $(q)"
sleep 1
"$bin" --cmd "cmd-set-text :" >/dev/null; sleep 0.6
wtype -k Escape; sleep 0.5
q | grep -q '"mode":"normal"' && ok "Escape closes the URL box" || fail "Escape closes the URL box: $(q)"
# reopened while the last close still fades out
"$bin" --cmd "cmd-set-text :open " >/dev/null; sleep 0.4
"$bin" --cmd "mode-leave" >/dev/null; sleep 0.05
"$bin" --cmd "cmd-set-text :" >/dev/null; sleep 0.6
wtype -k Escape; sleep 0.5
q | grep -q '"mode":"normal"' && ok "Escape closes a quickly reopened URL box" || fail "Escape closes a quickly reopened URL box: $(q)"
"$bin" --cmd "mode-leave" >/dev/null
sleep 1

dark="data:text/html,<title>dark</title><body style='background:%23123;margin:0'><h1 style='color:%23fff;font:40px sans-serif;padding:40px'>Dark page</h1>"
wf-recorder -y -r 60 -c libx264 -p crf=12 -f "$out/rec.mkv" >"$out/rec.log" 2>&1 &
rec=$!
rec_t=$(date +%s.%N)
echo "$rec_t" >"$out/rec_t"
sleep 1.5
: >"$out/marks"
mark() { echo "$(date +%s.%N) $1" >>"$out/marks"; }
# sync: the recording's clock starts at its first frame, not at launch; this light to
# black switch is found in the frames and lines the marks up
black="data:text/html,<title>black</title><body style='background:%23000'>"
"$bin" --cmd "open $black" >/dev/null; echo "$(date +%s.%N)" >"$out/sync"; sleep 1.2
"$bin" --cmd "open $www/light.html" >/dev/null; sleep 1.5
step() { mark "$1"; "$bin" --cmd "$2" >/dev/null; sleep "${3:-1.2}"; }
key() { mark "$1"; wtype "${@:2}"; sleep 1.2; }
# "goto" steps load a page (a brightness jump of their own): shown, not checked
step "urlbox-open-light" "cmd-set-text :open "
step "urlbox-close" "mode-leave" 0.8
step "search-open-light" "cmd-set-text /"
step "search-close-light" "mode-leave" 0.8
step "toast-light" "message-info Hello from the test" 1.5
key "keyhint-g-light" g
key "keyhint-esc-light" -k Escape
step "dark-toggle-on" "dark-toggle" 1.8
step "dark-toggle-off" "dark-toggle" 1.8
step "goto-dark" "open $dark" 1.5
step "urlbox-open-dark" "cmd-set-text :open "
step "urlbox-close-dark" "mode-leave" 0.8
step "search-open" "cmd-set-text /"
step "search-close" "mode-leave" 0.8
step "toast-dark" "message-error Something failed" 1.5
key "keyhint-g" g
key "keyhint-esc" -k Escape
step "goto-ask" "open $www/ask.html" 0.9
mark "prompt-show"; sleep 1.2
key "prompt-answer" n
# wf-recorder stops at its next frame: give it one; mkv stays readable even if killed
kill -INT $rec; "$bin" --cmd "message-info end" >/dev/null
for _ in $(seq 25); do kill -0 $rec 2>/dev/null || break; sleep 0.2; done
kill -KILL $rec 2>/dev/null; wait $rec 2>/dev/null; rec=

# brightness per frame of the whole window and of two regions small popups use: the
# top centre (toast, search pill, prompt) and the bottom right (which-key)
for r in "full:null" "top:crop=iw/2:ih/6:iw/4:0" "corner:crop=iw*0.35:ih*0.45:iw*0.65:ih*0.55"; do
    ffmpeg -v error -i "$out/rec.mkv" -vf "${r#*:},signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=$out/luma-${r%%:*}.txt" -f null -
done
python3 - "$out" <<'E' || fails=$((fails + $?))
import re, sys
out = sys.argv[1]
def load(region):
    ts, ys = [], []
    for l in open(f"{out}/luma-{region}.txt"):
        m = re.search(r"pts_time:([\d.]+)", l)
        if m: ts.append(float(m.group(1)))
        m = re.search(r"YAVG=([\d.]+)", l)
        if m: ys.append(float(m.group(1)))
    return ts, ys
regions = {r: load(r) for r in ("full", "top", "corner")}
ts, ys = regions["full"]
marks = [l.split(None, 1) for l in open(f"{out}/marks")]
# first frame after the sync command that is dark: the black page
sync = float(open(f"{out}/sync").read())
first_dark = next((tt for tt, y in zip(ts, ys) if y < 40), None)
if first_dark is None:
    print("no sync frame: marks use the recorder launch time (less exact)")
    t0 = float(open(f"{out}/rec_t").read())
else:
    t0 = sync - first_dark + 0.05  # the page shows ~50 ms after the command
def runs(seq):
    out, prev = [], None
    for v in map(round, seq):
        if prev is not None and abs(v - prev[0]) <= 1: prev[1] += 1
        else:
            prev = [v, 1]; out.append(prev)
    return " ".join(f"{v}" + (f"x{n}" if n > 1 else "") for v, n in out)
bad = 0
ends = [float(t) - t0 for t, _ in marks[1:]] + [1e9]
for (t, name), end in zip(marks, ends):
    t = float(t) - t0
    shown, flashed = None, []
    for region, (rts, rys) in regions.items():
        # up to the next step, which may show its own card
        seq = [y for tt, y in zip(rts, rys) if t - 0.1 <= tt <= min(t + 0.9, end - 0.02)]
        if not seq: continue
        lo, hi = min(seq[0], seq[-1]) - 12, max(seq[0], seq[-1]) + 12
        if any(y < lo or y > hi for y in seq): flashed.append((region, seq))
        if region == "full": shown = seq
    if name.strip().startswith("goto"): flashed = []
    bad += bool(flashed)
    region, seq = flashed[0] if flashed else ("full", shown or [])
    print(f"{'FLASH' if flashed else 'ok   '} {name.strip():20} {region:6} {runs(seq)}")
sys.exit(min(bad, 100))
E

[ "${KEEP:-0}" = 1 ] || rm -f "$out/rec.mkv"
"$bin" --cmd quit >/dev/null
echo "$fails failure(s); frames: $out/luma-*.txt"
exit $fails
