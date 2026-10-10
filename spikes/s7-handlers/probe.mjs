// usage: CDP_PORT=9307 node probe.mjs '<js expr>'  — like spikes/cdp.mjs, but evaluates with
// userGesture:true and focus emulation on (clipboard.readText needs a focused document).
const [,, expr] = process.argv;
const list = await (await fetch(`http://localhost:${process.env.CDP_PORT ?? 9307}/json/list`)).json();
const page = list.find(t => t.type === "page" && t.url.includes(process.env.T ?? ""));
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0; const pending = new Map();
ws.onmessage = e => { const m = JSON.parse(e.data); pending.get(m.id)?.(m); };
const send = (method, params = {}) => new Promise(r => { pending.set(++id, r); ws.send(JSON.stringify({ id, method, params })); });
await new Promise(r => ws.onopen = r);
await send("Emulation.setFocusEmulationEnabled", { enabled: true });
const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true, userGesture: true });
console.log(JSON.stringify(r.result?.result?.value ?? r.result ?? r.error));
ws.close();
