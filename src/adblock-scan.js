// Generic cosmetic filtering, Brave-style. Runs at document start in the
// "novabrowser" isolated world (pages can't see it): reports new class names and
// ids through the __novaClassIds binding; the browser answers with hide CSS via
// __novaHide. adblock.rs installs both.
(() => {
  const send = globalThis.__novaClassIds;
  if (!send) return;
  const seenC = new Set();
  const seenI = new Set();
  let sheet = null;
  globalThis.__novaHide = (css) => {
    if (!sheet) {
      sheet = new CSSStyleSheet();
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
    }
    for (const rule of css.split("\n")) {
      try {
        sheet.insertRule(rule, sheet.cssRules.length);
      } catch {
        // one invalid selector only drops itself
      }
    }
  };
  const c = [];
  const i = [];
  const visit = (el) => {
    if (el.id && !seenI.has(el.id)) {
      seenI.add(el.id);
      i.push(el.id);
    }
    for (const k of el.classList) {
      if (!seenC.has(k)) {
        seenC.add(k);
        c.push(k);
      }
    }
  };
  let queued = [];
  let timer = 0;
  const flush = () => {
    timer = 0;
    for (const [el, deep] of queued) {
      visit(el);
      if (deep) for (const d of el.querySelectorAll("[class],[id]")) visit(d);
    }
    queued = [];
    if (c.length || i.length) {
      send(JSON.stringify({ u: location.href, c: c.splice(0), i: i.splice(0) }));
    }
  };
  // Batch DOM changes: one scan + one message per 100 ms at most.
  const later = (el, deep) => {
    queued.push([el, deep]);
    if (!timer) timer = setTimeout(flush, 100);
  };
  const start = () => {
    later(document.documentElement, true);
    new MutationObserver((ms) => {
      for (const m of ms) {
        if (m.type === "attributes") later(m.target, false);
        else for (const n of m.addedNodes) if (n.nodeType === 1) later(n, true);
      }
    }).observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "id"],
    });
  };
  if (document.documentElement) start();
  else document.addEventListener("readystatechange", start, { once: true });
})();
