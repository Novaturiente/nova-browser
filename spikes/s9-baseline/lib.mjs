// S9 baseline helpers: launch CEF (cefsimple) or Chrome with matched flags,
// process-tree memory, Hyprland window placement, CDP via ../s5-s6/cdp-lib.mjs.
import { spawn, execFileSync } from "node:child_process";
import { readdirSync, readFileSync, rmSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { homedir } from "node:os";

export const SITES = [
  { name: "youtube", url: "https://www.youtube.com/" },
  { name: "github", url: "https://github.com/rust-lang/rust" },
  { name: "maps", url: "https://www.google.com/maps/@28.6139,77.209,13z" },
  { name: "reddit", url: "https://www.reddit.com/r/programming/" },
  { name: "wikipedia", url: "https://en.wikipedia.org/wiki/World_War_II" },
];
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const HERE = new URL(".", import.meta.url).pathname;
export const RESULTS = `${HERE}results`;
mkdirSync(RESULTS, { recursive: true });

const CEF_DIR = `${homedir()}/.local/share/cef`;
export const BROWSERS = {
  cef: {
    bin: `${HERE}../../../cef-rs/target/release/cefsimple`,
    port: 9309,
    dir: "/tmp/novabrowser-s9",
    args: (url) => [
      "--use-alloy-style",
      "--ozone-platform=wayland",
      "--window-size=1400,1000",
      "--remote-debugging-port=9309",
      "--root-cache-path=/tmp/novabrowser-s9",
      `--url=${url}`,
    ],
    env: { LD_LIBRARY_PATH: CEF_DIR },
  },
  s2: {
    bin: `${HERE}../s2-layout/target/release/s2-layout`,
    port: 9309,
    dir: "/tmp/novabrowser-s9",
    args: () => [
      "--use-alloy-style",
      "--ozone-platform=wayland",
      "--window-size=1400,1000",
      "--remote-debugging-port=9309",
      "--root-cache-path=/tmp/novabrowser-s9",
    ],
    env: { LD_LIBRARY_PATH: CEF_DIR },
  },
  chrome: {
    bin: "/usr/sbin/google-chrome-stable", // wrapper -> exec /opt/google/chrome/chrome
    port: 9319,
    dir: "/tmp/s9-chrome",
    args: (url) => [
      "--user-data-dir=/tmp/s9-chrome",
      "--ozone-platform=wayland",
      "--window-size=1400,1000",
      "--remote-debugging-port=9319",
      // Chrome-only UI suppression (no CEF equivalent needed):
      "--no-first-run",
      "--no-default-browser-check",
      "--password-store=basic",
      url,
    ],
    env: {},
  },
};
// Field-trial isolation (S9 follow-up): no server seed, no testing config, fresh profile each launch.
const NOFIELD = ["--disable-variations-seed-fetch", "--disable-field-trial-config"];
const chromeLike = (bin, dir) => ({
  bin, port: 9319, dir, fresh: true,
  args: (url) => [`--user-data-dir=${dir}`, "--ozone-platform=wayland", "--window-size=1400,1000", "--remote-debugging-port=9319",
    "--no-first-run", "--no-default-browser-check", "--password-store=basic", ...NOFIELD, url],
  env: {},
});
BROWSERS.chromenf = chromeLike("/usr/sbin/google-chrome-stable", "/tmp/s9-chrome-nf");
BROWSERS.cft154 = chromeLike("/tmp/cft154/chrome-linux64/chrome", "/tmp/s9-cft154");
// cefsimple with Chrome-style window (no --use-alloy-style)
BROWSERS.cefcs = { ...BROWSERS.cef, args: (url) => BROWSERS.cef.args(url).filter((a) => a !== "--use-alloy-style") };

const live = new Set();
function killAll() {
  for (const b of live) b.killSync();
}
process.on("exit", killAll);
for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => { killAll(); process.exit(130); });

