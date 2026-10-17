// Runs one candidate's wasm touch over one encoded file and prints timings.
// usage: node bench.mjs <wasm> <export> <file> <entities> [reps]
import { readFileSync } from "node:fs";
const [wasmPath, exp, filePath, entities, repsArg] = process.argv.slice(2);
const reps = Number(repsArg ?? 7);
const wasm = readFileSync(wasmPath);
const file = readFileSync(filePath);
const t0 = performance.now();
const mod = await WebAssembly.compile(wasm);
const t1 = performance.now();
const inst = await WebAssembly.instantiate(mod, {});
const t2 = performance.now();
const { alloc, memory } = inst.exports;
const fn = inst.exports[exp];
const ptr = alloc(file.length);
new Uint8Array(memory.buffer, ptr, file.length).set(file);
let result = fn(ptr, file.length);
const times = [];
for (let i = 0; i < reps; i++) {
  const a = performance.now();
  result = fn(ptr, file.length);
  times.push(performance.now() - a);
}
times.sort((x, y) => x - y);
const med = times[Math.floor(times.length / 2)];
const n = Number(entities);
console.log(
  `${exp} ${filePath.split("/").pop()} wasm_bytes=${wasm.length} compile_ms=${(t1 - t0).toFixed(1)} instantiate_ms=${(t2 - t1).toFixed(1)} ` +
    `median_ms=${med.toFixed(2)} ns_per_entity=${((med * 1e6) / n).toFixed(1)} MB_per_s=${(file.length / 1e6 / (med / 1e3)).toFixed(0)} ` +
    `memory_MB=${(memory.buffer.byteLength / 1048576).toFixed(0)} result=${BigInt.asUintN(64, result)}`
);
