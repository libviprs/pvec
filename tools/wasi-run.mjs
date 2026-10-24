// Runs a wasm32-wasip1 test binary under node's WASI, for the codec measurement (#110).
//   node tools/wasi-run.mjs <file.wasm> [args for the test binary...]
// The preopen is the repository root at the same absolute path it was compiled at, because the vendored
// corpus is read from CARGO_MANIFEST_DIR at run time.
import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";

const [file, ...args] = process.argv.slice(2);
const root = process.env.PVEC_ROOT ?? "/work";
const wasi = new WASI({
  version: "preview1",
  args: [file, ...args],
  env: {},
  preopens: { [root]: root },
});
const { instance } = await WebAssembly.instantiate(await readFile(file), wasi.getImportObject());
process.exitCode = wasi.start(instance);
