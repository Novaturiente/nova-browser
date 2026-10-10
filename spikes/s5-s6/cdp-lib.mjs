// Minimal CDP client over the *browser* websocket with flattened target sessions,
// so one connection can drive several tabs and keep their sessions open.
// Port: CDP_PORT (default 9305).
import { writeFileSync } from "node:fs";

export const PORT = process.env.CDP_PORT ?? "9305";

export async function connect() {
  const ver = await (await fetch(`http://localhost:${PORT}/json/version`)).json();
  const ws = new WebSocket(ver.webSocketDebuggerUrl);
  let id = 0;
  const pending = new Map();
  const listeners = [];
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.id && pending.has(m.id)) {
      const { resolve, reject, method } = pending.get(m.id);
      pending.delete(m.id);
      m.error ? reject(new Error(`${method}: ${JSON.stringify(m.error)}`)) : resolve(m.result);
    } else if (m.method) {
      for (const l of listeners) l(m);
    }
  };
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  const send = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      pending.set(++id, { resolve, reject, method });
      ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  const waitEvent = (method, sessionId, timeout = 15000) =>
    new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`timeout waiting ${method}`)), timeout);
      const l = (m) => {
        if (m.method === method && (!sessionId || m.sessionId === sessionId)) {
          clearTimeout(t);
          listeners.splice(listeners.indexOf(l), 1);
          resolve(m.params);
        }
      };
      listeners.push(l);
    });
  return { ws, version: ver, send, waitEvent, listeners, close: () => ws.close() };
}

// Attach (flatten) to a page target; returns a session helper.
export async function attach(c, targetId) {
  const { sessionId } = await c.send("Target.attachToTarget", { targetId, flatten: true });
  const s = {
    targetId,
    sessionId,
    send: (m, p) => c.send(m, p, sessionId),
    async eval(expression) {
      const r = await c.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, sessionId);
      if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
      return r.result.value;
    },
    async screenshot(path) {
      const r = await c.send("Page.captureScreenshot", { format: "png" }, sessionId);
      writeFileSync(path, Buffer.from(r.data, "base64"));
      return path;
    },
    async waitFor(expression, timeout = 15000) {
      for (const end = Date.now() + timeout; Date.now() < end; await sleep(100))
        if (await s.eval(expression).catch(() => false)) return;
      throw new Error(`timeout waiting for ${expression}`);
    },
    async navigate(url, event = "Page.loadEventFired", timeout = 15000) {
      await c.send("Page.enable", {}, sessionId);
      const loaded = c.waitEvent(event, sessionId, timeout);
      await c.send("Page.navigate", { url }, sessionId);
      await loaded;
    },
    async reload() {
      await c.send("Page.enable", {}, sessionId);
      const loaded = c.waitEvent("Page.loadEventFired", sessionId);
      await c.send("Page.reload", {}, sessionId);
      await loaded;
    },
    detach: () => c.send("Target.detachFromTarget", { sessionId }),
  };
  return s;
}

export async function pages(c) {
  const { targetInfos } = await c.send("Target.getTargets");
  return targetInfos.filter((t) => t.type === "page");
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