export function resetProfile(kind) {
  rmSync(BROWSERS[kind].dir, { recursive: true, force: true });
}

export function launch(kind, url = "about:blank", extra = []) {
  const b = BROWSERS[kind];
  if (b.fresh) rmSync(b.dir, { recursive: true, force: true });
  const args = [...b.args(url), ...extra];
  const t0 = Date.now();
  // outer `timeout` = safety net if this node process dies without cleanup
  const child = spawn("timeout", ["-k", "5", "1500", b.bin, ...args], {
    detached: true,
    stdio: ["ignore", "ignore", "ignore"],
    env: { ...process.env, ...b.env },
  });
  const h = {
    kind, port: b.port, t0, child, args,
    get pid() { return child.pid; },
    tree: () => tree(child.pid),
    killSync() {
      const pids = tree(child.pid);
      try { process.kill(-child.pid, "SIGTERM"); } catch {}
      for (const p of pids) try { process.kill(p, "SIGTERM"); } catch {}
      live.delete(h);
    },
    async kill() {
      const pids = tree(child.pid);
      try { process.kill(-child.pid, "SIGTERM"); } catch {}
      for (const p of pids) try { process.kill(p, "SIGTERM"); } catch {}
      for (let i = 0; i < 50 && pids.some(alive); i++) await sleep(100);
      for (const p of pids) if (alive(p)) try { process.kill(p, "SIGKILL"); } catch {}
      try { process.kill(-child.pid, "SIGKILL"); } catch {}
      for (let i = 0; i < 30 && pids.some(alive); i++) await sleep(100);
      live.delete(h);
    },
  };
  live.add(h);
  return h;
}

const alive = (p) => existsSync(`/proc/${p}`) && !readFileSync(`/proc/${p}/stat`, "utf8").includes(") Z ");

export function tree(root) {
  const kids = new Map();
  for (const d of readdirSync("/proc")) {
    if (!/^\d+$/.test(d)) continue;
    try {
      const s = readFileSync(`/proc/${d}/stat`, "utf8");
      const ppid = +s.slice(s.lastIndexOf(")") + 2).split(" ")[1];
      if (!kids.has(ppid)) kids.set(ppid, []);
      kids.get(ppid).push(+d);
    } catch {}
  }
  const out = [];
  const q = [root];
  while (q.length) {
    const p = q.shift();
    out.push(p);
    q.push(...(kids.get(p) ?? []));
  }
  return out;
}

// Sum PSS/RSS (kB) over the process tree; skips the `timeout` wrapper.
export function treeMem(root) {
  const procs = [];
  for (const p of tree(root)) {
    try {
      const comm = readFileSync(`/proc/${p}/comm`, "utf8").trim();
      if (comm === "timeout" || comm === "cat") continue; // wrappers
      const cmd = readFileSync(`/proc/${p}/cmdline`, "utf8").split(/[\0 ]/); // Chrome rewrites argv into one string
      const type = (cmd.find((a) => a.startsWith("--type=")) ?? "--type=browser").slice(7);
      let pss = null, rss = null, uss = null;
      try {
        const r = readFileSync(`/proc/${p}/smaps_rollup`, "utf8");
        const g = (k) => +(r.match(new RegExp(`^${k}:\\s+(\\d+)`, "m"))?.[1] ?? 0);
        pss = g("Pss"); rss = g("Rss"); uss = g("Private_Clean") + g("Private_Dirty");
      } catch {
        rss = +(readFileSync(`/proc/${p}/status`, "utf8").match(/^VmRSS:\s+(\d+)/m)?.[1] ?? 0);
      }
      procs.push({ pid: p, comm, type, pss, rss, uss });
    } catch {}
  }
  const sum = (k) => procs.reduce((a, x) => a + (x[k] ?? 0), 0);
  return { pssKB: sum("pss"), rssKB: sum("rss"), ussKB: sum("uss"), pssMissing: procs.filter((x) => x.pss === null).length, nprocs: procs.length, procs };
}

