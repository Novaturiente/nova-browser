// S1 leftovers: <select> popup + Wayland clipboard (both directions) over CDP.
// usage: CDP_PORT=9301 CEF_PID=<pid> node s1-test.mjs [outdir]
// Needs a running cefsimple with --remote-debugging-port=$CDP_PORT.
import { execFileSync } from "node:child_process";
import { connect, attach, pages, sleep } from "../s5-s6/cdp-lib.mjs";

const OUT = process.argv[2] ?? "/tmp/s1";
const PID = Number(process.env.CEF_PID);
const sh = (cmd, args, opts = {}) => {
  try { return execFileSync(cmd, args, { encoding: "utf8", timeout: 5000, ...opts }); }
  catch (e) { return `ERR(${e.status}): ${e.stderr ?? e.message}`; }
};
const log = (...a) => console.log(...a);

const opts = Array.from({ length: 20 }, (_, i) => `<option value="o${i}">Option ${i}</option>`).join("");
const html = `<!doctype html><title>s1</title><body style="font:16px sans-serif;margin:40px">
<p id="para">Copy source paragraph nova-primary-check</p>
<select id="sel">${opts}</select>
<p><textarea id="ta" rows="4" cols="40"></textarea></p>
<p><input id="inp" value="nova-copy-out-test" size="30"></p>
<script>window.evts=[];sel.addEventListener('change',e=>evts.push('change:'+sel.value));
sel.addEventListener('input',e=>evts.push('input:'+sel.value));</script></body>`;
const URL_ = "data:text/html;base64," + Buffer.from(html).toString("base64");

const c = await connect();
const s = await attach(c, (await pages(c))[0].targetId);
await s.send("Runtime.enable");
await s.navigate(URL_);
await sleep(500);

// --- window geometry / DPR ---
const clients = JSON.parse(sh("hyprctl", ["clients", "-j"]));
const win = clients.find((w) => w.pid === PID);
const mons = JSON.parse(sh("hyprctl", ["monitors", "-j"]));
log("window:", win && { at: win.at, size: win.size, class: win.class, xwayland: win.xwayland, monitor: win.monitor });
log("monitors:", mons.map((m) => ({ name: m.name, w: m.width, h: m.height, scale: m.scale })));
log("devicePixelRatio:", await s.eval("devicePixelRatio"), "inner:", await s.eval("[innerWidth, innerHeight]"));
const active = JSON.parse(sh("hyprctl", ["activewindow", "-j"]));
log("active window pid:", active.pid, "is cef:", active.pid === PID);
const geom = win ? `${win.at[0]},${win.at[1]} ${win.size[0]}x${win.size[1]}` : null;
const grab = (name) => {
  const p = `${OUT}/${name}.png`;
  // whole screen: popup may extend past the window rect
  log("grim", name, sh("grim", [p]) || "ok", p);
  if (geom) sh("grim", ["-g", geom, `${OUT}/${name}-win.png`]);
};

const center = (sel) => s.eval(`(() => { const r = document.querySelector(${JSON.stringify(sel)}).getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2, bottom: r.bottom, left: r.left, w: r.width, h: r.height }; })()`);
async function click(sel) {
  const p = await center(sel);
  await s.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: p.x, y: p.y });
  await s.send("Input.dispatchMouseEvent", { type: "mousePressed", x: p.x, y: p.y, button: "left", clickCount: 1 });
  await s.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: p.x, y: p.y, button: "left", clickCount: 1 });
  return p;
}
const key = async (k, code, vk, extra = {}) => {
  await s.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: k, code, windowsVirtualKeyCode: vk, ...extra });
  await s.send("Input.dispatchKeyEvent", { type: "keyUp", key: k, code, windowsVirtualKeyCode: vk, modifiers: extra.modifiers ?? 0 });
};

// --- <select> ---
log("\n== select ==");
grab("select-0-before");
const sp = await click("#sel");
log("select rect (css px):", sp);
await sleep(800);
grab("select-1-open");
for (let i = 0; i < 3; i++) { await key("ArrowDown", "ArrowDown", 40); await sleep(150); }
grab("select-2-after-arrows");
await key("Enter", "Enter", 13);
await sleep(400);
log("after keys: value=", await s.eval("sel.value"), "evts=", await s.eval("evts"));
grab("select-3-closed");

