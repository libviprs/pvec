// Issue #273: the viewer opens a URL by HTTP ranges through the module's -7 loop, sends the validator it captured
// at open as If-Range, and stops with "changed" when the file is replaced under it instead of drawing other bytes.
// Each test serves its own copy of tests/compat/raster-probe.pvec from tmp-273/, so it can replace it. That file is
// 51 KiB with its full resolution tiles in chunks after the first 4 KiB, so a view away from the raster has not
// fetched them yet; its one shown raster covers about x 100 to 388, y -78 to 178 (issue_221_raster_exports.rs).
import { test, expect } from "@playwright/test";
import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const FIXTURE = here("../../tests/compat/raster-probe.pvec");
const OTHER = here("../../tests/compat/viewer-page.pvec");
const AWAY = "-600,600,1";
const ON = [244, 50, 6];

let dir, url;
test.beforeEach(async ({}, info) => {
  dir = here(`./tmp-273/${info.testId}/`);
  await mkdir(dir, { recursive: true });
  await copyFile(FIXTURE, `${dir}page.pvec`);
  url = `/viewer/tests/tmp-273/${info.testId}/page.pvec`;
});
test.afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

// Every request the page makes for the file, with the headers that matter here.
function recordFile(page) {
  const seen = [];
  page.on("request", async (req) => {
    if (!req.url().includes("/tmp-273/")) return;
    const h = await req.allHeaders();
    seen.push({ method: req.method(), range: h.range, ifRange: h["if-range"] });
  });
  return seen;
}

// `view` starts the page away from the raster by default, so its tiles are still to be fetched.
async function open(page, query, view = AWAY, extra = "") {
  await page.goto(`/viewer/index.html?file=${encodeURIComponent(query)}&view=${view}${extra}`);
  await page.waitForFunction(() => window.pvecViewer && window.pvecViewer.settled === true, null, { timeout: 30_000 });
  await page.evaluate(() => window.pvecViewer.idle());
}

async function zoomOn(page, x, y, scale) {
  await page.evaluate(
    ([x, y, s]) => {
      const v = window.pvecViewer.view;
      v.cx = x;
      v.cy = y;
      v.scale = s;
      window.pvecViewer.redraw();
    },
    [x, y, scale],
  );
  await page.evaluate(() => window.pvecViewer.idle());
}

// A digest of the whole canvas (FNV-1a over its bytes), so two frames compare without shipping the pixels.
async function pixels(page) {
  return page.evaluate(() => {
    const canvas = document.getElementById("view");
    const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
    let h = 0x811c9dc5;
    for (let i = 0; i < data.length; i++) h = Math.imul(h ^ data[i], 0x01000193) >>> 0;
    return h;
  });
}

test("#273 a URL is opened by ranges, never fetched whole, and draws what a whole open draws", async ({ page }) => {
  const seen = recordFile(page);
  const view = ON.join(",");
  await open(page, url, view);
  const status = page.locator("#status");
  await expect(status).toContainText("1 entity, 2 rasters, 1 skipped (RASTER-04), read by ranges");
  await expect(status).not.toHaveClass(/error/);
  const ranged = await pixels(page);
  const gets = seen.filter((r) => r.method === "GET");
  expect(gets.length).toBeGreaterThan(1);
  expect(gets.every((r) => /^bytes=\d+-\d+$/.test(r.range || "")), JSON.stringify(seen)).toBe(true);
  await open(page, url, view, "&ranges=0");
  await expect(status).not.toContainText("read by ranges");
  expect(await pixels(page)).toEqual(ranged);
});

test("#273 every range after the open carries the validator from the open as If-Range", async ({ page }) => {
  const seen = recordFile(page);
  await open(page, url);
  await zoomOn(page, ...ON);
  const gets = seen.filter((r) => r.method === "GET");
  expect(gets.length).toBeGreaterThan(1);
  const first = gets[0].ifRange;
  expect(first, JSON.stringify(seen)).toMatch(/^"[0-9a-f]+-[0-9a-f]+"$/);
  expect(gets.every((r) => r.ifRange === first), JSON.stringify(seen)).toBe(true);
});

test("#273 a file replaced at the same length stops the view with 'changed' and draws none of it", async ({ page }) => {
  await open(page, url);
  const bytes = await readFile(`${dir}page.pvec`);
  // the shape of a re-calibration that replaces the last block: same length, other bytes, a new ETag
  bytes[bytes.length - 100] ^= 0xff;
  await new Promise((r) => setTimeout(r, 20));
  await writeFile(`${dir}page.pvec`, bytes);
  await zoomOn(page, ...ON);
  const status = page.locator("#status");
  await expect(status).toHaveClass(/error/);
  await expect(status).toContainText("changed");
});

test("#273 a server that ignores If-Range but sends a new ETag is caught as well", async ({ page }) => {
  await open(page, `${url}?ignore-if-range=1`);
  await new Promise((r) => setTimeout(r, 20));
  await copyFile(OTHER, `${dir}page.pvec`);
  await zoomOn(page, ...ON);
  const status = page.locator("#status");
  await expect(status).toHaveClass(/error/);
  await expect(status).toContainText("changed");
});

test("#273 a weak ETag is never sent as If-Range, and the view still draws", async ({ page }) => {
  const seen = recordFile(page);
  await open(page, `${url}?weak=1`);
  await zoomOn(page, ...ON);
  await expect(page.locator("#status")).not.toHaveClass(/error/);
  const gets = seen.filter((r) => r.method === "GET");
  expect(gets.length).toBeGreaterThan(0);
  expect(gets.every((r) => r.ifRange === undefined), JSON.stringify(seen)).toBe(true);
});
