// Instantiates the stand-in under Node, writes an artifact into wasm memory and decodes the whole drawing,
// so the size that is reported belongs to a module that works. Usage: node run.mjs module.wasm file.pvec x0 y0 x1 y1
import fs from "node:fs";
const [, , wasmPath, filePath, ...w] = process.argv;
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
const { memory, pvec_alloc, pvec_decode } = instance.exports;
const bytes = fs.readFileSync(filePath);
const ptr = pvec_alloc(bytes.length);
new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
const t = process.hrtime.bigint();
const n = pvec_decode(ptr, bytes.length, ...w.map(Number));
console.log(`decoded=${n} ms=${Number(process.hrtime.bigint() - t) / 1e6}`);
