// usage: [CDP_PORT=9333] [T=<url substring>] node cdp.mjs <expr-or-@screenshot> ; first matching page target
const [,, what, out] = process.argv;
const list = await (await fetch(`http://localhost:${process.env.CDP_PORT ?? 9333}/json/list`)).json();
const page = list.find(t => t.type === "page" && t.url.includes(process.env.T ?? ""));
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0; const pending = new Map();
ws.onmessage = e => { const m = JSON.parse(e.data); pending.get(m.id)?.(m); };
const send = (method, params = {}) => new Promise(r => { pending.set(++id, r); ws.send(JSON.stringify({ id, method, params })); });
await new Promise(r => ws.onopen = r);
if (what === "@screenshot") {
  const r = await send("Page.captureScreenshot", { format: "png" });
  (await import("node:fs")).writeFileSync(out, Buffer.from(r.result.data, "base64"));
  console.log("saved", out, page.url);
} else if (what.startsWith("@type:")) {
  await send("Input.insertText", { text: what.slice(6) });
  for (const type of ["keyDown", "keyUp"]) await send("Input.dispatchKeyEvent", { type, key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: type === "keyDown" ? "\r" : undefined });
  console.log("typed");
} else {
  const r = await send("Runtime.evaluate", { expression: what, awaitPromise: true, returnByValue: true });
  console.log(JSON.stringify(r.result.result.value ?? r.result, null, 1));
}
ws.close();
