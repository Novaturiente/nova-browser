// S6: log in via Input.dispatchMouseEvent + Input.insertText only (no DOM value setting).
// usage: CDP_PORT=9305 node s6-input.mjs [outdir]
import { connect, attach, pages, sleep } from "./cdp-lib.mjs";

const OUT = process.argv[2] ?? "/tmp/s5s6";
const LOGIN = "https://the-internet.herokuapp.com/login";
const c = await connect();
const s = await attach(c, (await pages(c))[0].targetId);
await s.send("Page.enable");
await s.send("Runtime.enable");

// Event recorder survives the POST/redirect via sessionStorage (same origin).
await s.send("Page.addScriptToEvaluateOnNewDocument", { source: `
  for (const t of ["mousedown","mouseup","click","focus","input","change","submit"])
    addEventListener(t, e => {
      const log = JSON.parse(sessionStorage.s6log || "[]");
      const el = e.target;
      log.push({ type: t, trusted: e.isTrusted, target: el.id || el.name || el.tagName,
                 inputType: e.inputType, data: e.data, x: e.clientX, y: e.clientY });
      sessionStorage.s6log = JSON.stringify(log);
    }, true);` });
await s.navigate(LOGIN, "Page.domContentEventFired", 60000);
await s.eval("sessionStorage.s6log = '[]'");
await s.screenshot(`${OUT}/s6-1-login.png`);

const center = async (sel) => s.eval(`(() => { const r = document.querySelector(${JSON.stringify(sel)}).getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
async function click(sel) {
  const { x, y } = await center(sel);
  await s.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await s.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
  await s.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
  const active = await s.eval("document.activeElement.id || document.activeElement.tagName");
  console.log(`click ${sel} at (${x.toFixed(0)},${y.toFixed(0)}) -> activeElement=${active}`);
}

await click("#username");
await s.send("Input.insertText", { text: "tomsmith" });
await click("#password");
await s.send("Input.insertText", { text: "SuperSecretPassword!" });
console.log("field values:", await s.eval("[username.value, password.value.length]"));
await s.screenshot(`${OUT}/s6-2-filled.png`);

await click("button[type=submit]");
await s.waitFor("location.pathname === '/secure' && !!document.querySelector('#flash')", 60000);
await sleep(500);
await s.screenshot(`${OUT}/s6-3-after-submit.png`);
console.log("url:", await s.eval("location.href"));
console.log("flash:", await s.eval("document.querySelector('#flash')?.innerText.trim()"));
const log = JSON.parse(await s.eval("sessionStorage.s6log"));
const summary = {};
for (const e of log) {
  const k = `${e.type}@${e.target}`;
  summary[k] ??= { n: 0, trusted: new Set() };
  summary[k].n++; summary[k].trusted.add(e.trusted);
}
for (const [k, v] of Object.entries(summary)) console.log(`  ${k}: n=${v.n} isTrusted=${[...v.trusted]}`);
console.log("sample input events:", JSON.stringify(log.filter((e) => e.type === "input").slice(0, 2)));
c.close();
