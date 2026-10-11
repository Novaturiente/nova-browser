// Login-field helper, evaluated by renderer.rs in every http(s) frame before page
// scripts. `report` is a native function (not reachable from the page) that sends
// submitted credentials to the browser; the returned object stays in Rust.
(function (report) {
  const D = document;
  const vis = (e) => !!e && e.isConnected && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== "hidden";
  const input = (e) => e instanceof HTMLInputElement && !e.disabled && !e.readOnly;
  const textish = (e) => input(e) && /^(text|email|tel|)$/.test(e.type);
  const hint = (e) => [e.name, e.id, e.autocomplete, e.placeholder, e.getAttribute("aria-label")].join(" ").toLowerCase();
  const isOtp = (e) => textish(e) && (e.autocomplete === "one-time-code" ||
    (/otp|totp|2fa|mfa|one.?time|verification|authenticat|security.?code|\bcode\b|\btoken\b/.test(hint(e)) &&
     !/zip|postal|promo|coupon|country|area|discount|gift/.test(hint(e))));
  const isUser = (e) => textish(e) && (e.type === "email" || /username|email/.test(e.autocomplete) ||
    /user|e-?mail|login|account|identifier|uname|phone/.test(hint(e)));
  const pwIn = (s) => [...s.querySelectorAll("input[type=password]")].filter((e) => vis(e) && input(e));
  const isNew = (e) => e.autocomplete === "new-password" || /new|confirm|repeat|retype|again|verify/.test(hint(e));

  function kind(e) {
    if (!input(e)) return "";
    if (e.type === "password") return isNew(e) ? "new" : "password";
    if (isOtp(e)) return "otp";
    if (isUser(e)) return "user";
    if (textish(e) && pwIn(e.form || D).length) return "user";
    return "";
  }

  // Set like a user would: native setter (React tracks it) + input/change events.
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  function set(e, v) {
    e.focus();
    setter.call(e, v);
    e.dispatchEvent(new Event("input", { bubbles: true }));
    e.dispatchEvent(new Event("change", { bubbles: true }));
  }

  // Username field for a password field: the last visible text-ish input before it.
  function userFor(pw, scope) {
    const all = [...scope.querySelectorAll("input")].filter((e) => vis(e) && textish(e) && !isOtp(e));
    const before = pw ? all.filter((e) => e.compareDocumentPosition(pw) & Node.DOCUMENT_POSITION_FOLLOWING) : all;
    return before.reverse().find(isUser) || before[0] || null;
  }

  function fill(user, pass, what) {
    const a = D.activeElement;
    if (what === "otp") {
      const t = a && kind(a) === "otp" ? a : [...D.querySelectorAll("input")].find((e) => vis(e) && isOtp(e));
      if (t) set(t, pass);
      return !!t;
    }
    if (what === "new") {
      const scope = (a && a.form) || D;
      let t = pwIn(scope).filter(isNew);
      if (!t.length && a && a.type === "password") t = [a];
      t.forEach((e) => set(e, pass));
      return t.length > 0;
    }
    const scope = (a && a.form) || D;
    const pw = a && a.type === "password" ? a : pwIn(scope)[0] || pwIn(D)[0] || null;
    const u = a && kind(a) === "user" ? a : userFor(pw, (pw && pw.form) || scope);
    if (u && user) set(u, user);
    if (pw && pass) set(pw, pass);
    if (pw) pw.focus(); else if (u) u.focus();
    return !!(u || pw);
  }

  // On submit / Enter / submit-ish click: send what a password manager would save.
  function grab(scope) {
    scope = scope || D;
    const pws = pwIn(scope).filter((e) => e.value);
    if (pws.length) {
      const pw = pws[pws.length - 1]; // change-password forms: the new one comes last
      const u = userFor(pws[0], scope);
      report(JSON.stringify({ user: u ? u.value : "", pass: pw.value }));
    } else {
      const u = [...scope.querySelectorAll("input")].find((e) => vis(e) && isUser(e) && e.value);
      if (u) report(JSON.stringify({ user: u.value }));
    }
  }
  const formOf = (e) => (e && (e.form || (e.closest && e.closest("form")))) || null;
  addEventListener("submit", (e) => grab(e.target), true);
  addEventListener("keydown", (e) => { if (e.key === "Enter" && e.target instanceof HTMLInputElement) grab(formOf(e.target)); }, true);
  addEventListener("click", (e) => {
    const b = e.target && e.target.closest && e.target.closest("button,input[type=submit],[role=button]");
    if (b) grab(formOf(b));
  }, true);

  return {
    // kind of the focused element + its rect (frame CSS px) as "kind x y w h"
    focus() {
      const e = D.activeElement, k = kind(e);
      if (!k) return "";
      const r = e.getBoundingClientRect();
      return [k, r.left, r.top, r.width, r.height].map((x) => (typeof x === "number" ? Math.round(x) : x)).join(" ");
    },
    fill,
  };
})
