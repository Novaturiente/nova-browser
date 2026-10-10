// S9 startup: spawn → first window mapped (Hyprland) and spawn → `load` of the first page.
// usage: node startup.mjs [runs=5] [kinds=cef,chrome,s2]
// Profile dir is reset once per kind, then one discarded warm-up run, then `runs` measured runs.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { launch, connect, resetProfile, tree, sleep, save, median, conditions } from "./lib.mjs";

const pexec = promisify(execFile);
const runs = +(process.argv[2] ?? 5);
const kinds = (process.argv[3] ?? "cef,chrome,s2").split(",");
const URL0 = "https://example.com/";

async function once(kind) {
  const b = launch(kind, URL0);
  const r = { kind };
  let stop = false;
  // window poll (25 ms)
  const winP = (async () => {
    while (!stop) {
      const pids = new Set(tree(b.pid));
      try {
        const cs = JSON.parse((await pexec("hyprctl", ["clients", "-j"])).stdout);
        if (cs.some((c) => pids.has(c.pid) && c.mapped)) return Date.now() - b.t0;
      } catch {}
      await sleep(25);
    }
    return null;
  })();
  try {
    // CDP up
    for (;;) {
      try { await (await fetch(`http://localhost:${b.port}/json/version`)).json(); break; } catch { await sleep(10); }
      if (Date.now() - b.t0 > 20000) throw new Error("no CDP");
    }
    r.cdpUpMs = Date.now() - b.t0;
    const c = await connect(b.port);
    const want = kind === "s2" ? "wikipedia.org" : "example.com";
    let page;
    for (const end = Date.now() + 20000; !page && Date.now() < end; await sleep(20))
      page = (await c.pages()).find((p) => p.url.includes(want));
    if (!page) throw new Error("no page");
    const s = await c.attach(page.targetId);
    await s.waitFor("performance.timing.loadEventEnd > 0", 30000, 20);
    await s.waitFor('performance.getEntriesByType("paint").some(e => e.name === "first-contentful-paint")', 3000, 20).catch(() => {});
    const t = await s.eval(`(() => {
      const n = performance.getEntriesByType("navigation")[0];
      const p = Object.fromEntries(performance.getEntriesByType("paint").map(e => [e.name, e.startTime]));
      return { timeOrigin: performance.timeOrigin, loadEventEnd: n.loadEventEnd, responseEnd: n.responseEnd,
               fcp: p["first-contentful-paint"] ?? null, url: location.href };
    })()`);
    r.navStartMs = t.timeOrigin - b.t0; // spawn → navigation start
    r.loadMs = t.timeOrigin + t.loadEventEnd - b.t0; // spawn → load event end (wall clock)
    r.fcpMs = t.fcp == null ? null : t.timeOrigin + t.fcp - b.t0;
    r.navLoadMs = t.loadEventEnd; // navigation start → load (network-dominated)
    r.url = t.url;
    r.windowMs = await Promise.race([winP, sleep(3000).then(() => null)]);
    c.close();
  } catch (e) {
    r.error = String(e);
  }
  stop = true;
  await b.kill();
  await sleep(1500);
  return r;
}

const out = { conditions: conditions(), runs: {}, medians: {} };
for (const k of kinds) { resetProfile(k); out.runs[k] = []; console.log("warm-up", k, JSON.stringify(await once(k))); }
for (let i = 0; i < runs; i++)
  for (const k of kinds) { // interleaved
    const r = await once(k);
    console.log(i, JSON.stringify(r));
    out.runs[k].push(r);
  }
for (const k of kinds)
  out.medians[k] = Object.fromEntries(["windowMs", "cdpUpMs", "navStartMs", "fcpMs", "loadMs", "navLoadMs"].map((m) => [m, median(out.runs[k].map((r) => r[m]))]));
out.conditionsEnd = conditions();
console.log(JSON.stringify(out.medians, null, 1));
console.log(save("startup", out));
