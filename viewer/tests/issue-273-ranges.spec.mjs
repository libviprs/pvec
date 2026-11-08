// Issue #273: the viewer opens a URL by HTTP ranges through the module's -7 loop, sends the validator it captured
// at open as If-Range, and stops with "changed" when the file is replaced under it instead of drawing other bytes.
// Each test serves its own copy of the fixture from tmp-273/, so it can replace it.
import { test, expect } from "@playwright/test";
import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const FIXTURE = here("../../tests/compat/viewer-page.pvec");
const OTHER = here("../../tests/compat/raster-probe.pvec");
const RED = [0xd0, 0x40, 0x30];

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

// `view` starts the page on the circle, away from the raster, so its tiles are still to be fetched.
async function open(page, query, view = "306,200,2") {
  await page.goto(`/viewer/index.html?file=${encodeURIComponent(query)}&view=${view}`);
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

async function pixel(page, x, y) {
  return page.evaluate(([x, y]) => {
    const [sx, sy] = window.pvecViewer.toScreen(x, y);
    const canvas = document.getElementById("view");
    const k = canvas.width / canvas.clientWidth;
    return Array.from(canvas.getContext("2d").getImageData(Math.round(sx * k), Math.round(sy * k), 1, 1).data.slice(0, 3));
  }, [x, y]);
}

const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);

test("#273 a URL is opened by ranges, never fetched whole, and draws what a whole open draws", async ({ page }) => {
  const seen = recordFile(page);
  await open(page, url, "306,396,1");
  await expect(page.locator("#status")).toContainText("4 entities");
  await expect(page.locator("#status")).not.toHaveClass(/error/);
  expect(dist(await pixel(page, 200, 500), RED)).toBeLessThan(12);
  const gets = seen.filter((r) => r.method === "GET");
  expect(gets.length).toBeGreaterThan(0);
  expect(gets.every((r) => /^bytes=\d+-\d+$/.test(r.range || "")), JSON.stringify(seen)).toBe(true);
});

test("#273 every range after the open carries the validator from the open as If-Range", async ({ page }) => {
  const seen = recordFile(page);
  await open(page, url);
  await zoomOn(page, 300, 460, 12);
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
  await zoomOn(page, 300, 460, 12);
  const status = page.locator("#status");
  await expect(status).toHaveClass(/error/);
  await expect(status).toContainText("changed");
});

test("#273 a server that ignores If-Range but sends a new ETag is caught as well", async ({ page }) => {
  await open(page, `${url}?ignore-if-range=1`);
  await new Promise((r) => setTimeout(r, 20));
  await copyFile(OTHER, `${dir}page.pvec`);
  await zoomOn(page, 300, 460, 12);
  const status = page.locator("#status");
  await expect(status).toHaveClass(/error/);
  await expect(status).toContainText("changed");
});

test("#273 a weak ETag is never sent as If-Range, and the view still draws", async ({ page }) => {
  const seen = recordFile(page);
  await open(page, `${url}?weak=1`);
  await zoomOn(page, 300, 460, 12);
  await expect(page.locator("#status")).not.toHaveClass(/error/);
  const gets = seen.filter((r) => r.method === "GET");
  expect(gets.length).toBeGreaterThan(0);
  expect(gets.every((r) => r.ifRange === undefined), JSON.stringify(seen)).toBe(true);
});
