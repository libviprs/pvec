// The PVec viewer (ADR 0020): a page over the loadable pvec-wasm module, drawing in source space on a 2D canvas.
// The module does the reading, culling and tessellation; this file holds the view, the input and the drawing.
// docs/WASM_ABI.md is the contract every call below is written against.

const canvas = document.getElementById("view");
const statusLine = document.getElementById("status");
const params = new URLSearchParams(location.search);

// docs/WASM_ABI.md, "Error codes"
const CODES = {
  "-1": "the handle was released",
  "-2": "the view is stale",
  "-3": "an argument is out of range",
  "-4": "not a PVec file this reader accepts",
  "-5": "the handle is the wrong kind",
  "-6": "the file has no spatial directory",
  "-7": "the call needs byte ranges",
  "-8": "the file has no raster stream",
  "-9": "the tile is not stored",
  "-10": "the raster is skipped by a rule",
  "-11": "the window needs more vertices than one result holds",
};

class ModuleError extends Error {
  constructor(call, code) {
    super(`${call}: ${CODES[code] || "error"} (code ${code})`);
    this.code = code;
  }
}

// Loads the module the way tools/wasm-raster-probe.mjs does: its one import is never called with anything we
// need to answer, so every import is a stub, and the ABI version is checked before anything else.
async function loadModule(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`the module ${url}: HTTP ${response.status}`);
  const compiled = await WebAssembly.compile(await response.arrayBuffer());
  const imports = {};
  for (const { module, name } of WebAssembly.Module.imports(compiled)) {
    imports[module] ??= {};
    imports[module][name] = () => 0;
  }
  const { exports: x } = await WebAssembly.instantiate(compiled, imports);
  if (x.__wbindgen_start) x.__wbindgen_start();
  if (typeof x.wasm_abi_version !== "function" || x.wasm_abi_version() !== 2) {
    throw new Error("the module does not speak wasm ABI 2");
  }
  return x;
}

// A call that returns a handle or a count: a negative number is an error code.
function check(call, value) {
  if (value < 0) throw new ModuleError(call, value);
  return value;
}

// Copies `array` (a typed array) into module memory and returns [ptr, len] for a `&[T]` argument.
function pass(x, array) {
  const ptr = x.__wbindgen_malloc(array.byteLength, array.BYTES_PER_ELEMENT) >>> 0;
  new Uint8Array(x.memory.buffer, ptr, array.byteLength).set(new Uint8Array(array.buffer, array.byteOffset, array.byteLength));
  return [ptr, array.length];
}

// One array of a tessellation result, as a fresh view (memory may have grown since the last one).
function tessArray(x, result, which, Type) {
  const len = check("buffer_len", x.buffer_len(result, which));
  return new Type(x.memory.buffer, x.buffer_ptr(result, which) >>> 0, len);
}

// One array of a raster result (placement, plan or tile), as a fresh view.
function rasterArray(x, result, which, Type) {
  const len = check("raster_buffer_len", x.raster_buffer_len(result, which));
  return new Type(x.memory.buffer, x.raster_buffer_ptr(result, which) >>> 0, len);
}

// `m` after `n`, both in PVec's [a b c d tx ty] order (rows [a b tx] and [c d ty]).
function compose(m, n) {
  return [
    m[0] * n[0] + m[1] * n[2],
    m[0] * n[1] + m[1] * n[3],
    m[2] * n[0] + m[3] * n[2],
    m[2] * n[1] + m[3] * n[3],
    m[0] * n[4] + m[1] * n[5] + m[4],
    m[2] * n[4] + m[3] * n[5] + m[5],
  ];
}

function rule(family, number) {
  return `${family === 1 ? "LIMIT" : "RASTER"}-${String(number).padStart(2, "0")}`;
}

const state = {
  x: null, // the module's exports
  file: -1, // the open file handle
  name: "",
  extent: null, // [min_x, min_y, max_x, max_y] in source units
  view: { cx: 0, cy: 0, scale: 1 }, // CSS pixels per source unit, and the source point at the centre
  settled: false,
  pending: null,
  fitted: false,
  rasters: [], // per RasterId: { p: pixel_to_source, f: the fourteen u32 fields } or null when skipped
  tiles: new Map(), // "raster/level/col/row" -> ImageBitmap, or null for a tile that draws nothing
  decoding: new Map(), // the same keys -> the promise of the decode in flight
};

