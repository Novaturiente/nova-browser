#!/bin/bash
# S8 check: start the spike (bounded by timeout), drive focus + nova.send via DevTools, print the s8 log.
set -u
cd /home/nova/personal/novabrowser/novabrowser
export CEF_PATH=$HOME/.local/share/cef LD_LIBRARY_PATH=$HOME/.local/share/cef CDP_PORT=9308
LOG=/tmp/s8.log
rm -rf /tmp/novabrowser-s8
timeout 120 spikes/s8-renderer/target/release/s8-renderer --use-alloy-style --ozone-platform=wayland \
  --remote-debugging-port=9308 >"$LOG" 2>&1 &
PID=$!
for _ in $(seq 40); do curl -s localhost:9308/json/list >/dev/null && break; sleep 0.5; done
sleep 2
cdp() { echo ">>> ${T:-} $1"; timeout 8 node spikes/cdp.mjs "$1"; sleep 0.7; }
cdp 'document.querySelector("#i").focus()'
cdp 'document.querySelector("#p").focus()'
cdp 'document.querySelector("#t").focus()'
cdp 'document.querySelector("#ce").focus()'
cdp 'document.querySelector("#b").focus()'
cdp 'document.activeElement.blur()'
cdp 'window.nova.send("from-devtools")'
cdp 'JSON.stringify(window.novaReplies)'
cdp 'try { nova.send(42) } catch (e) { "threw: " + e.message }'
cdp 'document.querySelector("#f").contentDocument.querySelector("#fi").focus()'
cdp 'document.querySelector("#i").focus()'
node spikes/cdp.mjs @screenshot /tmp/s8-page.png
echo "=== navigation"
cdp 'location.href = "https://example.com/"; 1'
sleep 3
export T=example
cdp 'typeof window.nova'
cdp 'window.nova.send("after-nav")'
cdp 'document.querySelector("a").focus(); document.activeElement.tagName'
cdp 'document.body.insertAdjacentHTML("beforeend", "<input id=n>"); document.querySelector("#n").focus()'
echo "=== cross-origin iframe"
cdp 'new Promise(r => { const f = document.createElement("iframe"); f.src = "https://example.org/"; f.onload = () => r("loaded"); document.body.append(f); })'
sleep 2
curl -s localhost:9308/json/list | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).map(t=>t.type+" "+t.url)))'
timeout 8 node spikes/s8-renderer/frame.mjs example.org 'typeof window.nova + " " + window.nova.send("from-oopif")'; sleep 0.7
timeout 8 node spikes/s8-renderer/frame.mjs example.org 'document.body.insertAdjacentHTML("beforeend","<textarea id=ot></textarea>"); document.querySelector("#ot").focus(); document.activeElement.tagName'; sleep 0.7
T=example node spikes/cdp.mjs @screenshot /tmp/s8-nav.png
kill $PID; wait $PID 2>/dev/null
echo "=== LOG"; grep -E "^s8\[" "$LOG" | cut -c1-200
