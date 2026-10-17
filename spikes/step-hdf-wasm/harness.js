// Loads a wasm build of the pure-Rust HDF5 reader, hands it a file image, and times the decode.
// usage: node harness.js <module.wasm> <layout:h1|h2> <file.h5>...
const fs = require('fs');

(async () => {
  const [wasmPath, layout, ...files] = process.argv.slice(2);
  const bytes = fs.readFileSync(wasmPath);
  const cpu0 = process.cpuUsage();
  const t0 = process.hrtime.bigint();
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const instMs = Number(process.hrtime.bigint() - t0) / 1e6;
  const instCpuMs = (process.cpuUsage(cpu0).user + process.cpuUsage(cpu0).system) / 1e3;
  console.log(`module ${wasmPath}: ${bytes.length} bytes, instantiate ${instMs.toFixed(2)} ms wall, ${instCpuMs.toFixed(2)} ms cpu`);
  const ex = instance.exports;
  const fn = layout === 'h2' ? ex.h2_decode : ex.h1_decode;
  for (const path of files) {
    const buf = fs.readFileSync(path);
    const ptr = ex.alloc(buf.length);
    new Uint8Array(ex.memory.buffer, ptr, buf.length).set(buf);
    const wall = [], cpu = [];
    let h = 0n;
    for (let i = 0; i < 5; i++) {
      const c0 = process.cpuUsage();
      const s = process.hrtime.bigint();
      h = fn(ptr, buf.length);
      wall.push(Number(process.hrtime.bigint() - s) / 1e6);
      const c = process.cpuUsage(c0);
      cpu.push((c.user + c.system) / 1e3);
    }
    wall.sort((a, b) => a - b);
    cpu.sort((a, b) => a - b);
    const mw = wall[2], mc = cpu[2];
    const mb = buf.length / 1048576;
    console.log(`| ${path.split('/').pop()} | ${buf.length} bytes | hash ${h.toString(16).padStart(16, '0')} | wall ${mw.toFixed(1)} ms (${(mb / (mw / 1000)).toFixed(0)} MiB/s) | cpu ${mc.toFixed(1)} ms | wasm memory ${(ex.memory.buffer.byteLength / 1048576).toFixed(0)} MiB |`);
  }
})();
