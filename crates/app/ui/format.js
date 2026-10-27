// Small shared helpers. Classic script (no modules) so the page also works
// from file:// in tests. Everything hangs off window.DC.
"use strict";
window.DC = window.DC || {};

(function (DC) {
  const UNITS = ["B", "KB", "MB", "GB", "TB"];

  // Binary units with Explorer-style labels, same as the CLI.
  DC.fmtBytes = function (bytes, digits) {
    let v = Number(bytes) || 0;
    let u = 0;
    while (v >= 1024 && u < UNITS.length - 1) {
      v /= 1024;
      u++;
    }
    if (u === 0) return `${v} B`;
    const d = digits ?? (v >= 100 ? 0 : 1);
    return `${v.toFixed(d)} ${UNITS[u]}`;
  };

  DC.fmtCount = (n) => Number(n || 0).toLocaleString("en-US");

  DC.pct = (part, whole) => (whole > 0 ? (part * 100) / whole : 0);

  // Last path segment, for Windows or POSIX paths.
  DC.baseName = function (p) {
    const s = String(p).replace(/[\\/]+$/, "");
    const i = Math.max(s.lastIndexOf("\\"), s.lastIndexOf("/"));
    return i >= 0 ? s.slice(i + 1) || s : s;
  };

  DC.sep = (p) => (String(p).includes("\\") ? "\\" : "/");

  // ["C:\\", "C:\\Users", "C:\\Users\\streamer"] for "C:\\Users\\streamer".
  DC.ancestors = function (p) {
    const s = String(p);
    if (s.includes("\\")) {
      const parts = s.split("\\").filter(Boolean);
      const out = [];
      let cur = parts[0] + "\\";
      out.push(cur);
      for (let i = 1; i < parts.length; i++) {
        cur = (cur.endsWith("\\") ? cur : cur + "\\") + parts[i];
        out.push(cur);
      }
      return out;
    }
    const parts = s.split("/").filter(Boolean);
    const out = ["/"];
    let cur = "";
    for (const part of parts) {
      cur += "/" + part;
      out.push(cur);
    }
    return out;
  };

  // Case-insensitive on Windows-style paths, like the engine.
  DC.normPath = function (p) {
    let s = String(p).replace(/[\\/]+$/, "");
    if (/^[a-zA-Z]:$/.test(s)) s += "\\";
    return s.includes("\\") ? s.toLowerCase() : s;
  };
  DC.isInside = function (child, parent) {
    const c = DC.normPath(child);
    const p = DC.normPath(parent);
    if (c === p) return true;
    const sep = p.includes("\\") ? "\\" : "/";
    return c.startsWith(p.endsWith(sep) ? p : p + sep);
  };

  // Tiny DOM builder: h("div", {class: "x", onclick: fn}, child, "text").
  // Never takes HTML strings, so data can't inject markup.
  DC.h = function (tag, attrs, ...children) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === undefined || v === null || v === false) continue;
      if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2), v);
      else if (k === "class") el.className = v;
      else if (k === "dataset") Object.assign(el.dataset, v);
      else if (k in el && typeof v !== "string") el[k] = v;
      else el.setAttribute(k, v === true ? "" : v);
    }
    for (const c of children.flat()) {
      if (c === null || c === undefined || c === false) continue;
      el.append(c instanceof Node ? c : document.createTextNode(String(c)));
    }
    return el;
  };

  const SVGNS = "http://www.w3.org/2000/svg";
  DC.s = function (tag, attrs, ...children) {
    const el = document.createElementNS(SVGNS, tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === undefined || v === null) continue;
      if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v);
    }
    for (const c of children.flat()) {
      if (c === null || c === undefined) continue;
      el.append(c instanceof Node ? c : document.createTextNode(String(c)));
    }
    return el;
  };

  // Clone an <svg><use> icon from the sprite in index.html.
  DC.icon = function (name, cls) {
    return DC.s("svg", { class: "icon " + (cls || ""), "aria-hidden": "true" }, DC.s("use", { href: "#i-" + name }));
  };
})(window.DC);
