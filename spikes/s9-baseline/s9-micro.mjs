import { launch, connect, waitCdp, sleep, BROWSERS, tree } from "/home/nova/personal/novabrowser/novabrowser/spikes/s9-baseline/lib.mjs";
const BENCH = `(() => {
  const t = (f) => { const r = []; for (let i = 0; i < 7; i++) { const a = performance.now(); f(); r.push(performance.now() - a); } r.sort((x, y) => x - y); return +r[3].toFixed(1); };
  const fib = (n) => n < 2 ? n : fib(n - 1) + fib(n - 2);
  return {
    jsFib: t(() => fib(30)),
    jsAlloc: t(() => { let s = 0; for (let i = 0; i < 1e6; i++) { const o = {a: i, b: [i, i]}; s += o.b[1]; } return s; }),
    jsString: t(() => { let s = ""; for (let i = 0; i < 2e5; i++) s += i.toString(36); return JSON.parse(JSON.stringify(s.split(""))).length; }),
    domBuild: t(() => { const d = document.createElement("div"); for (let i = 0; i < 20000; i++) { const e = document.createElement("span"); e.textContent = "x" + i; d.appendChild(e); } document.body.appendChild(d); d.offsetHeight; d.remove(); }),
    layout: t(() => { const d = document.createElement("div"); d.innerHTML = "<p style='width:50%'>lorem ipsum dolor</p>".repeat(3000); document.body.appendChild(d); for (let i = 0; i < 20; i++) { d.style.width = (300 + i) + "px"; d.offsetHeight; } d.remove(); }),
  };
})()`;
BROWSERS.cefje = { ...BROWSERS.cef, env: { ...BROWSERS.cef.env, LD_PRELOAD: "/usr/lib/libjemalloc.so.2" } };
BROWSERS.ceftc = { ...BROWSERS.cef, env: { ...BROWSERS.cef.env, LD_PRELOAD: "/usr/lib/libtcmalloc_minimal.so.4" } };
import { readFileSync } from "node:fs";
const out = {};
for (const round of [1, 2]) for (const k of ["cef", "cefje", "ceftc", "cft154"]) {
  const b = launch(k, "about:blank");
  try {
    await waitCdp(b.port); const c = await connect(b.port);
    let p; for (let i = 0; !p && i < 100; i++, await sleep(100)) p = (await c.pages())[0];
    const s = await c.attach(p.targetId); await s.send("Page.navigate", {url: "data:text/html,<body>"}); await sleep(2500);
    const pre = tree(b.pid).filter(p => { try { return readFileSync(`/proc/${p}/cmdline`,"utf8").includes("type=renderer"); } catch { return false; } }).map(p => { try { return /jemalloc|tcmalloc/.exec(readFileSync(`/proc/${p}/maps`,"utf8"))?.[0] ?? "none"; } catch (e) { return "unreadable"; } });
    const r = await s.eval(BENCH); r.rendererAlloc = pre.join(","); console.log(round, k, JSON.stringify(r)); c.close();
  } catch (e) { console.log(k, "ERR", String(e)); }
  await b.kill(); await sleep(1500);
}
process.exit(0);
