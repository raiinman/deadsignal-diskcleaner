// Squarified treemap (Bruls, Huizing, van Wijk 2000) rendered as SVG.
// layout is pure and unit-testable; render builds DOM with no innerHTML.
"use strict";
window.DC = window.DC || {};

(function (DC) {
  // Worst aspect ratio of a row of areas laid along a side of length `side`.
  function worst(row, side) {
    let s = 0, max = -Infinity, min = Infinity;
    for (const a of row) {
      s += a;
      if (a > max) max = a;
      if (a < min) min = a;
    }
    const s2 = s * s, w2 = side * side;
    return Math.max((w2 * max) / s2, s2 / (w2 * min));
  }

  // values: positive numbers, any order. Returns rects in the same order as
  // `values`: [{x, y, w, h}]. Zero/negative values get an empty rect.
  DC.squarify = function (values, x, y, w, h) {
    const out = values.map(() => ({ x, y, w: 0, h: 0 }));
    const order = values
      .map((v, i) => [v, i])
      .filter(([v]) => v > 0)
      .sort((a, b) => b[0] - a[0]);
    const total = order.reduce((a, [v]) => a + v, 0);
    if (!total || w <= 0 || h <= 0) return out;
    const scale = (w * h) / total;
    const r = { x, y, w, h };
    let row = []; // [area, index]

    const flush = () => {
      const s = row.reduce((a, [ar]) => a + ar, 0);
      if (r.w >= r.h) {
        const cw = s / r.h;
        let cy = r.y;
        for (const [ar, i] of row) {
          const ch = ar / cw;
          out[i] = { x: r.x, y: cy, w: cw, h: ch };
          cy += ch;
        }
        r.x += cw;
        r.w -= cw;
      } else {
        const rh = s / r.w;
        let cx = r.x;
        for (const [ar, i] of row) {
          const cw = ar / rh;
          out[i] = { x: cx, y: r.y, w: cw, h: rh };
          cx += cw;
        }
        r.y += rh;
        r.h -= rh;
      }
      row = [];
    };

    for (let k = 0; k < order.length; ) {
      const [v, i] = order[k];
      const a = v * scale;
      const side = Math.min(r.w, r.h);
      const areas = row.map(([ar]) => ar);
      if (row.length === 0 || worst(areas.concat(a), side) <= worst(areas, side)) {
        row.push([a, i]);
        k++;
      } else {
        flush();
      }
    }
    if (row.length) flush();
    return out;
  };

  // Sequential steps (index 0 = smallest share). CSS supplies the actual colors
  // per theme through classes seq-0..seq-4.
  function seqStep(share) {
    if (share >= 0.25) return 4;
    if (share >= 0.12) return 3;
    if (share >= 0.05) return 2;
    if (share >= 0.015) return 1;
    return 0;
  }

  // Render `scan` (a ScanResult) into `host`. opts.onDrill(entry) for folders.
  DC.renderTreemap = function (host, scan, opts) {
    const { s, fmtBytes, fmtCount } = DC;
    host.replaceChildren();
    const W = Math.max(200, host.clientWidth);
    const H = Math.max(200, host.clientHeight);
    const tiles = scan.children
      .filter((c) => c.bytes > 0)
      .map((c) => ({ kind: c.is_dir ? "dir" : "file", entry: c }));
    if (scan.other_bytes > 0) {
      tiles.push({ kind: "other", entry: { name: `${fmtCount(scan.other_count)} smaller items`, bytes: scan.other_bytes, path: null, locked: false, is_dir: false } });
    }
    const svg = s("svg", { class: "treemap", width: W, height: H, viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": `Treemap of ${scan.root}` });
    svg.append(
      s("defs", {}, s("pattern", { id: "hatch", width: 8, height: 8, patternUnits: "userSpaceOnUse", patternTransform: "rotate(45)" }, s("rect", { width: 8, height: 8, class: "hatch-bg" }), s("line", { x1: 0, y1: 0, x2: 0, y2: 8, class: "hatch-line" })))
    );
    if (!tiles.length) {
      svg.append(s("text", { x: W / 2, y: H / 2, class: "tm-empty", "text-anchor": "middle" }, "This folder is empty"));
      host.append(svg);
      return;
    }
    const rects = DC.squarify(tiles.map((t) => t.entry.bytes), 0, 0, W, H);
    const total = scan.total_bytes || tiles.reduce((a, t) => a + t.entry.bytes, 0);
    const GAP = 2;
    tiles.forEach((t, i) => {
      const r = rects[i];
      if (r.w < 1 || r.h < 1) return;
      const e = t.entry;
      const share = e.bytes / total;
      const drillable = t.kind === "dir";
      const cls = ["tm-tile", t.kind === "dir" ? "seq-" + seqStep(share) : "tm-" + t.kind, e.locked ? "locked" : "", drillable ? "drillable" : ""].join(" ");
      const g = s("g", {
        class: cls,
        transform: `translate(${r.x},${r.y})`,
        tabindex: drillable ? 0 : -1,
        role: drillable ? "button" : null,
        "aria-label": `${e.name}, ${fmtBytes(e.bytes)}${e.locked ? ", protected" : ""}`,
        "data-path": e.path || "",
        "data-name": e.name,
        "data-locked": e.locked ? "1" : "0",
      });
      const rw = Math.max(0, r.w - GAP), rh = Math.max(0, r.h - GAP);
      g.append(s("rect", { class: "tm-fill", width: rw, height: rh, rx: 4, ry: 4 }));
      if (e.locked) g.append(s("rect", { class: "tm-hatch", width: rw, height: rh, rx: 4, ry: 4, fill: "url(#hatch)" }));
      if (rw > 56 && rh > 30) {
        const pad = 8;
        const maxChars = Math.floor((rw - pad * 2 - (e.locked ? 16 : 0)) / 7.2);
        const name = e.name.length > maxChars ? e.name.slice(0, Math.max(1, maxChars - 1)) + "…" : e.name;
        let tx = pad;
        if (e.locked) {
          const lock = DC.icon("lock", "tm-lock");
          lock.setAttribute("x", pad);
          lock.setAttribute("y", pad + 1);
          lock.setAttribute("width", 13);
          lock.setAttribute("height", 13);
          g.append(lock);
          tx += 17;
        }
        g.append(s("text", { x: tx, y: pad + 12, class: "tm-name" }, name));
        if (rh > 48) {
          // Long form if it fits, else just the size, else nothing.
          const fit = Math.floor((rw - pad * 2) / 6.8);
          const long = `${fmtBytes(e.bytes)} · ${(share * 100).toFixed(share < 0.1 ? 1 : 0)}%`;
          const short = fmtBytes(e.bytes);
          const label = long.length <= fit ? long : short.length <= fit ? short : null;
          if (label) g.append(s("text", { x: pad, y: pad + 30, class: "tm-size" }, label));
        }
      }
      if (drillable) {
        g.addEventListener("click", () => opts.onDrill(e));
        g.addEventListener("keydown", (ev) => {
          if (ev.key === "Enter" || ev.key === " ") {
            ev.preventDefault();
            opts.onDrill(e);
          }
        });
      }
      g.addEventListener("pointermove", (ev) => opts.onHover && opts.onHover(ev, t, share));
      g.addEventListener("pointerleave", () => opts.onHover && opts.onHover(null));
      svg.append(g);
    });
    host.append(svg);
  };
})(window.DC);