const TILE_CACHE = 512;

function size() {
  return [canvas.clientWidth, canvas.clientHeight];
}

function toScreen(x, y) {
  const [w, h] = size();
  const { cx, cy, scale } = state.view;
  return [w / 2 + (x - cx) * scale, h / 2 - (y - cy) * scale];
}

function sourceAt(px, py) {
  const [w, h] = size();
  const { cx, cy, scale } = state.view;
  return [cx + (px - w / 2) / scale, cy - (py - h / 2) / scale];
}

function window_() {
  const [w, h] = size();
  const [x0, y1] = sourceAt(0, 0);
  const [x1, y0] = sourceAt(w, h);
  return [x0, y0, x1, y1];
}

function fit() {
  const [w, h] = size();
  if (!state.extent || w === 0 || h === 0) return;
  const [x0, y0, x1, y1] = state.extent;
  const margin = 16;
  const sw = Math.max(x1 - x0, 1e-9);
  const sh = Math.max(y1 - y0, 1e-9);
  state.view.scale = Math.min((w - 2 * margin) / sw, (h - 2 * margin) / sh);
  state.view.cx = (x0 + x1) / 2;
  state.view.cy = (y0 + y1) / 2;
  state.fitted = true;
  schedule();
}

function setStatus(text, error = false) {
  statusLine.textContent = text;
  statusLine.classList.toggle("error", error);
}

