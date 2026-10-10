// S5: Emulation.setAutoDarkModeOverride scoped to one tab.
// usage: CDP_PORT=9305 [URL=<tab B start url>] node s5-darkmode.mjs [outdir] [mode]
//   mode "on"  (default): browser has no force-dark flag; enable override on tab A only.
//   mode "off": browser started with --enable-features=WebContentsForceDark;
//               disable override on tab A only (the PLAN.md design: global on, per-tab off).
import { execFileSync } from "node:child_process";
import { connect, attach, pages, sleep } from "./cdp-lib.mjs";

const OUT = process.argv[2] ?? "/tmp/s5s6";
const MODE = process.argv[3] ?? "on";
const luma = (p) => execFileSync("python3", ["/tmp/s5s6/brightness.py", p]).toString().match(/mean_luma=([\d.]+)/)[1];

const c = await connect();
let ps = await pages(c);
if (ps.length < 2) {
  const { targetId } = await c.send("Target.createTarget", { url: process.env.URL ?? "https://example.com/" });
  console.log("created tab B via Target.createTarget:", targetId);
  await sleep(2500);
  ps = await pages(c);
}
console.log("page targets:", ps.map((p) => `${p.targetId.slice(0, 8)} ${p.url}`));
let A = await attach(c, ps[0].targetId);
const B = await attach(c, ps[1].targetId);
for (const s of [A, B]) { await s.send("Page.enable"); await s.send("Runtime.enable"); }

const PROBE = `({url: location.href,
  prefersDark: matchMedia('(prefers-color-scheme: dark)').matches,
  bodyBg: getComputedStyle(document.body).backgroundColor,
  htmlBg: getComputedStyle(document.documentElement).backgroundColor})`;
async function probe(label) {
  for (const [name, s] of [["A", A], ["B", B]]) {
    const v = await s.eval(PROBE);
    const p = await s.screenshot(`${OUT}/s5-${MODE}-${label}-${name}.png`);
    console.log(`[${label}] tab ${name}: luma=${luma(p)} prefersDark=${v.prefersDark} bodyBg=${v.bodyBg} url=${v.url}`);
  }
}

await probe("1-before");
await A.send("Emulation.setAutoDarkModeOverride", { enabled: MODE === "on" });
await sleep(800);
await probe("2-override-A");

await A.reload(); await sleep(800);
await probe("3-A-reloaded");

await A.navigate("https://en.wikipedia.org/wiki/Web_browser");
await B.navigate("https://en.wikipedia.org/wiki/Web_browser");
await sleep(1500);
await probe("4-both-wikipedia");

// Session lifetime: detach A's session (the one that set the override), reattach fresh.
await A.detach();
await sleep(1000);
A = await attach(c, ps[0].targetId);
await A.send("Page.enable");
await probe("5-A-session-detached-reattached");

// Clear on a fresh session (no 'enabled' = clear override).
await A.send("Emulation.setAutoDarkModeOverride", { enabled: MODE === "on" });
await sleep(500);
await probe("6-A-override-again-new-session");
await A.send("Emulation.setAutoDarkModeOverride", {});
await sleep(800);
await probe("7-A-override-cleared");

// Leave override set on A, then close the whole websocket; a separate run checks persistence.
await A.send("Emulation.setAutoDarkModeOverride", { enabled: MODE === "on" });
await sleep(500);
console.log("override re-set on A; closing browser websocket");
c.close();
