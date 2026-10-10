// Probe: launch one browser, report CDP version, GPU info, createTarget support, mem access, window placement.
import { launch, waitCdp, treeMem, hyprPlace, sleep, save, resetProfile } from "./lib.mjs";
import { connect } from "./lib.mjs";
const kind = process.argv[2];
resetProfile(kind);
const b = launch(kind, "https://example.com");
const out = { kind };
try {
  out.version = await waitCdp(b.port);
  process.env.CDP_PORT = String(b.port);
  await sleep(3000);
  out.windows = hyprPlace(b.pid);
  const c = await connect(b.port);
  const gi = await c.send("SystemInfo.getInfo").catch((e) => ({ error: String(e) }));
  out.gpu = gi.gpu ? { featureStatus: gi.gpu.featureStatus, devices: gi.gpu.devices, driverBugWorkarounds: gi.gpu.driverBugWorkarounds?.length, aux: gi.gpu.auxAttributes } : gi;
  out.createTarget = await c.send("Target.createTarget", { url: "https://example.org" }).catch((e) => ({ error: String(e) }));
  await sleep(3000);
  out.pages = (await c.pages()).map((p) => p.url);
  out.windowsAfter = hyprPlace(b.pid);
  const p = (await c.pages())[0];
  const s = await c.attach(p.targetId);
  out.vp = await s.eval("({w:innerWidth,h:innerHeight,dpr:devicePixelRatio,vis:document.visibilityState,ua:navigator.userAgent})");
  out.mem = treeMem(b.pid);
  c.close();
} catch (e) { out.error = String(e.stack); }
await b.kill();
console.log(JSON.stringify({ ...out, mem: { ...out.mem, procs: out.mem?.procs.map((p) => `${p.comm}/${p.type} pss=${p.pss} rss=${p.rss}`) } }, null, 1));
save(`probe-${kind}`, out);
