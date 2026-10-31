// Probes the raster exports of a staged pvec-wasm module on the raster fixture (issue #221).
//   node tools/wasm-raster-probe.mjs <pvec_wasm_bg.wasm> <raster-probe.pvec>
// It instantiates the module with stub imports (the loadable module imports nothing it calls), hands it the
// fixture as whole bytes, and checks what tests/compat/raster-probe.pvec holds: two rasters, one skipped by an
// unknown tile codec; the placement of the RASTER.md test vector; the plan of its footprint; one tile that is
// the PNG the tiler wrote. It exits 1 on a module that lacks an export or answers differently, and prints why.
import { readFile } from "node:fs/promises";

const [wasmPath, fixturePath] = process.argv.slice(2);
if (!wasmPath || !fixturePath) {
  console.error("usage: wasm-raster-probe.mjs <module.wasm> <fixture.pvec>");
  process.exit(2);
}
const fail = (why) => {
  console.error(`raster probe FAILED: ${why}`);
  process.exit(1);
};

const module = await WebAssembly.compile(await readFile(wasmPath));
const imports = {};
for (const { module: m, name, kind } of WebAssembly.Module.imports(module)) {
  imports[m] ??= {};
  imports[m][name] = kind === "function" ? () => 0 : fail(`the module imports a ${kind} (${m}.${name})`);
}
const { exports: x } = await WebAssembly.instantiate(module, imports);
if (x.__wbindgen_start) x.__wbindgen_start();

const NEED = [
  "wasm_abi_version", "open", "release",
  "raster_stat", "raster_skipped", "raster_placement", "raster_plan", "raster_tile",
  "raster_buffer_len", "raster_buffer_ptr", "raster_buffer_elem",
];
const missing = NEED.filter((n) => typeof x[n] !== "function");
if (missing.length) fail(`the module does not export ${missing.join(", ")}`);
if (x.wasm_abi_version() !== 2) fail(`wasm_abi_version is ${x.wasm_abi_version()}, not 2`);

const bytes = new Uint8Array(await readFile(fixturePath));
const ptr = x.__wbindgen_malloc(bytes.length, 1) >>> 0;
new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
const file = x.open(ptr, bytes.length);
if (file < 0) fail(`open returned ${file}`);

const want = (what, got, expect) => {
  if (JSON.stringify(got) !== JSON.stringify(expect)) fail(`${what}: ${JSON.stringify(got)}, expected ${JSON.stringify(expect)}`);
};
want("raster_stat 0 (rasters)", x.raster_stat(file, 0), 2);
want("raster_stat 1 (skipped)", x.raster_stat(file, 1), 1);
want("raster_stat 2 (unrastered)", x.raster_stat(file, 2), 1);
want("raster_stat 3 (has a stream)", x.raster_stat(file, 3), 1);
want("raster_skipped (id, rule)", [x.raster_skipped(file, 0, 0), x.raster_skipped(file, 0, 1)], [1, 4]);

const array = (h, n, Type, elem) => {
  const len = x.raster_buffer_len(h, n), at = x.raster_buffer_ptr(h, n) >>> 0;
  if (len < 0) fail(`raster_buffer_len(${h}, ${n}) = ${len}`);
  if (x.raster_buffer_elem(h, n) !== elem) fail(`raster_buffer_elem(${h}, ${n}) = ${x.raster_buffer_elem(h, n)}, expected ${elem}`);
  return Array.from(new Type(x.memory.buffer, at, len));
};
const placement = x.raster_placement(file, 0);
if (placement < 0) fail(`raster_placement = ${placement}`);
want("pixel_to_source", array(placement, 0, Float64Array, 0), [0.5, 0.25, 0.125, -0.5, 100, 50]);
want("width and height", array(placement, 1, Uint32Array, 2).slice(0, 2), [512, 256]);
x.release(placement);

const plan = x.raster_plan(file, 100, -206, 420, 0, 1, 0);
if (plan < 0) fail(`raster_plan = ${plan}`);
want("planned raster ids", array(plan, 0, Uint32Array, 2), [0]);
const tiles = array(plan, 3, Uint32Array, 2).length / 2;
if (tiles < 1) fail("the plan holds no tile");
x.release(plan);

const level = array(x.raster_plan(file, 100, -206, 420, 0, 1, 0), 1, Uint32Array, 2)[0];
const tile = x.raster_tile(file, 0, level, 0, 0);
if (tile < 0) fail(`raster_tile = ${tile}`);
const png = array(tile, 0, Uint8Array, 3);
want("the tile is a PNG", png.slice(0, 8), [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
x.release(tile);
want("a skipped raster's tile", x.raster_tile(file, 1, 0, 0, 0), -10);
want("a tile outside the grid", x.raster_tile(file, 0, level, 99, 99), -3);
console.log(`raster probe ok: ${wasmPath} (abi ${x.wasm_abi_version()}, ${bytes.length} byte fixture, ${tiles} tile(s) planned at level ${level})`);
