#!/bin/bash
# S7 check: drive.sh perms <S7_ALLOW value> | drive.sh hold | drive.sh cert <S7_CERT value> | drive.sh extra
# Starts the spike (bounded by timeout), drives it over DevTools, screenshots the window with grim, prints the s7 log.
set -u
cd /home/nova/personal/novabrowser/novabrowser
export CEF_PATH=$HOME/.local/share/cef LD_LIBRARY_PATH=$HOME/.local/share/cef CDP_PORT=9307
MODE=$1 VAL=${2:-}
TAG=$MODE${VAL:+-$VAL}
LOG=/tmp/s7-$TAG.log
SHOT=/tmp/s7-shots; mkdir -p $SHOT
rm -rf /tmp/novabrowser-s7
case $MODE in
  perms) export S7_ALLOW=$VAL ;;
  hold) export S7_ALLOW=hold ;;
  cert|proceed) export S7_CERT=$VAL S7_URL=https://expired.badssl.com/ ;;
  extra) export S7_ALLOW=0 S7_JS=${VAL:-0} ;;
esac
timeout 150 spikes/s7-handlers/target/release/s7-handlers --use-alloy-style --ozone-platform=wayland \
  --remote-debugging-port=9307 --use-fake-device-for-media-stream >"$LOG" 2>&1 &
PID=$!
for _ in $(seq 40); do curl -s localhost:9307/json/list >/dev/null && break; sleep 0.5; done
sleep 3
BPID=$(pgrep -P $PID -f s7-handlers | head -1)
p() { echo ">>> $1"; timeout ${2:-15} node spikes/s7-handlers/probe.mjs "$1"; sleep 0.5; }
shot() { # grim the window of the browser process
  local g; g=$(hyprctl clients -j | node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{const c=JSON.parse(s).find(c=>c.pid==$BPID);if(c)console.log(c.at[0]+','+c.at[1]+' '+c.size[0]+'x'+c.size[1])})")
  if [ -n "$g" ]; then grim -g "$g" "$SHOT/$1.png" && echo "shot $SHOT/$1.png ($g)"; else echo "no window for pid $BPID"; fi
}
QUERY='Promise.all(["notifications","geolocation","microphone","camera","clipboard-read"].map(n=>navigator.permissions.query({name:n}).then(s=>n+"="+s.state,e=>n+"=err"))).then(a=>a.join(" "))'
NOTIF='Notification.requestPermission()'
GEO='new Promise(r=>navigator.geolocation.getCurrentPosition(p=>r("ok "+p.coords.latitude),e=>r("err code="+e.code+" "+e.message),{timeout:8000}))'
MIC='navigator.mediaDevices.getUserMedia({audio:true}).then(s=>"ok "+s.getTracks().map(t=>t.kind+":"+t.label).join(),e=>"err "+e.name+": "+e.message)'
CAM='navigator.mediaDevices.getUserMedia({video:true}).then(s=>"ok "+s.getTracks().map(t=>t.kind+":"+t.label).join(),e=>"err "+e.name+": "+e.message)'
CLIP='navigator.clipboard.readText().then(t=>"ok len="+t.length,e=>"err "+e.name+": "+e.message)'
case $MODE in
  perms)
    p 'location.href + " secure=" + isSecureContext'
    p "$QUERY"
    p "$NOTIF"; p "$GEO"; p "$MIC"; p "$CAM"; p "$CLIP"
    p "$QUERY"
    shot perms-$VAL ;;
  hold)
    p 'window.r = {}; Notification.requestPermission().then(v => r.notif = v); navigator.geolocation.getCurrentPosition(p => r.geo = "ok", e => r.geo = "err code=" + e.code, {timeout: 20000}); navigator.mediaDevices.getUserMedia({audio:true}).then(s => r.mic = "ok", e => r.mic = "err " + e.name); "started"'
    sleep 2
    p 'JSON.stringify(r)'
    shot hold-pending
    p 'document.title = "s7:release:1"; "released"'
    sleep 2
    p 'JSON.stringify(r)'
    p "$QUERY"
    shot hold-released ;;
  cert)
    sleep 2
    p 'location.href + " | " + document.title + " | " + document.body.innerText.slice(0, 120)'
    curl -s localhost:9307/json/list | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log("targets:",JSON.parse(s).map(t=>t.type+" "+t.url)))'
    shot cert-$VAL-expired
    p 'location.href = "https://self-signed.badssl.com/"; 1'
    sleep 4
    p 'location.href + " | " + document.title + " | " + document.body.innerText.slice(0, 120)'
    curl -s localhost:9307/json/list | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log("targets:",JSON.parse(s).map(t=>t.type+" "+t.url)))'
    shot cert-$VAL-selfsigned ;;
  proceed) # deny, then click the interstitial's own Advanced -> Proceed link
    sleep 2
    p 'document.querySelector("#details-button").click(); const l = document.querySelector("#proceed-link"); l ? l.textContent : "no proceed link"'
    shot proceed-advanced
    p 'document.querySelector("#proceed-link").click(); "clicked"'
    sleep 4
    p 'location.href + " | " + document.title'
    shot proceed-after ;;
  extra)
    p 'setTimeout(() => window.alertResult = alert("s7 alert"), 0); "alert queued"'
    sleep 2
    shot extra-alert
    curl -s localhost:9307/json/list | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log("targets:",JSON.parse(s).map(t=>t.type+" "+t.url)))'
    p '"page responsive"' 5
    p 'window.open("https://example.org/") === null ? "open returned null" : "open returned window"'
    p 'const a = document.createElement("a"); a.href = URL.createObjectURL(new Blob(["s7"])); a.download = "s7.txt"; a.click(); "download clicked"'
    sleep 2
    shot extra-after ;;
esac
kill $PID; wait $PID 2>/dev/null
pkill -f /tmp/novabrowser-s7 2>/dev/null
echo "=== LOG $LOG"; grep -E "^s7:" "$LOG" | cut -c1-260
