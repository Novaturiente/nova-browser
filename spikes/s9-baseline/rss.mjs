// S9 idle memory: open SITES in one browser, wait for load, idle 60 s, sum PSS/RSS/USS of the process tree.
// usage: node rss.mjs [runs=3] [modes=cef,chrome-win,chrome-tabs]
//   cef         cefsimple; site 0 via --url, others via window.open(noopener) → each its own Alloy window (all visible)
//   chrome-win  Chrome; site 0 via argv, others via Target.createTarget{newWindow:true} (all visible, like cef)
//   chrome-tabs Chrome; others as tabs in one window (only the last tab visible) — Chrome's normal usage
import { launch, connect, waitCdp, treeMem, hyprPlace, WIN, SITES, sleep, save, median, conditions } from "./lib.mjs";

const runs = +(process.argv[2] ?? 3);
const modes = (process.argv[3] ?? "cef,chrome-win,chrome-tabs").split(",");
const IDLE_MS = 60000;
const NAME = process.argv[4] ?? "rss";

const host = (u) => new URL(u).host;

async function once(mode) {
  const kind = mode.startsWith("chrome") ? "chrome" : "cef";
  const b = launch(kind, SITES[0].url);
  const r = { mode, browser: kind };
  try {
    await waitCdp(b.port);
    const c = await connect(b.port);
    let first;
    for (const end = Date.now() + 20000; !first && Date.now() < end; await sleep(100))
      first = (await c.pages()).find((p) => p.url.includes(host(SITES[0].url)));
    const s0 = await c.attach(first.targetId);
    await s0.waitFor("document.readyState === 'complete'", 45000).catch(() => {});
    for (const site of SITES.slice(1)) {
      if (kind === "cef") await s0.eval(`void window.open(${JSON.stringify(site.url)}, "_blank", "noopener")`);
      else await c.send("Target.createTarget", { url: site.url, newWindow: mode === "chrome-win" });
      await sleep(500);
    }
    // wait for each site to load
    r.pages = [];
    const t0 = Date.now();
    for (const site of SITES) {
      let p;
      for (const end = Date.now() + 20000; !p && Date.now() < end; await sleep(200))
        p = (await c.pages()).find((x) => x.url.includes(host(site.url)));
      if (!p) { r.pages.push({ site: site.name, error: "no target" }); continue; }
      const s = await c.attach(p.targetId);
      const ok = await s.waitFor("document.readyState === 'complete'", 45000).then(() => true, () => false);
      const info = await s.eval("({url: location.href, title: document.title, vis: document.visibilityState, text: document.body?.innerText.length ?? 0, w: innerWidth, h: innerHeight})").catch((e) => ({ error: String(e) }));
      r.pages.push({ site: site.name, loaded: ok, ...info });
      await c.send("Target.detachFromTarget", { sessionId: s.sessionId }).catch(() => {});
    }
    r.loadAllMs = Date.now() - t0;
    await c.send("Target.detachFromTarget", { sessionId: s0.sessionId }).catch(() => {});
    r.windows = hyprPlace(b.pid, ...WIN[kind]);
    c.close(); // no CDP session during idle
    r.samples = [];
    for (const at of [0, 30000, IDLE_MS]) {
      await sleep(at - (r.samples.at(-1)?.at ?? 0));
      const m = treeMem(b.pid);
      r.samples.push({ at, pssKB: m.pssKB, rssKB: m.rssKB, ussKB: m.ussKB, nprocs: m.nprocs, pssMissing: m.pssMissing });
      if (at === IDLE_MS) r.procs = m.procs;
    }
    Object.assign(r, r.samples.at(-1));
  } catch (e) {
    r.error = String(e.stack);
  }
  await b.kill();
  await sleep(2000);
  return r;
}

const out = { conditions: conditions(), sites: SITES, idleMs: IDLE_MS, runs: {}, medians: {} };
for (const m of modes) out.runs[m] = [];
for (let i = 0; i < runs; i++)
  for (const m of modes) {
    const r = await once(m);
    console.log(i, m, JSON.stringify({ pssMB: r.pssKB / 1024, rssMB: r.rssKB / 1024, ussMB: r.ussKB / 1024, nprocs: r.nprocs, err: r.error, pages: r.pages?.map((p) => `${p.site}:${p.loaded}:${p.vis}:${p.title?.slice(0, 40)}`) }));
    out.runs[m].push(r);
    save(NAME, out);
  }
for (const m of modes)
  out.medians[m] = Object.fromEntries(["pssKB", "rssKB", "ussKB", "nprocs"].map((k) => [k, median(out.runs[m].filter((r) => !r.error && r.nprocs).map((r) => r[k]))]));
out.conditionsEnd = conditions();
console.log(JSON.stringify(out.medians, null, 1));
console.log(save(NAME, out));
