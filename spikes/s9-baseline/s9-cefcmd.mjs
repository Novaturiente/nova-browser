import { launch, connect, waitCdp, sleep } from "/home/nova/personal/novabrowser/novabrowser/spikes/s9-baseline/lib.mjs";
for (const k of ["cef", "cft154"]) {
  const b = launch(k, "about:blank");
  try {
    await waitCdp(b.port); const c = await connect(b.port);
    let p; for (let i = 0; !p && i < 100; i++, await sleep(100)) p = (await c.pages())[0];
    const s = await c.attach(p.targetId); await s.send("Page.navigate", {url: "chrome://version"}); await sleep(2000);
    console.log("==", k, await s.eval("document.getElementById('command_line')?.innerText"));
    c.close();
  } catch (e) { console.log(k, "ERR", String(e)); }
  await b.kill(); await sleep(1500);
}
process.exit(0);
