// S9 Speedometer 3.1: fresh launch per run, window floated to a 1400x913 viewport, click Start, read score.
// usage: node speedometer.mjs [runs=3] [kinds=cef,chrome] [outName=speedometer]
import { launch, connect, waitCdp, hyprPlace, WIN, sleep, save, median, conditions } from "./lib.mjs";

const runs = +(process.argv[2] ?? 3);
const kinds = (process.argv[3] ?? "cef,chrome").split(",");
const NAME = process.argv[4] ?? "speedometer";
const URL = "https://browserbench.org/Speedometer3.1/";

async function once(kind) {
  const b = launch(kind, URL);
  const r = { kind, conditions: conditions() };
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
    await s.waitFor("location.hash === '#summary' && document.getElementById('result-number').textContent.trim() !== ''", 20 * 60000, 5000);
    r.durationS = (Date.now() - t0) / 1000;
    Object.assign(r, await s.eval(`({
      score: parseFloat(document.getElementById('result-number').textContent),
      confidence: document.getElementById('confidence-number').textContent.trim(),
      invalid: document.getElementById('invalid-score-text')?.textContent.trim() ?? '',
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
    console.log(i, k, JSON.stringify({ score: r.score, conf: r.confidence, dur: r.durationS, before: r.before, after: r.after, load: r.conditions.loadavg, err: r.error }));
    out.runs[k].push(r);
    save(NAME, out);
  }
for (const k of kinds) out.medians[k] = median(out.runs[k].map((r) => r.score));
out.conditionsEnd = conditions();
console.log(JSON.stringify(out.medians));
console.log(save(NAME, out));
