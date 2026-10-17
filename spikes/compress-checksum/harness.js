// usage: node harness.js <module.wasm> <dir> <fixture>
// Decodes every block of each packed buffer in wasm and checksums the raw blocks, reporting CPU time.
const fs = require('fs');
(async () => {
  const [wasmPath, dir, fixture] = process.argv.slice(2);
  const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
  const ex = instance.exports;
  const put = (buf) => { const p = ex.alloc(buf.length); new Uint8Array(ex.memory.buffer, p, buf.length).set(buf); return p; };
  const lens = fs.readFileSync(`${dir}/${fixture}.lens`);
  const count = lens.length / 4;
  const lp = put(lens);
  const raw = fs.readFileSync(`${dir}/${fixture}.raw`);
  const mb = raw.length / 1e6;
  const time = (f) => { const t = []; let r; for (let i = 0; i < 5; i++) { const c = process.cpuUsage(); r = f(); const u = process.cpuUsage(c); t.push((u.user + u.system) / 1e6); } t.sort((a, b) => a - b); return [t[2], r]; };
  console.log(`## ${fixture}: ${count} blocks, ${raw.length} raw bytes (wasm32, node ${process.version})`);
  for (const [name, codec, shuf] of [['deflate1', 0, 0], ['deflate1-shuf', 0, 1], ['lz4', 1, 0], ['lz4-shuf', 1, 1]]) {
    const buf = fs.readFileSync(`${dir}/${fixture}.${name}`);
    const p = put(buf);
    const [s, r] = time(() => ex.decode_blocks(codec, shuf, p, buf.length, lp, count));
    console.log(`| ${name} | ${buf.length} bytes | decompress ${(mb / s).toFixed(0)} MB/s | result ${r === 0xffffffffffffffffn ? 'FAILED' : 'ok'} |`);
  }
  const rp = put(raw);
  ['crc32 (crc32fast)', 'crc32c', 'xxh3-64', 'fnv-1a 64', 'sha-256 (first 8 bytes)'].forEach((k, i) => {
    const [s, r] = time(() => ex.checksum_blocks(i, rp, raw.length, lp, count));
    console.log(`| ${k} | ${(mb / s).toFixed(0)} MB/s | ${r.toString(16)} |`);
  });
})();
