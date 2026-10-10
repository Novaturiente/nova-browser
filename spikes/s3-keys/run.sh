#!/usr/bin/env bash
# S3 driver: start s3-keys (bounded by timeout), send real Wayland keys via wtype,
# read page-side results over CDP, restore focus, stop browser.
set -u
cd "$(dirname "$0")"
export CEF_PATH=$HOME/.local/share/cef LD_LIBRARY_PATH=$HOME/.local/share/cef
export CDP_PORT=9303
CDP="node ../cdp.mjs"
LOG=/tmp/novabrowser-s3.log; : >"$LOG"
PREV=$(hyprctl activewindow -j | node -e 'process.stdin.on("data",d=>console.log(JSON.parse(d).address))')
timeout 90 target/release/s3-keys --use-alloy-style --ozone-platform=wayland \
  --remote-debugging-port=$CDP_PORT >>"$LOG" 2>&1 &
BPID=$!
for _ in $(seq 50); do curl -s localhost:$CDP_PORT/json/list | grep -q '"page"' && break; sleep 0.2; done
sleep 1
ADDR=$(hyprctl clients -j | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const c=JSON.parse(s).find(c=>c.pid=='"$BPID"'||c.title.includes("novabrowser-s3"));console.log(c?c.address:"");})')
echo "== browser pid $BPID window $ADDR (prev $PREV)"
hyprctl clients -j | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>JSON.parse(s).filter(c=>/s3/.test(c.title+c.class)).forEach(c=>console.log("client",c.address,c.class,c.title,c.pid)))'
active() { echo "   active: $(hyprctl activewindow -j | node -e "process.stdin.on(\"data\",d=>{const w=JSON.parse(d);console.log(w.address,w.pid,w.title)})")"; }
focus() { hyprctl dispatch focuswindow address:$ADDR >/dev/null; sleep 0.4; active; }
mark() { echo "-- $1" >>"$LOG"; echo "== $1"; }

mark "A: input focused: type a j k b Esc x q"
focus
$CDP 'document.activeElement.id'
wtype ajkb; wtype -k Escape; wtype xq; active; sleep 0.5
$CDP '({value: i.value, active: document.activeElement.tagName, L})'

mark "B: body focused: type a j k b Esc"
$CDP 'i.blur(); L.length=0; document.activeElement.tagName'
focus
wtype ajkb; wtype -k Escape; active; sleep 0.5
$CDP '({value: i.value, active: document.activeElement.tagName, L})'

mark "C: hold j 1.2s in input (repeat), then hold a 1.2s"
$CDP 'i.focus(); i.value=""; L.length=0; document.activeElement.id'
focus
wtype -P j -s 1200 -p j; wtype -P a -s 1200 -p a; active; sleep 0.5
$CDP '({value: i.value, n: L.length, last: L.slice(-4)})'

mark "D: wtype J (keysym), real shift+j, ctrl+j"
$CDP 'i.value=""; L.length=0; 1'
focus
wtype J; wtype -M shift j -m shift; wtype -M ctrl j -m ctrl; active; sleep 0.5
$CDP '({value: i.value, L})'

mark "E: separate wtype per key: -k j, -k k, hold Escape 0.8s"
$CDP 'i.value=""; L.length=0; 1'
focus
wtype -k j; wtype -k k; wtype -P Escape -s 800 -p Escape; active; sleep 0.5
$CDP '({value: i.value, L})'

mark "done"
[ -n "$PREV" ] && hyprctl dispatch focuswindow address:$PREV >/dev/null
kill $BPID 2>/dev/null; wait $BPID 2>/dev/null
echo "== rust log (s3: lines + markers):"
grep -aE '^(s3:|--)' "$LOG"
