// S9 scrolling smoothness: one browser instance per run, SITES loaded one at a time in the single
// foreground page (Page.navigate), then ~5 s of Input.synthesizeScrollGesture (mouse wheel, down/up
// alternating) — Maps: mouse drag-pan instead. rAF intervals collected in page JS.
// usage: node scroll.mjs [runs=2] [kinds=cef,chrome] [outName=scroll]
import { execFileSync } from "node:child_process";
import { launch, connect, waitCdp, hyprPlace, hyprClients, WIN, SITES, sleep, save, median, conditions } from "./lib.mjs";

const runs = +(process.argv[2] ?? 2);
const kinds = (process.argv[3] ?? "cef,chrome").split(",");
const NAME = process.argv[4] ?? "scroll";
const EXTRA = ["--disable-renderer-backgrounding", "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows"];
const DUR_MS = 5000;

const COLLECT = `(() => { window.__fr = []; window.__stop = false; let last = 0;
  const f = (t) => { if (last) __fr.push(t - last); last = t; if (!__stop) requestAnimationFrame(f); };
  requestAnimationFrame(f); window.__y0 = scrollY; return true; })()`;
const STOP = `(() => { window.__stop = true; return { fr: __fr, dy: scrollY - __y0, maxY: document.scrollingElement.scrollHeight - innerHeight, vis: document.visibilityState, focus: document.hasFocus() }; })()`;

function stats(fr, ms) {
  const s = [...fr].sort((a, b) => a - b);
  return {
    frames: fr.length,
    avgFps: +(fr.length / (fr.reduce((a, b) => a + b, 0) / 1000)).toFixed(1),
    pctOver20ms: +((100 * fr.filter((x) => x > 20).length) / fr.length).toFixed(1),
    p95ms: +s[Math.floor(0.95 * (s.length - 1))].toFixed(1),
    maxMs: +s[s.length - 1].toFixed(1),
    wallMs: ms,
  };
}

async function scrollSite(s, site) {
  const t0 = Date.now();
  if (site.name === "maps") {
    // drag-pan: press at centre, move 6 px every ~16 ms, reverse direction every 1 s
    const send = (type, x, y) => s.send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: type === "mouseReleased" ? 0 : 1, clickCount: 1 });
    let x = 700, y = 450, dx = -6;
    await send("mousePressed", x, y);
    for (let i = 0; Date.now() - t0 < DUR_MS; i++) {
      if (i % 60 === 59) dx = -dx;
      x += dx; y += dx / 3;
      await send("mouseMoved", x, y);
      await sleep(16);
    }
    await send("mouseReleased", x, y);
  } else {
    let dir = -1; // negative yDistance = scroll down
    while (Date.now() - t0 < DUR_MS) {
      const left = DUR_MS - (Date.now() - t0);
      const dist = Math.max(300, Math.min(3000, Math.round((left / 1000) * 1500)));
      await s.send("Input.synthesizeScrollGesture", { x: 700, y: 450, yDistance: dir * dist, speed: 1500, gestureSourceType: "mouse", preventFling: true });
      // reached the end of the page → go back the other way
      const at = await s.eval("({y: scrollY, max: document.scrollingElement.scrollHeight - innerHeight})");
      if ((dir < 0 && at.y >= at.max - 5) || (dir > 0 && at.y <= 5)) dir = -dir;
    }
  }
  return Date.now() - t0;
}

async function once(kind) {
  const b = launch(kind, "about:blank", EXTRA);
  const r = { kind, conditions: conditions(), extraFlags: EXTRA, sites: [] };
  try {
    await waitCdp(b.port);
    const c = await connect(b.port);
    let p;
    for (const end = Date.now() + 20000; !p && Date.now() < end; await sleep(100)) p = (await c.pages())[0];
    const s = await c.attach(p.targetId);
    await sleep(1000);
    r.windows = hyprPlace(b.pid, ...WIN[kind]);
    const focus = () => { for (const w of hyprClients(b.pid)) try { execFileSync("hyprctl", ["dispatch", `hl.dsp.focus({ window = "address:${w.address}" })`]); } catch {} };
    for (const site of SITES) {
      const x = { site: site.name, url: site.url };
      try {
        await s.send("Page.navigate", { url: site.url });
        await sleep(1000);
        await s.waitFor("document.readyState === 'complete'", 30000).catch(() => (x.loadTimeout = true));
        await sleep(4000); // settle
        focus();
        x.viewport = await s.eval("({w: innerWidth, h: innerHeight, vis: document.visibilityState, focus: document.hasFocus(), title: document.title.slice(0, 60)})");
        await s.eval(COLLECT);
        await sleep(100);
        const ms = await scrollSite(s, site);
        const res = await s.eval(STOP);
        Object.assign(x, stats(res.fr, ms), { scrolledPx: Math.round(res.dy), maxScrollY: res.maxY, visEnd: res.vis, focusEnd: res.focus });
      } catch (e) {
        x.error = String(e.stack).slice(0, 400);
      }
      console.log(kind, JSON.stringify({ ...x, url: undefined }));
      r.sites.push(x);
    }
    c.close();
  } catch (e) {
    r.error = String(e.stack);
  }
  await b.kill();
  await sleep(3000);
  return r;
}

let refresh = null;
try { refresh = JSON.parse(execFileSync("hyprctl", ["monitors", "-j"], { encoding: "utf8" })).map((m) => ({ name: m.name, mode: `${m.width}x${m.height}@${m.refreshRate}`, scale: m.scale, focused: m.focused })); } catch {}
const out = { conditions: conditions(), monitors: refresh, durationMs: DUR_MS, runs: {}, medians: {} };
for (const k of kinds) out.runs[k] = [];
for (let i = 0; i < runs; i++)
  for (const k of kinds) {
    out.runs[k].push(await once(k));
    save(NAME, out);
  }
for (const k of kinds) {
  out.medians[k] = {};
  for (const site of SITES) {
    const xs = out.runs[k].map((r) => r.sites.find((x) => x.site === site.name)).filter((x) => x && !x.error);
    out.medians[k][site.name] = Object.fromEntries(["avgFps", "pctOver20ms", "p95ms"].map((m) => [m, median(xs.map((x) => x[m]))]));
  }
}
out.conditionsEnd = conditions();
console.log(JSON.stringify(out.medians, null, 1));
console.log(save(NAME, out));
