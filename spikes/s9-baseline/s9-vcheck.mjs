import { launch, connect, waitCdp, hyprPlace, WIN, sleep } from "/home/nova/personal/novabrowser/novabrowser/spikes/s9-baseline/lib.mjs";
for (const k of (process.argv[2]||"chromenf").split(",")) {
  const b = launch(k, "about:blank");
  try {
    const v = await waitCdp(b.port); const c = await connect(b.port);
    let p; for (let i = 0; !p && i < 100; i++, await sleep(100)) p = (await c.pages())[0];
    const s = await c.attach(p.targetId); await s.send("Page.navigate", {url: "chrome://version"}); await sleep(2000); hyprPlace(b.pid, ...WIN[k]); await sleep(1500);
    const r = await s.eval("({h: innerHeight, w: innerWidth, var: (document.getElementById('variations-section')?.innerText||'').slice(0,900), cmd: document.getElementById('command_line')?.innerText.includes('disable-field-trial')})");
    console.log(k, v.Browser, JSON.stringify(r)); c.close();
  } catch (e) { console.log(k, "ERR", String(e)); }
  await b.kill(); await sleep(1500);
}
process.exit(0);
