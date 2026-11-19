#!/bin/bash
# S10 bench: for each MODE, run the spike in a nested niri (WAYLAND_DISPLAY=$NESTED, start
# it with `niri -c tests/niri.kdl`) and measure
#   idle CPU, scrolling (page frame rate, frames over 25 ms, OSR paints/s, GTK frames/s, CPU)
#   and 1080p60 VP9 video (frames shown/dropped, CPU).
# CPU is % of one core, browser + all its subprocesses.
# Usage: NESTED=wayland-2 spikes/s10-osr-gtk/bench.sh [modes...]   (default: windowed gpu cpu)
# LIB=<dir with libcef.so> swaps the CEF build (same version) at run time.
set -u
here=$(cd "$(dirname "$0")" && pwd)
bin=${BIN:-$here/../../target/release/s10-osr-gtk}
www=/tmp/s10-www
mkdir -p "$www"
cp "$here"/www/*.html "$www/"
[ -f "$www/video.webm" ] || ffmpeg -v error -f lavfi -i testsrc2=size=1920x1080:rate=60 -t 12 \
    -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -b:v 8M -pix_fmt yuv420p "$www/video.webm"
python3 -u -m http.server 8931 --bind 127.0.0.1 --directory "$www" >/tmp/s10-http.log 2>&1 &
http=$!
trap 'kill $http 2>/dev/null' EXIT
export WAYLAND_DISPLAY=${NESTED:?NESTED=wayland-N} CDP_PORT=9310
unset DISPLAY
[ -n "${LIB:-}" ] && export LD_LIBRARY_PATH=$LIB
cdp() { T=${T:-} timeout 30 node "$here/../cdp.mjs" "$1"; }
paints() { # mean paints/frames per second over log lines $1.. (skips the first)
    tail -n +"$1" /tmp/s10.log | grep '^s10 paints' | tail -n +2 |
        awk -F'[= ]' '{p+=$3; f+=$5; n++} END {if (n) printf "paints/s=%.0f gtk-frames/s=%.0f", p/n, f/n}'
}
for mode in "${@:-windowed gpu cpu}"; do
  for m in $mode; do
    rm -rf /tmp/novabrowser-s10
    MODE=$m CARD=${CARD:-} URL=http://127.0.0.1:8931/scroll.html "$bin" >/tmp/s10.log 2>&1 &
    pid=$!
    for _ in $(seq 60); do curl -s localhost:$CDP_PORT/json/list | grep -q scroll && break; sleep 0.5; done
    sleep 4
    idle=$(python3 "$here/cpu.py" $pid 5)
    n=$(wc -l </tmp/s10.log)
    python3 "$here/cpu.py" $pid 7 >/tmp/s10-cpu &
    cpu_pid=$!
    scroll=$(T=scroll cdp 'bench(8000)' | tr -d ' \n')
    wait $cpu_pid
    echo "$m idle-cpu=$idle%"
    echo "$m scroll $scroll cpu=$(cat /tmp/s10-cpu)% $(paints $n)"
    T=scroll cdp 'location.href = "/video.html"; 1' >/dev/null
    sleep 3
    n=$(wc -l </tmp/s10.log)
    python3 "$here/cpu.py" $pid 9 >/tmp/s10-cpu &
    cpu_pid=$!
    video=$(T=video cdp 'bench(10000)' | tr -d ' \n')
    wait $cpu_pid
    echo "$m video $video cpu=$(cat /tmp/s10-cpu)% $(paints $n)"
    kill $pid; wait $pid 2>/dev/null
    grep -E "^s10 (first|dmabuf|no )" /tmp/s10.log | head -3
  done
done
