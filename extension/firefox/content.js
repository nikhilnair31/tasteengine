// Taste Engine save feedback overlay + hotkey save engine.
"use strict";

(() => {
  const INSTANCE_KEY = "__tasteEngineSaveFeedback";
  if (window[INSTANCE_KEY]) return;
  window[INSTANCE_KEY] = true;

  const host = document.createElement("div");
  host.id = "taste-engine-save-feedback";
  host.style.cssText = "position:fixed;inset:0;z-index:2147483647;pointer-events:none";
  (document.documentElement || document.body).appendChild(host);

  const shadow = host.attachShadow({ mode: "open" });
  shadow.innerHTML = `
    <style>
      :host { all: initial; }
      * { box-sizing: border-box; }
      .te-panel {
        position: fixed;
        top: 16px;
        right: 16px;
        width: min(300px, calc(100vw - 32px));
        overflow: hidden;
        color: #ffffff;
        background: #c83044;
        border: 1px solid #e96a76;
        border-radius: 4px;
        box-shadow: 0 10px 22px rgba(28, 12, 20, .22);
        font: 400 13px/1.3 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        pointer-events: auto;
        opacity: 0;
        transform: translateY(-4px);
        transition: opacity 180ms ease, transform 180ms ease;
      }
      .te-panel.te-in { opacity: 1; transform: translateY(0); }
      .te-panel.te-leave { opacity: 0; transform: translateY(-4px); }
      .te-panel[hidden] { display: none; }
      .te-panel.te-error { background: #9c2320; border-color: #d9533f; }
      .te-main {
        display: flex;
        align-items: center;
        gap: 10px;
        min-height: 44px;
        padding: 9px 13px;
      }
      .te-icon {
        display: grid;
        flex: 0 0 24px;
        width: 24px;
        height: 24px;
        place-items: center;
        color: #ffffff;
        background: rgba(255, 255, 255, .16);
        border: 1px solid rgba(255, 255, 255, .55);
        border-radius: 50%;
      }
      .te-icon svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 2; }
      .te-icon .te-spinner { animation: te-spin 850ms linear infinite; transform-origin: center; }
      .te-copy { min-width: 0; flex: 1; }
      .te-title { color: #ffffff; font-size: 14px; font-weight: 600; line-height: 1.25; }
      .te-subtitle {
        margin-top: 2px;
        overflow: hidden;
        color: rgba(255, 255, 255, .82);
        font-size: 11px;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .te-close {
        align-self: flex-start;
        width: 18px;
        height: 18px;
        padding: 0;
        color: rgba(255, 255, 255, .7);
        background: transparent;
        border: 0;
        cursor: pointer;
        font: 400 14px/14px system-ui, sans-serif;
      }
      .te-close:hover { color: #ffffff; }
      @keyframes teSpin { to { transform: rotate(360deg); } }
      @media (prefers-reduced-motion: reduce) {
        .te-panel { transition: none; }
        .te-icon .te-spinner { animation: none; }
      }
    </style>
    <section class="te-panel" id="te-panel" hidden role="status" aria-live="polite">
      <div class="te-main">
        <div class="te-icon" id="te-icon" aria-hidden="true"></div>
        <div class="te-copy">
          <div class="te-title" id="te-title"></div>
          <div class="te-subtitle" id="te-subtitle"></div>
        </div>
        <button class="te-close" id="te-close" type="button" aria-label="close">×</button>
      </div>
    </section>
  `;

  const panel = shadow.querySelector("#te-panel");
  const icon = shadow.querySelector("#te-icon");
  const title = shadow.querySelector("#te-title");
  const subtitle = shadow.querySelector("#te-subtitle");
  const close = shadow.querySelector("#te-close");
  const state = { id: "", dismissTimer: null };

  const savingIcon = '<svg class="te-spinner" viewBox="0 0 24 24"><path d="M12 3a9 9 0 0 1 9 9"/><path d="M12 21a9 9 0 0 1-9-9"/></svg>';
  const savedIcon = '<svg viewBox="0 0 24 24"><path d="M6 12.5 10 17l8-10"/><circle cx="12" cy="12" r="9"/></svg>';
  const errorIcon = '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 4h.01"/></svg>';

  function show() {
    clearTimeout(state.dismissTimer);
    panel.hidden = false;
    panel.classList.remove("te-leave");
    panel.classList.remove("te-in");
    void panel.offsetWidth; // restart the fade-in transition
    panel.classList.add("te-in");
  }

  function hide() {
    clearTimeout(state.dismissTimer);
    if (panel.hidden) return;
    panel.classList.remove("te-in");
    panel.classList.add("te-leave");
    setTimeout(() => {
      if (panel.classList.contains("te-leave")) panel.hidden = true;
    }, 180);
  }

  function scheduleHide() {
    clearTimeout(state.dismissTimer);
    state.dismissTimer = setTimeout(hide, 3500);
  }

  function setSaving(label) {
    panel.classList.remove("te-error");
    icon.innerHTML = savingIcon;
    title.textContent = "One moment, saving\u2026";
    subtitle.textContent = label ? `Saving ${label.toLowerCase()} to taste engine` : "Saving to taste engine";
    show();
  }

  function setSaved(message) {
    state.id = message.id || "";
    panel.classList.remove("te-error");
    icon.innerHTML = savedIcon;
    title.textContent = "Saved to taste engine";
    subtitle.textContent = message.label ? `${message.label} is ready` : "Your item is ready";
    show();
    scheduleHide();
  }

  function setError(message) {
    panel.classList.add("te-error");
    icon.innerHTML = errorIcon;
    title.textContent = "Could not save";
    subtitle.textContent = message.error || "The item was not saved";
    show();
    scheduleHide();
  }

  close.addEventListener("click", hide);

  chrome.runtime.onMessage.addListener((message) => {
    if (!message || message.type !== "tv-save-state") return;
    if (message.phase === "saving") setSaving(message.label || "");
    else if (message.phase === "saved") setSaved(message);
    else if (message.phase === "error") setError(message);
  });

  // ---------------------------------------------------------------------------
  // Hotkey saves. The combos live in chrome.storage.local (hotkeyImage /
  // hotkeyLink, e.g. "alt+shift+s", "" = off) so the options page can edit
  // them — MV3 gives no API to rebind chrome.commands at runtime.
  // A keydown matching the image combo saves the image under the cursor (or
  // the largest on-screen image); the link combo saves the hovered link.
  // ---------------------------------------------------------------------------

  const DEFAULT_KEYS = { hotkeyImage: "alt+shift+s", hotkeyLink: "alt+shift+l" };
  const EDITABLE_SEL = "input, textarea, select, [contenteditable], [contenteditable='true'], [contenteditable='plaintext-only']";

  let combos = { image: parseCombo(DEFAULT_KEYS.hotkeyImage), link: parseCombo(DEFAULT_KEYS.hotkeyLink) };

  function normKeyName(k) {
    k = String(k || "").toLowerCase();
    if (k === "esc") return "escape";
    if (k === "del") return "delete";
    if (k === "space") return " ";
    return k;
  }

  function parseCombo(s) {
    const parts = String(s || "").trim().toLowerCase().split("+").map((p) => p.trim()).filter(Boolean);
    if (!parts.length) return null;
    const c = { alt: false, ctrl: false, meta: false, shift: false, key: "" };
    for (const p of parts) {
      if (p === "alt" || p === "option") c.alt = true;
      else if (p === "ctrl" || p === "control") c.ctrl = true;
      else if (p === "meta" || p === "cmd" || p === "command") c.meta = true;
      else if (p === "shift") c.shift = true;
      else c.key = normKeyName(p);
    }
    return c.key ? c : null;
  }

  function comboLabel(s) {
    return String(s || "").trim().toLowerCase().split("+").filter(Boolean)
      .map((p) => (p === "ctrl" || p === "control") ? "Ctrl" : (p === "meta" || p === "cmd" || p === "command") ? "Cmd" : p.charAt(0).toUpperCase() + p.slice(1))
      .join(" + ");
  }

  function eventKey(e) {
    let k = String(e.key || "").toLowerCase();
    // Alt+letter on some layouts yields a dead char (ß for Alt+S); prefer the
    // physical code letter unless the key is plain ASCII alphanumeric.
    if (!/^[a-z0-9]$/.test(k) && /^key([a-z0-9])$/i.test(e.code || "")) k = e.code.slice(3).toLowerCase();
    return k;
  }

  function matchesCombo(e, c) {
    if (!c) return false;
    return !!e.altKey === c.alt && !!e.ctrlKey === c.ctrl && !!e.metaKey === c.meta
      && !!e.shiftKey === c.shift && eventKey(e) === c.key;
  }

  async function loadCombos() {
    try {
      const cfg = await chrome.storage.local.get(DEFAULT_KEYS);
      combos.image = parseCombo(cfg.hotkeyImage);
      combos.link = parseCombo(cfg.hotkeyLink);
    } catch (e) { /* keep current */ }
  }

  // Last hovered image / link wins. Images must be http(s), loaded, and at
  // least 40px on screen so icons and tracking pixels never win.
  function isSaveableImg(el) {
    if (!el || el.tagName !== "IMG") return false;
    const src = el.currentSrc || el.src || "";
    return /^https?:/i.test(src) && el.naturalWidth > 0
      && el.clientWidth >= 40 && el.clientHeight >= 40;
  }

  const hover = { img: null, link: null };
  const mouse = { x: null, y: null };
  document.addEventListener("mousemove", (e) => {
    mouse.x = e.clientX;
    mouse.y = e.clientY;
  }, { capture: true, passive: true });
  document.addEventListener("mouseover", (e) => {
    const t = e.target instanceof Element ? e.target : null;
    if (!t) return;
    const img = t.closest("img");
    if (isSaveableImg(img)) hover.img = img;
    const a = t.closest("a[href]");
    if (a && /^https?:/i.test(a.href || "")) hover.link = a;
  }, { capture: true, passive: true });

  function fallbackImage() {
    let best = null;
    let bestArea = 0;
    for (const img of document.querySelectorAll("img")) {
      if (!isSaveableImg(img)) continue;
      const r = img.getBoundingClientRect();
      if (r.bottom <= 0 || r.top >= innerHeight || r.right <= 0 || r.left >= innerWidth) continue;
      const w = Math.min(r.right, innerWidth) - Math.max(r.left, 0);
      const h = Math.min(r.bottom, innerHeight) - Math.max(r.top, 0);
      const area = w * h;
      if (area > bestArea) { bestArea = area; best = img; }
    }
    return best;
  }

  function linkPick(a) {
    const text = (a.innerText || a.textContent || a.getAttribute("aria-label") || "")
      .replace(/\s+/g, " ").trim();
    return { type: "link", href: a.href, text: text.slice(0, 200) };
  }

  function imgPick(img) {
    return { type: "image", src: img.currentSrc || img.src, alt: (img.alt || "").trim() };
  }

  function bgUrlFromEl(el) {
    // Sites often paint photos as CSS background-image divs (galleries,
    // cards). Walk the element and a few ancestors for a real http(s) URL.
    let node = el;
    for (let i = 0; node && node.nodeType === 1 && i < 4; i++, node = node.parentElement) {
      const bg = (node.ownerDocument.defaultView || window).getComputedStyle(node).backgroundImage || "";
      const m = /url\((['"]?)(https?:\/\/[^'")]+)\1\)/.exec(bg);
      if (m) return m[2];
    }
    return "";
  }

  // What sits under the cursor RIGHT NOW (top of the hit-test stack).
  // Handles virtualized grids that re-render nodes, hover overlays painted
  // above images, and background-image divs — a mouseover snapshot goes
  // stale in all of those.
  function pickAtPoint(type) {
    if (mouse.x == null || mouse.y == null) return null;
    let stack;
    try {
      stack = document.elementsFromPoint(mouse.x, mouse.y);
    } catch (e) {
      return null;
    }
    if (!stack || !stack.length) return null;
    for (const el of stack.slice(0, 12)) {
      if (type === "image") {
        const img = el && el.closest ? el.closest("img") : null;
        if (isSaveableImg(img)) return imgPick(img);
      } else {
        const a = el && el.closest ? el.closest("a[href]") : null;
        if (a && /^https?:/i.test(a.href || "")) return linkPick(a);
      }
    }
    if (type === "image") {
      for (const el of stack.slice(0, 8)) {
        const u = bgUrlFromEl(el);
        if (u) return { type: "image", src: u, alt: "" };
      }
    }
    return null;
  }

  function pickFor(type) {
    // 1. exactly under the cursor at keypress
    const at = pickAtPoint(type);
    if (at) return at;
    // 2. last hovered (tracked) element, if still attached and valid
    if (type === "image") {
      if (hover.img && document.contains(hover.img) && isSaveableImg(hover.img)) {
        return imgPick(hover.img);
      }
    } else if (hover.link && document.contains(hover.link)) {
      return linkPick(hover.link);
    }
    // 3. largest visible image
    if (type === "image") {
      const img = fallbackImage();
      if (img) return imgPick(img);
    }
    return null;
  }

  function inEditable(e) {
    const t = e.target;
    if (!(t instanceof Element)) return false;
    if (t.closest(EDITABLE_SEL)) return true;
    return !!t.isContentEditable;
  }

  function fire(type) {
    const pick = pickFor(type);
    if (!pick) {
      setError({ error: type === "image"
        ? "no image found \u2014 hover one first"
        : "no link found \u2014 hover one first" });
      return;
    }
    setSaving(type === "image" ? "Image" : "Link");
    if (pick.type !== "image") {
      try {
        chrome.runtime.sendMessage({ type: "tv-save-pick", pick }, () => void chrome.runtime.lastError);
      } catch (e) { /* panel error comes from the background on failure */ }
      return;
    }
    // Images: fetch the bytes FROM THE PAGE (own session/cookies) so
    // private/self-hosted sources (Immich, authed galleries) work. The
    // vault cannot re-download those URLs — it gets bounced to a login.
    fetch(pick.src, { credentials: "include", mode: "cors", redirect: "follow" })
      .then((res) => {
        if (!res.ok) throw new Error("HTTP " + res.status);
        return res.blob();
      })
      .then((blob) => {
        if (!blob || blob.size < 200) throw new Error("empty image response");
        return new Promise((resolve, reject) => {
          const fr = new FileReader();
          fr.onload = () => resolve({ dataUrl: fr.result, mime: blob.type || "" });
          fr.onerror = () => reject(new Error("could not read image bytes"));
          fr.readAsDataURL(blob);
        });
      })
      .then(({ dataUrl, mime }) => {
        pick.bytes = dataUrl;
        pick.mime = mime;
        try {
          chrome.runtime.sendMessage({ type: "tv-save-pick", pick }, () => void chrome.runtime.lastError);
        } catch (e) { /* ignore */ }
      })
      .catch(() => {
        // CORS or auth blocked the page fetch — let the vault try its own
        // download (works for public hosts like pbs.twimg / itch / steam).
        try {
          chrome.runtime.sendMessage({ type: "tv-save-pick", pick }, () => void chrome.runtime.lastError);
        } catch (e) { /* ignore */ }
      });
  }

  document.addEventListener("keydown", (e) => {
    if (e.repeat || e.isComposing) return;
    // Inside a typed field, only modifier-carrying combos may fire so plain
    // letter keys keep typing normally.
    if (inEditable(e) && !(e.altKey || e.ctrlKey || e.metaKey)) return;
    if (matchesCombo(e, combos.image)) {
      e.preventDefault();
      e.stopPropagation();
      fire("image");
    } else if (matchesCombo(e, combos.link)) {
      e.preventDefault();
      e.stopPropagation();
      fire("link");
    }
  }, { capture: true });

  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === "local" && (changes.hotkeyImage || changes.hotkeyLink)) loadCombos();
  });
  try {
    chrome.runtime.onMessage.addListener((m) => {
      if (m && m.type === "tv-hotkeys-changed") loadCombos();
    });
  } catch (e) { /* ignore */ }

  loadCombos();
})();
