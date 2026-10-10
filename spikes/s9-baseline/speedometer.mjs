// S9 Speedometer 3.1: fresh launch per run, window floated to a 1400x913 viewport, click Start, read score.
// usage: node speedometer.mjs [runs=3] [kinds=cef,chrome] [outName=speedometer] [query=""]
// Validity = #summary.valid (Speedometer shows "Error" + makes #invalid-score-text visible otherwise;
// that div is always in the DOM, so its textContent alone means nothing).
import { execFileSync } from "node:child_process";
import { launch, connect, hyprClients, waitCdp, hyprPlace, WIN, sleep, save, median, conditions } from "./lib.mjs";

const runs = +(process.argv[2] ?? 3);
const kinds = (process.argv[3] ?? "cef,chrome").split(",");
const NAME = process.argv[4] ?? "speedometer";
const URL = "https://browserbench.org/Speedometer3.1/" + (process.argv[5] ?? "");
// identical anti-throttling flags for both browsers (window stays on top, but guard against occlusion/blur)
const EXTRA = ["--disable-renderer-backgrounding", "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows"];

async function once(kind) {
  const b = launch(kind, URL, EXTRA);
  const r = { kind, conditions: conditions(), extraFlags: EXTRA };
  try {
    await waitCdp(b.port);
    const c = await connect(b.port);
    let p;
    for (const end = Date.now() + 20000; !p && Date.now() < end; await sleep(100))
      p = (await c.pages()).find((x) => x.url.includes("Speedometer"));
    const s = await c.attach(p.targetId);
    await s.waitFor("document.readyState === 'complete' && !!document.querySelector('.start-tests-button')", 30000);
    r.windows = hyprPlace(b.pid, ...WIN[kind]);
    await sleep(3000);
    r.before = await s.eval("({w: innerWidth, h: innerHeight, dpr: devicePixelRatio, vis: document.visibilityState, focus: document.hasFocus(), sizeWarning: getComputedStyle(document.getElementById('screen-size-warning')).display})");
    const t0 = Date.now();
    await s.eval("document.querySelector('.start-tests-button').click()");
    // poll visibility/focus during the run; keep the window focused via hyprctl
    r.during = [];
    for (const end = Date.now() + 20 * 60000; Date.now() < end; await sleep(5000)) {
      const st = await s.eval("({hash: location.hash, vis: document.visibilityState, focus: document.hasFocus(), res: document.getElementById('result-number').textContent.trim()})").catch((e) => ({ err: String(e) }));
      r.during.push(st);
      if (st.hash === "#summary" && st.res) break;
      if (!st.focus) for (const w of hyprClients(b.pid)) try { execFileSync("hyprctl", ["dispatch", `hl.dsp.focus({ window = "address:${w.address}" })`]); } catch {}
    }
    r.durationS = (Date.now() - t0) / 1000;
    Object.assign(r, await s.eval(`({
      score: parseFloat(document.getElementById('result-number').textContent),
      confidence: document.getElementById('confidence-number').textContent.trim(),
      summaryClass: document.getElementById('summary').className,
      valid: document.getElementById('summary').className === 'valid',
      invalidTextShown: getComputedStyle(document.getElementById('invalid-score-text')).display !== 'none',
      after: { vis: document.visibilityState, focus: document.hasFocus() },
    })`));
    c.close();
  } catch (e) {
    r.error = String(e.stack);
  }
  await b.kill();
  await sleep(3000);
  return r;
}

const out = { conditions: conditions(), url: URL, runs: {}, medians: {} };
for (const k of kinds) out.runs[k] = [];
for (let i = 0; i < runs; i++)
  for (const k of kinds) {
    const r = await once(k);
    console.log(i, k, JSON.stringify({ score: r.score, valid: r.valid, notVisible: r.during?.filter((d) => d.vis !== "visible").length, unfocused: r.during?.filter((d) => !d.focus).length, conf: r.confidence, dur: r.durationS, before: r.before, after: r.after, load: r.conditions.loadavg, err: r.error }));
    out.runs[k].push(r);
    save(NAME, out);
  }
for (const k of kinds) out.medians[k] = median(out.runs[k].filter((r) => r.valid).map((r) => r.score));
out.conditionsEnd = conditions();
console.log(JSON.stringify(out.medians));
console.log(save(NAME, out));