// mouse pick inside the popup: reopen, click on the 6th row if popup is drawn below the select
await click("#sel");
await sleep(800);
const rowH = sp.h; // approx; Chromium popup rows ~ select height
const target = { x: sp.left + 20, y: sp.bottom + rowH * 5.5 };
await s.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...target });
await s.send("Input.dispatchMouseEvent", { type: "mousePressed", ...target, button: "left", clickCount: 1 });
await s.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...target, button: "left", clickCount: 1 });
await sleep(400);
log("after popup mouse click at", target, ": value=", await s.eval("sel.value"), "evts=", await s.eval("evts"));
grab("select-4-after-mouse");
await key("Escape", "Escape", 27);
await sleep(300);

// --- clipboard: paste in ---
log("\n== clipboard paste-in ==");
sh("wl-copy", ["nova-paste-test"], { stdio: "ignore" }); // wl-copy forks a server; keep it off our pipes
await sleep(300);
log("wl-paste -n now:", JSON.stringify(sh("wl-paste", ["-n"])));
await click("#ta");
await key("v", "KeyV", 86, { modifiers: 2 });
await sleep(400);
log("textarea after Ctrl+V (no commands):", JSON.stringify(await s.eval("ta.value")));
await s.eval("ta.value=''");
await click("#ta");
await key("v", "KeyV", 86, { modifiers: 2, commands: ["paste"] });
await sleep(400);
log("textarea after Ctrl+V (commands:[paste]):", JSON.stringify(await s.eval("ta.value")));
const rt = await s.send("Runtime.evaluate", {
  expression: "navigator.clipboard.readText().then(t=>'ok:'+t, e=>'err:'+e)",
  awaitPromise: true, returnByValue: true, userGesture: true });
log("navigator.clipboard.readText():", JSON.stringify(rt.result.value ?? rt));

// --- clipboard: copy out ---
log("\n== clipboard copy-out ==");
await click("#inp");
await s.eval("inp.select()");
await key("c", "KeyC", 67, { modifiers: 2 });
await sleep(400);
log("wl-paste -n after Ctrl+C (input selected):", JSON.stringify(sh("wl-paste", ["-n"])));
await s.eval("inp.value='nova-copy-cmd-test'; inp.select()");
await key("c", "KeyC", 67, { modifiers: 2, commands: ["copy"] });
await sleep(400);
log("wl-paste -n after Ctrl+C commands:[copy]:", JSON.stringify(sh("wl-paste", ["-n"])));
const ex = await s.send("Runtime.evaluate", {
  expression: "inp.value='nova-execcommand-test'; inp.select(); document.execCommand('copy')",
  returnByValue: true, userGesture: true });
await sleep(400);
log("execCommand('copy') ->", ex.result.value, "wl-paste -n:", JSON.stringify(sh("wl-paste", ["-n"])));
const wt = await s.send("Runtime.evaluate", {
  expression: "navigator.clipboard.writeText('nova-writetext-test').then(()=>'ok', e=>'err:'+e)",
  awaitPromise: true, returnByValue: true, userGesture: true });
await sleep(400);
log("clipboard.writeText ->", JSON.stringify(wt.result.value ?? wt), "wl-paste -n:", JSON.stringify(sh("wl-paste", ["-n"])));

// what does Chromium's own clipboard hold? paste it back into the textarea
await s.eval("ta.value=''");
await click("#ta");
await key("v", "KeyV", 86, { modifiers: 2 });
await sleep(400);
log("textarea after Ctrl+V (Chromium-side clipboard after copies):", JSON.stringify(await s.eval("ta.value")));
log("wl-paste -l:", JSON.stringify(sh("wl-paste", ["-l"])));

