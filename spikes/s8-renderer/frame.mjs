// usage: node s8frame.mjs <url-substr> <expr> — evaluate in an OOPIF (type=iframe) target
const [,, sub, expr] = process.argv;
const list = await (await fetch(`http://localhost:9308/json/list`)).json();
const t = list.find(t => t.type === "iframe" && t.url.includes(sub));
if (!t) { console.log("no iframe target", list.map(t => t.type + " " + t.url.slice(0, 60))); process.exit(0); }
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise(r => ws.onopen = r);
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id === 1) { console.log(JSON.stringify(m.result.result.value ?? m.result)); process.exit(0); } };
ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: expr, returnByValue: true, awaitPromise: true } }));
