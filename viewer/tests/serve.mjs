// A static file server for the viewer's browser tests: node serve.mjs <root> <port>. Test-only, no dependency.
//
// It answers HEAD and GET, serves byte ranges (one range per request) with a strong ETag made from the file's size
// and modification time, and honours If-Range: a validator that no longer matches gets the whole file with 200,
// as RFC 9110 says. Every answer also carries Last-Modified. Two query parameters change that for the specs of issue
// #273: `ignore-if-range=1` serves the range whatever If-Range says, and `weak=1` sends the ETag as a weak one.
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { extname, join, normalize, resolve } from "node:path";

const [rootArg = ".", portArg = "8754"] = process.argv.slice(2);
const root = resolve(rootArg);
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".pvec": "application/octet-stream",
};

createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  const file = join(root, normalize(decodeURIComponent(url.pathname)));
  if (!file.startsWith(root)) {
    res.writeHead(403).end();
    return;
  }
  let body, info;
  try {
    [body, info] = await Promise.all([readFile(file), stat(file)]);
  } catch {
    res.writeHead(404).end();
    return;
  }
  const strong = `"${info.size.toString(16)}-${Math.round(info.mtimeMs).toString(16)}"`;
  const etag = url.searchParams.get("weak") === "1" ? `W/${strong}` : strong;
  const headers = {
    "content-type": types[extname(file)] || "application/octet-stream",
    "cache-control": "no-store",
    "accept-ranges": "bytes",
    etag,
    "last-modified": info.mtime.toUTCString(),
  };
  const range = /^bytes=(\d+)-(\d+)$/.exec(req.headers.range || "");
  const ifRange = req.headers["if-range"];
  const stale = ifRange !== undefined && ifRange !== strong && url.searchParams.get("ignore-if-range") !== "1";
  if (req.method === "HEAD") {
    res.writeHead(200, { ...headers, "content-length": body.length }).end();
  } else if (range && !stale) {
    const [first, last] = [Number(range[1]), Math.min(Number(range[2]), body.length - 1)];
    if (first > last) {
      res.writeHead(416, { ...headers, "content-range": `bytes */${body.length}` }).end();
      return;
    }
    const part = body.subarray(first, last + 1);
    res.writeHead(206, { ...headers, "content-range": `bytes ${first}-${last}/${body.length}`, "content-length": part.length });
    res.end(part);
  } else {
    res.writeHead(200, { ...headers, "content-length": body.length });
    res.end(body);
  }
}).listen(Number(portArg), "127.0.0.1");