// --- primary selection (report only) ---
log("\n== primary selection ==");
log("wl-paste -p before:", JSON.stringify(sh("wl-paste", ["-p", "-n"])));
// mouse drag-select over the paragraph
const pp = await center("#para");
const x0 = pp.left + 2, x1 = pp.left + pp.w - 2, y = pp.y;
await s.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: x0, y });
await s.send("Input.dispatchMouseEvent", { type: "mousePressed", x: x0, y, button: "left", clickCount: 1 });
for (let i = 1; i <= 10; i++)
  await s.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: x0 + (x1 - x0) * i / 10, y, button: "left", buttons: 1 });
await s.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: x1, y, button: "left", clickCount: 1 });
await sleep(400);
log("page selection:", JSON.stringify(await s.eval("getSelection().toString()")));
log("wl-paste -p after drag-select:", JSON.stringify(sh("wl-paste", ["-p", "-n"])));
grab("primary-after-drag");

// --- copy-out with a real Wayland key serial (opt-in) ---
// Chromium only calls wl_data_device.set_selection with a key/button-press serial; CDP input has none.
// hyprctl send_shortcut (Lua-config Hyprland 0.56 syntax) delivers a compositor key event to our window. Only when it already has focus.
if (process.env.S1_SENDSHORTCUT) {
  log("\n== copy-out via hyprctl sendshortcut ==");
  const act = JSON.parse(sh("hyprctl", ["activewindow", "-j"]));
  if (act.pid !== PID) log("skipped: cef window not focused (active pid", act.pid, ")");
  else {
    await click("#inp");
    await s.eval("inp.value='nova-real-serial-copy'; inp.select()");
    log("sendshortcut:", sh("hyprctl", ["dispatch", `hl.dsp.send_shortcut({ mods = "CTRL", key = "C", window = "pid:${PID}" })`]).trim());
    await sleep(500);
    log("wl-paste -n after compositor Ctrl+C:", JSON.stringify(sh("wl-paste", ["-n"])));
    // a later CDP-only copy now has a press serial to reuse
    await s.eval("inp.value='nova-cdp-after-serial'; inp.select()");
    await key("c", "KeyC", 67, { modifiers: 2 });
    await sleep(400);
    log("wl-paste -n after CDP Ctrl+C (serial now known):", JSON.stringify(sh("wl-paste", ["-n"])));
    await click("#para");
    await s.eval("getSelection().selectAllChildren(para)");
    await sleep(300);
    log("wl-paste -p after JS selection (serial known):", JSON.stringify(sh("wl-paste", ["-p", "-n"])));
  }
}

// --- async clipboard API needs a secure context (data: URL is not) ---
log("\n== navigator.clipboard on https://example.com ==");
await s.navigate("https://example.com", "Page.loadEventFired", 30000).catch((e) => log("nav:", e.message));
const asyncClip = async (label) => {
  // race in-page: an unanswered permission prompt would hang the promise forever
  const ev = (expression) => s.send("Runtime.evaluate", { expression: `Promise.race([${expression},
      new Promise(r => setTimeout(() => r('timeout 3s (permission prompt unanswered?)'), 3000))])`,
    awaitPromise: true, returnByValue: true, userGesture: true })
    .then((r) => r.result.value ?? r.exceptionDetails?.exception?.description);
  log(label, "isSecureContext:", await ev("isSecureContext"));
  log(label, "readText:", JSON.stringify(await ev("navigator.clipboard.readText().then(t=>'ok:'+t, e=>'err:'+e)")));
  log(label, "writeText:", await ev("navigator.clipboard.writeText('nova-writetext-test').then(()=>'ok', e=>'err:'+e)"),
    "wl-paste -n:", JSON.stringify(sh("wl-paste", ["-n"])));
};
await asyncClip("no grant:");
await click("body"); // focus the document (async clipboard requires document focus)
log("document.hasFocus():", await s.eval("document.hasFocus()"));
await asyncClip("after click:");
await s.send("Emulation.setFocusEmulationEnabled", { enabled: true });
await asyncClip("focus emulation:");
await c.send("Browser.grantPermissions", { origin: "https://example.com", permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"] })
  .then(() => asyncClip("after Browser.grantPermissions:"), (e) => log("grantPermissions:", e.message));

c.close();
