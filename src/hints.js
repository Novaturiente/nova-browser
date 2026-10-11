// Hint labels. Runs in the "novabrowser" isolated world of the main frame
// (devtools::eval_isolated), so page scripts can't see or patch it. Element
// search and visibility are ported from qutebrowser javascript/webelem.js.
(() => {
  const H = (globalThis.__nova = {});
  let elems = []; // [elem, frame offset {left, top}]
  let host = null;
  let labels = [];

  const hidden = (e) => {
    const s = e.ownerDocument.defaultView.getComputedStyle(e);
    const framework = e.classList.contains("ace_text-input") || e.classList.contains("custom-control-input");
    return s.visibility !== "visible" || s.display === "none" || (s.opacity === "0" && !framework);
  };
  const visible = (e, off) => {
    if (hidden(e)) return false;
    const r = e.getBoundingClientRect();
    const top = r.top + off.top, left = r.left + off.left;
    if (top > innerHeight || r.bottom + off.top < 0 || left > innerWidth || r.right + off.left < 0) return false;
    return e.getClientRects().length > 0;
  };
  const shadowRoots = (c) => {
    const out = [];
    for (const e of c.querySelectorAll("*")) if (e.shadowRoot) out.push(e.shadowRoot, ...shadowRoots(e.shadowRoot));
    return out;
  };

  // Visible elements matching `selector`; returns their count.
  H.collect = (selector) => {
    const zero = { left: 0, top: 0 };
    const containers = [[document, zero]];
    for (const f of Array.from(window.frames)) {
      try {
        const r = f.frameElement.getBoundingClientRect();
        containers.push([f.document, { left: r.left, top: r.top }]);
      } catch (e) { /* cross-origin frame: qutebrowser skips these too */ }
    }
    for (const root of shadowRoots(document)) containers.push([root, zero]);
    elems = [];
    for (const [c, off] of containers) for (const e of c.querySelectorAll(selector)) if (visible(e, off)) elems.push([e, off]);
    return elems.length;
  };

  // st: {font, fg, bg, match, border, radius, padding} from config (hints.rs style()).
  H.draw = (ls, uppercase, st) => {
    H.clear();
    labels = ls;
    host = document.createElement("novabrowser-hints");
    // Closed shadow root: page CSS and scripts can't restyle or read the labels.
    const root = host.attachShadow({ mode: "closed" });
    const style = document.createElement("style");
    style.textContent = `
      :host { all: initial; position: fixed; inset: 0; z-index: 2147483647; pointer-events: none; }
      div { position: fixed; font: ${st.font}; color: ${st.fg}; background: ${st.bg}; border: ${st.border};
            border-radius: ${st.radius}; padding: ${st.padding}; line-height: 1.15; white-space: nowrap; }
      div[hidden] { display: none; } b { color: ${st.match}; font-weight: inherit; }`;
    root.append(style);
    H.nodes = elems.map(([e, off], i) => {
      const r = e.getClientRects()[0];
      const d = document.createElement("div");
      d.textContent = uppercase ? ls[i].toUpperCase() : ls[i];
      d.style.left = Math.max(0, r.left + off.left) + "px";
      d.style.top = Math.max(0, r.top + off.top) + "px";
      root.append(d);
      return d;
    });
    document.documentElement.append(host);
  };

  // Hide labels that don't start with `prefix`; the typed part is dimmed.
  H.filter = (prefix, uppercase) => {
    H.nodes.forEach((d, i) => {
      const l = labels[i];
      d.hidden = !l.startsWith(prefix);
      const up = (s) => (uppercase ? s.toUpperCase() : s);
      d.replaceChildren();
      const b = document.createElement("b");
      b.textContent = up(l.slice(0, prefix.length));
      d.append(b, up(l.slice(prefix.length)));
    });
  };

  // The picked element: scrolled into view, with its centre in main-frame CSS px.
  H.pick = (i) => {
    const [e, off] = elems[i];
    e.scrollIntoView({ block: "nearest", inline: "nearest" });
    const r = e.getClientRects()[0] || e.getBoundingClientRect();
    const url = (x) => (x ? new URL(x, e.ownerDocument.baseURI).href : null);
    const href = e.href instanceof SVGAnimatedString ? e.href.baseVal : e.getAttribute("href") !== null ? e.href : null;
    return {
      x: r.left + off.left + Math.min(r.width / 2, 20),
      y: r.top + off.top + r.height / 2,
      href: url(href),
      src: url(e.currentSrc || e.getAttribute("src")),
      text: (e.innerText || e.value || "").trim().slice(0, 200),
      editable: e.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(e.tagName),
    };
  };

  // `hint inputs --first`: focus the first visible input directly.
  H.focusFirst = () => {
    if (!elems.length) return false;
    elems[0][0].focus();
    return true;
  };

  H.clear = () => {
    host?.remove();
    host = null;
  };
})();