function token(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

// Source to CSS pixels, as an affine in PVec order.
function viewAffine() {
  const [w, h] = size();
  const { cx, cy, scale } = state.view;
  return [scale, 0, 0, -scale, w / 2 - scale * cx, h / 2 + scale * cy];
}

function tileKey(r, level, col, row) {
  return `${r}/${level}/${col}/${row}`;
}

// Reads a tile's encoded bytes from the module and decodes them in the background; the frame that asked draws
// without it, and the decode schedules the next one. The module decodes nothing (ADR 0018): the browser does,
// with orientation and colour space left alone (WASM_ABI.md, "Rasters").
function decodeTile(r, level, col, row) {
  const key = tileKey(r, level, col, row);
  if (state.tiles.has(key) || state.decoding.has(key)) return;
  const { x, file } = state;
  const handle = x.raster_tile(file, r, level, col, row);
  if (handle === -9) {
    state.tiles.set(key, null);
    return;
  }
  check("raster_tile", handle);
  const bytes = rasterArray(x, handle, 0, Uint8Array).slice();
  x.release(handle);
  const job = createImageBitmap(new Blob([bytes]), { imageOrientation: "none", colorSpaceConversion: "none" })
    .then((bitmap) => {
      state.tiles.set(key, bitmap);
      while (state.tiles.size > TILE_CACHE) {
        const [oldest, old] = state.tiles.entries().next().value;
        if (old) old.close();
        state.tiles.delete(oldest);
      }
    })
    .catch(() => state.tiles.set(key, null))
    .finally(() => {
      state.decoding.delete(key);
      schedule();
    });
  state.decoding.set(key, job);
}

// The rasters of one band in the window: `raster_plan` picks each raster's level and tiles for the display scale,
// and every tile is drawn through view ∘ pixel_to_source ∘ (the level's pixel size, the tile's place), clipped to
// the visible box so a padded edge never shows.
function drawRasters(ctx, band) {
  const { x, file } = state;
  if (!state.rasters.some((r) => r && r.f[12] === band)) return;
  const dpr = window.devicePixelRatio || 1;
  const [x0, y0, x1, y1] = window_();
  const plan = check("raster_plan", x.raster_plan(file, x0, y0, x1, y1, state.view.scale * dpr, 0));
  try {
    const ids = rasterArray(x, plan, 0, Uint32Array).slice();
    const levels = rasterArray(x, plan, 1, Uint32Array).slice();
    const table = rasterArray(x, plan, 2, Uint32Array).slice();
    const cells = rasterArray(x, plan, 3, Uint32Array).slice();
    const view = viewAffine();
    for (let i = 0; i < ids.length; i++) {
      const raster = state.rasters[ids[i]];
      if (!raster || raster.f[12] !== band) continue;
      const { p, f } = raster;
      const [vx0, vy0, vx1, vy1] = [f[2], f[3], f[4], f[5]];
      const edge = f[6];
      const level = levels[i];
      const k = 2 ** (f[7] - 1 - level);
      const toPixels = compose(view, p);
      ctx.save();
      ctx.globalAlpha = f[11] / 255;
      ctx.setTransform(dpr * toPixels[0], dpr * toPixels[2], dpr * toPixels[1], dpr * toPixels[3], dpr * toPixels[4], dpr * toPixels[5]);
      ctx.beginPath();
      ctx.rect(vx0, vy0, vx1 - vx0, vy1 - vy0);
      ctx.clip();
      for (let t = table[i]; t < table[i + 1]; t++) {
        const [col, row] = [cells[2 * t], cells[2 * t + 1]];
        const key = tileKey(ids[i], level, col, row);
        const bitmap = state.tiles.get(key);
        if (bitmap === undefined) decodeTile(ids[i], level, col, row);
        if (!bitmap) continue;
        const m = compose(toPixels, [k, 0, 0, k, col * edge * k, row * edge * k]);
        ctx.setTransform(dpr * m[0], dpr * m[2], dpr * m[1], dpr * m[3], dpr * m[4], dpr * m[5]);
        ctx.drawImage(bitmap, 0, 0);
      }
      ctx.restore();
    }
  } finally {
    x.release(plan);
  }
}

// The vectors of the window: `query_viewport` culls, `tessellate_batch` holds every curve to a quarter of a
// device pixel after the whole transform chain, and the vertices come back in source space.
function drawVectors(ctx) {
  const { x, file } = state;
  const dpr = window.devicePixelRatio || 1;
  const [w, h] = size();
  const { cx, cy, scale } = state.view;
  const [x0, y0, x1, y1] = window_();
  const batch = check("query_viewport", x.query_viewport(file, x0, y0, x1, y1, 1 << 22));
  try {
    const chain = new Float64Array([1, 0, 0, 1, 0, 0, scale, 0, 0, -scale, w / 2 - scale * cx, h / 2 + scale * cy, dpr, 0.25, Infinity]);
    const [ptr, len] = pass(x, chain);
    const result = check("tessellate_batch", x.tessellate_batch(batch, ptr, len, 1 << 16, 0));
    try {
      const vertices = tessArray(x, result, 0, Float64Array);
      const offsets = tessArray(x, result, 1, Uint32Array);
      ctx.beginPath();
      for (let i = 0; i + 1 < offsets.length; i++) {
        for (let v = offsets[i]; v < offsets[i + 1]; v++) {
          const sx = w / 2 + (vertices[2 * v] - cx) * scale;
          const sy = h / 2 - (vertices[2 * v + 1] - cy) * scale;
          if (v === offsets[i]) ctx.moveTo(sx, sy);
          else ctx.lineTo(sx, sy);
        }
      }
      ctx.lineWidth = 1;
      ctx.lineJoin = "round";
      ctx.lineCap = "round";
      ctx.strokeStyle = token("--stroke");
      ctx.stroke();
    } finally {
      x.release(result);
    }
  } finally {
    x.release(batch);
  }
}

function draw() {
  const dpr = window.devicePixelRatio || 1;
  const [w, h] = size();
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
  }
  const ctx = canvas.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.fillStyle = token("--canvas");
  ctx.fillRect(0, 0, w, h);
  if (state.file < 0) return;
  try {
    drawRasters(ctx, 0);
    drawVectors(ctx);
    drawRasters(ctx, 1);
  } catch (e) {
    setStatus(`${state.name}: ${e.message}`, true);
  }
}

// Draws on the next frame, once however often it is asked; `idle()` resolves when that frame is drawn.
function schedule() {
  if (!state.pending) {
    state.pending = new Promise((resolve) =>
      requestAnimationFrame(() => {
        state.pending = null;
        draw();
        resolve();
      }),
    );
  }
  return state.pending;
}

