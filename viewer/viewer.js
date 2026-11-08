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

const state = {
  x: null, // the module's exports
  file: -1, // the open file handle
  name: "",
  extent: null, // [min_x, min_y, max_x, max_y] in source units
  view: { cx: 0, cy: 0, scale: 1 }, // CSS pixels per source unit, and the source point at the centre
  settled: false,
  pending: null,
  fitted: false,
};

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
    drawVectors(ctx);
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

function rasterCount(x, file) {
  if (typeof x.raster_stat !== "function") return 0;
  const n = x.raster_stat(file, 0);
  return n < 0 ? 0 : n;
}

async function openBytes(name, bytes) {
  const x = state.x;
  if (state.file >= 0) x.release(state.file);
  state.file = -1;
  state.name = name;
  const [ptr, len] = pass(x, new Uint8Array(bytes));
  const file = check("open", x.open(ptr, len));
  const { extent, entities } = extentOf(x, file);
  const rasters = rasterCount(x, file);
  state.file = file;
  state.extent = extent;
  setStatus(`${name}: ${entities} entit${entities === 1 ? "y" : "ies"}, ${rasters} raster${rasters === 1 ? "" : "s"}`);
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
  idle: () => state.pending || Promise.resolve(),
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
