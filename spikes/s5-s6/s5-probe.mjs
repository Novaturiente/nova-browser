// Probe all page targets (fresh connection, no emulation set): luma + prefers-color-scheme.
// usage: CDP_PORT=9305 node s5-probe.mjs <label> [outdir]
import { execFileSync } from "node:child_process";
import { connect, attach, pages } from "./cdp-lib.mjs";

const label = process.argv[2] ?? "probe";
const OUT = process.argv[3] ?? "/tmp/s5s6";
const c = await connect();
for (const [i, t] of (await pages(c)).entries()) {
  const s = await attach(c, t.targetId);
  const v = await s.eval(`({prefersDark: matchMedia('(prefers-color-scheme: dark)').matches, bodyBg: getComputedStyle(document.body).backgroundColor})`);
  const p = await s.screenshot(`${OUT}/s5-${label}-${i}.png`);
  const l = execFileSync("python3", ["/tmp/s5s6/brightness.py", p]).toString().match(/mean_luma=([\d.]+)/)[1];
  console.log(`[${label}] target ${i} ${t.targetId.slice(0, 8)}: luma=${l} prefersDark=${v.prefersDark} bodyBg=${v.bodyBg} url=${t.url}`);
  await s.detach();
}
c.close();