// Hyprland: float + size + place every window owned by the browser process tree.
export function hyprClients(root) {
  const pids = new Set(tree(root));
  const all = JSON.parse(execFileSync("hyprctl", ["clients", "-j"], { encoding: "utf8" }));
  return all.filter((c) => pids.has(c.pid));
}
// Window sizes chosen so both get a 1400x913 page viewport: Chrome's toolbar is 87 px, cefsimple (Alloy) has none.
export const WIN = { cef: [1400, 913], s2: [1400, 913], chrome: [1400, 1000], chromenf: [1400, 1000], cft154: [1400, 1000], cefcs: [1400, 1000] };
export function hyprPlace(root, w = 1400, h = 1000) {
  const cs = hyprClients(root);
  for (const [i, c] of cs.entries()) {
    // Hyprland 0.56 Lua dispatchers
    const a = `window = "address:${c.address}"`;
    const d = (x) => execFileSync("hyprctl", ["dispatch", x]);
    if (!c.floating) d(`hl.dsp.window.float({ action = "enable", ${a} })`);
    d(`hl.dsp.window.resize({ x = ${w}, y = ${h}, ${a} })`);
    d(`hl.dsp.window.move({ x = ${260 + i * 10}, y = ${100 + i * 10}, ${a} })`);
  }
  return hyprClients(root).map((c) => ({ class: c.class, title: c.title, size: c.size, at: c.at, floating: c.floating, workspace: c.workspace.name }));
}

export async function waitCdp(port, timeout = 20000) {
  for (const end = Date.now() + timeout; Date.now() < end; await sleep(20)) {
    try {
      return await (await fetch(`http://localhost:${port}/json/version`)).json();
    } catch {}
  }
  throw new Error(`no CDP on ${port}`);
}

export function conditions() {
  const rd = (f) => { try { return readFileSync(f, "utf8").trim(); } catch { return null; } };
  return {
    at: new Date().toISOString(),
    loadavg: rd("/proc/loadavg"),
    acOnline: rd("/sys/class/power_supply/ACAD/online"),
    battery: rd("/sys/class/power_supply/BAT1/status"),
    governor: rd("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
    epp: rd("/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference"),
    platformProfile: rd("/sys/firmware/acpi/platform_profile"),
    memAvailableKB: +rd("/proc/meminfo").match(/MemAvailable:\s+(\d+)/)[1],
  };
}

export function save(name, data) {
  const p = `${RESULTS}/${name}.json`;
  writeFileSync(p, JSON.stringify(data, null, 1));
  return p;
}

export const median = (xs) => {
  const s = [...xs].filter((x) => x != null).sort((a, b) => a - b);
  if (!s.length) return null;
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};

// CDP over the browser websocket with flattened sessions (port-parameterised
// variant of ../s5-s6/cdp-lib.mjs, whose port is fixed at import time).
export async function connect(port) {
  const ver = await (await fetch(`http://localhost:${port}/json/version`)).json();
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
    } else if (m.method) for (const l of listeners) l(m);
  };
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  const send = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      pending.set(++id, { resolve, reject, method });
      ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  const c = { version: ver, send, listeners, close: () => ws.close() };
  c.pages = async () => (await send("Target.getTargets")).targetInfos.filter((t) => t.type === "page");
  c.attach = async (targetId) => {
    const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
    const s = {
      targetId, sessionId,
      send: (m, p) => send(m, p, sessionId),
      async eval(expression, timeout) {
        const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true, userGesture: true, ...(timeout ? { timeout } : {}) }, sessionId);
        if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 500));
        return r.result.value;
      },
      async waitFor(expression, timeout = 30000, every = 200) {
        for (const end = Date.now() + timeout; Date.now() < end; await sleep(every))
          if (await s.eval(expression).catch(() => false)) return true;
        throw new Error(`timeout waiting for ${expression}`);
      },
    };
    return s;
  };
  return c;
}