// The extent to fit: the spatial directory's box when the file has one, else the vertices of every curve at a
// coarse tolerance, joined with the footprints of the rasters.
function extentOf(x, file) {
  const corner = [0, 1, 2, 3].map((i) => x.directory_bound(file, i));
  if (corner.every(Number.isFinite)) return { extent: corner, entities: x.directory_stat(file, 3) };
  const big = 1e300;
  const batch = check("query_viewport", x.query_viewport(file, -big, -big, big, big, 1 << 22));
  const entities = x.batch_len(batch);
  let box = [Infinity, Infinity, -Infinity, -Infinity];
  try {
    const chain = new Float64Array([1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1e9, Infinity]);
    const [ptr, len] = pass(x, chain);
    const result = x.tessellate_batch(batch, ptr, len, 64, 0);
    if (result >= 0) {
      const v = tessArray(x, result, 0, Float64Array);
      for (let i = 0; i < v.length; i += 2) {
        box = [Math.min(box[0], v[i]), Math.min(box[1], v[i + 1]), Math.max(box[2], v[i]), Math.max(box[3], v[i + 1])];
      }
      x.release(result);
    }
  } finally {
    x.release(batch);
  }
  return { extent: box[0] <= box[2] ? box : [0, 0, 1, 1], entities };
}

// The placements of the file, and what its status line says about them: how many, how many skipped and by
// which rule, or the rule that refused the whole stream.
function readRasters(x, file) {
  const out = { rasters: [], text: "0 rasters" };
  if (typeof x.raster_stat !== "function") return out;
  const n = x.raster_stat(file, 0);
  if (n === -8) return out;
  check("raster_stat", n);
  const notes = [];
  if (x.raster_stat(file, 3) === 1 && x.raster_stat(file, 2) === 1 && x.raster_stat(file, 4) > 0) {
    notes.push(`raster stream refused (${rule(x.raster_stat(file, 5), x.raster_stat(file, 4))})`);
  }
  const skipped = Math.max(0, x.raster_stat(file, 1));
  const rules = new Set();
  for (let i = 0; i < skipped; i++) rules.add(rule(x.raster_skipped(file, i, 2), x.raster_skipped(file, i, 1)));
  if (skipped > 0) notes.push(`${skipped} skipped (${[...rules].join(", ")})`);
  for (let r = 0; r < n; r++) {
    const h = x.raster_placement(file, r);
    if (h === -10) {
      out.rasters.push(null);
      continue;
    }
    check("raster_placement", h);
    out.rasters.push({ p: Array.from(rasterArray(x, h, 0, Float64Array)), f: rasterArray(x, h, 1, Uint32Array).slice() });
    x.release(h);
  }
  out.text = `${n} raster${n === 1 ? "" : "s"}${notes.length ? `, ${notes.join(", ")}` : ""}`;
  return out;
}

// The box of a raster's visible pixels in source space.
function footprint({ p, f }) {
  const corners = [
    [f[2], f[3]],
    [f[4], f[3]],
    [f[4], f[5]],
    [f[2], f[5]],
  ].map(([u, v]) => [p[0] * u + p[1] * v + p[4], p[2] * u + p[3] * v + p[5]]);
  const xs = corners.map((c) => c[0]);
  const ys = corners.map((c) => c[1]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

async function openBytes(name, bytes) {
  const x = state.x;
  if (state.file >= 0) x.release(state.file);
  state.file = -1;
  state.name = name;
  const [ptr, len] = pass(x, new Uint8Array(bytes));
  const file = check("open", x.open(ptr, len));
  const { extent, entities } = extentOf(x, file);
  const { rasters, text } = readRasters(x, file);
  for (const r of rasters.filter(Boolean)) {
    const [a, b, c, d] = footprint(r);
    extent[0] = Math.min(extent[0], a);
    extent[1] = Math.min(extent[1], b);
    extent[2] = Math.max(extent[2], c);
    extent[3] = Math.max(extent[3], d);
  }
  for (const bitmap of state.tiles.values()) if (bitmap) bitmap.close();
  state.tiles.clear();
  state.file = file;
  state.extent = extent;
  state.rasters = rasters;
  setStatus(`${name}: ${entities} entit${entities === 1 ? "y" : "ies"}, ${text}`);
  fit();
}

async function openUrl(url) {
  const name = decodeURIComponent(new URL(url, location.href).pathname.split("/").pop() || url);
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`);
  try {
    await openBytes(name, await response.arrayBuffer());
  } catch (e) {
    throw new Error(`${name}: ${e.message}`);
  }
}

// Input: drag to pan, the wheel to zoom about the pointer, double click to zoom in, keys for the same.
function zoomAt(px, py, factor) {
  const [sx, sy] = sourceAt(px, py);
  const [w, h] = size();
  state.view.scale *= factor;
  state.view.cx = sx - (px - w / 2) / state.view.scale;
  state.view.cy = sy + (py - h / 2) / state.view.scale;
  schedule();
}

let drag = null;
canvas.addEventListener("pointerdown", (e) => {
  canvas.setPointerCapture(e.pointerId);
  drag = { x: e.clientX, y: e.clientY };
});
canvas.addEventListener("pointermove", (e) => {
  if (!drag) return;
  state.view.cx -= (e.clientX - drag.x) / state.view.scale;
  state.view.cy += (e.clientY - drag.y) / state.view.scale;
  drag = { x: e.clientX, y: e.clientY };
  schedule();
});
const endDrag = () => {
  drag = null;
};
canvas.addEventListener("pointerup", endDrag);
canvas.addEventListener("pointercancel", endDrag);
canvas.addEventListener(
  "wheel",
  (e) => {
    e.preventDefault();
    const r = canvas.getBoundingClientRect();
    const delta = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    zoomAt(e.clientX - r.left, e.clientY - r.top, Math.exp(-delta * 0.0015));
  },
  { passive: false },
);
canvas.addEventListener("dblclick", (e) => {
  const r = canvas.getBoundingClientRect();
  zoomAt(e.clientX - r.left, e.clientY - r.top, 2);
});
canvas.tabIndex = 0;
canvas.addEventListener("keydown", (e) => {
  const [w, h] = size();
  const step = 64 / state.view.scale;
  const keys = {
    "+": () => zoomAt(w / 2, h / 2, 1.5),
    "=": () => zoomAt(w / 2, h / 2, 1.5),
    "-": () => zoomAt(w / 2, h / 2, 1 / 1.5),
    0: fit,
    ArrowLeft: () => ((state.view.cx -= step), schedule()),
    ArrowRight: () => ((state.view.cx += step), schedule()),
    ArrowUp: () => ((state.view.cy += step), schedule()),
    ArrowDown: () => ((state.view.cy -= step), schedule()),
  };
  if (keys[e.key]) {
    e.preventDefault();
    keys[e.key]();
  }
});
document.getElementById("fit").addEventListener("click", fit);
document.getElementById("pick").addEventListener("change", async (e) => {
  const f = e.target.files[0];
  if (!f) return;
  try {
    await openBytes(f.name, await f.arrayBuffer());
  } catch (err) {
    setStatus(`${f.name}: ${err.message}`, true);
    schedule();
  }
});
new ResizeObserver(() => (state.fitted ? schedule() : fit())).observe(canvas);
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => schedule());

window.pvecViewer = {
  get settled() {
    return state.settled;
  },
  get view() {
    return state.view;
  },
  // resolves once no frame is pending and no tile is being decoded
  idle: async () => {
    while (state.pending || state.decoding.size) {
      await Promise.all([state.pending, ...state.decoding.values()]);
    }
  },
  redraw: schedule,
  toScreen,
  sourceAt,
  fit,
};

(async () => {
  try {
    state.x = await loadModule(params.get("module") || new URL("./pvec_wasm_bg.wasm", import.meta.url).href);
    const file = params.get("file");
    if (file) await openUrl(file);
  } catch (e) {
    setStatus(e.message, true);
  } finally {
    schedule();
    state.settled = true;
  }
})();
