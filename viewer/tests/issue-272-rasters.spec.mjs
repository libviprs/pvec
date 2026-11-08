// Issue #272: the viewer draws the rasters of a .pvec from the module's raster exports, under the vectors by band,
// where pixel_to_source puts them. tests/compat/viewer-page.pvec holds one 512 by 256 RGB raster on 400 by 200
// points from (100, 450): the left half is RED, the right half BLUE, and a GREEN overlay raster on 60 by 60 points
// from (360, 170) (crates/pvec-wasm/tests/issue_271_viewer_fixture.rs).
import { test, expect } from "@playwright/test";

const PAGE = "/viewer/index.html?file=/tests/compat/viewer-page.pvec";
const RED = [0xd0, 0x40, 0x30];
const BLUE = [0x30, 0x60, 0xd0];
const GREEN = [0x30, 0xa0, 0x50];

async function open(page, url = PAGE) {
  await page.goto(url);
  await page.waitForFunction(() => window.pvecViewer && window.pvecViewer.settled === true, null, { timeout: 30_000 });
  await page.evaluate(() => window.pvecViewer.idle());
}

async function tokens(page) {
  return page.evaluate(() => {
    const css = getComputedStyle(document.documentElement);
    const rgb = (name) => {
      const hex = css.getPropertyValue(name).trim().replace("#", "");
      return [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
    };
    return { canvas: rgb("--canvas"), stroke: rgb("--stroke") };
  });
}

async function around(page, x, y, r = 1) {
  return page.evaluate(
    ([x, y, r]) => {
      const [sx, sy] = window.pvecViewer.toScreen(x, y);
      const canvas = document.getElementById("view");
      const k = canvas.width / canvas.clientWidth;
      const d = Math.ceil(r * k);
      const img = canvas.getContext("2d").getImageData(Math.round(sx * k) - d, Math.round(sy * k) - d, 2 * d + 1, 2 * d + 1).data;
      const out = [];
      for (let i = 0; i < img.length; i += 4) out.push([img[i], img[i + 1], img[i + 2]]);
      return out;
    },
    [x, y, r],
  );
}

const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
const all = (pixels, colour, tol = 12) => pixels.every((p) => dist(p, colour) <= tol);

async function zoomOn(page, x, y, factor) {
  await page.evaluate(
    ([x, y, f]) => {
      const v = window.pvecViewer.view;
      v.cx = x;
      v.cy = y;
      v.scale *= f;
      window.pvecViewer.redraw();
    },
    [x, y, factor],
  );
  await page.evaluate(() => window.pvecViewer.idle());
}

test("#272 the raster lands where pixel_to_source puts it, and nothing outside it is painted", async ({ page }) => {
  await open(page);
  const { canvas } = await tokens(page);
  expect(all(await around(page, 200, 500), RED), "left half at (200, 500)").toBe(true);
  expect(all(await around(page, 400, 600), BLUE), "right half at (400, 600)").toBe(true);
  for (const [x, y] of [[90, 500], [510, 500], [200, 440], [400, 660]]) {
    expect(all(await around(page, x, y), canvas, 8), `paper at (${x}, ${y})`).toBe(true);
  }
});

test("#272 an underlay raster is drawn before the vectors, so the line crosses it", async ({ page }) => {
  await open(page);
  const { stroke } = await tokens(page);
  const pixels = await around(page, 200, 550, 3);
  expect(pixels.some((p) => dist(p, RED) <= 12), "the raster around the line at (200, 550)").toBe(true);
  expect(pixels.some((p) => dist(p, RED) >= dist(stroke, RED) / 3), "the line over the raster at (200, 550)").toBe(true);
});

test("#272 an overlay raster is drawn after the vectors, so it covers the circle", async ({ page }) => {
  await open(page);
  expect(all(await around(page, 386, 200, 2), GREEN), "the overlay over the circle's east at (386, 200)").toBe(true);
  expect(all(await around(page, 390, 225), GREEN), "the overlay's top right").toBe(true);
});

test("#272 zoomed in on the seam, the finer level still lands in place", async ({ page }) => {
  await open(page);
  await zoomOn(page, 300, 460, 16);
  const { canvas } = await tokens(page);
  expect(all(await around(page, 299, 470), RED), "just left of the seam").toBe(true);
  expect(all(await around(page, 301, 470), BLUE), "just right of the seam").toBe(true);
  expect(all(await around(page, 290, 449), canvas, 8), "just below the raster").toBe(true);
  expect(all(await around(page, 290, 451), RED), "just inside the bottom edge, left").toBe(true);
  expect(all(await around(page, 310, 451), BLUE), "just inside the bottom edge, right").toBe(true);
});

test("#272 the status line names the rasters, and a skipped one with its rule", async ({ page }) => {
  await open(page);
  await expect(page.locator("#status")).toContainText("2 rasters");
  await open(page, "/viewer/index.html?file=/tests/compat/raster-probe.pvec");
  const status = page.locator("#status");
  await expect(status).toContainText("2 rasters");
  await expect(status).toContainText("1 skipped");
  await expect(status).toContainText("RASTER-04");
  await expect(status).not.toHaveClass(/error/);
});

test("#272 the dark theme leaves the raster's colours alone", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await open(page);
  expect(all(await around(page, 200, 500), RED)).toBe(true);
  expect(all(await around(page, 400, 600), BLUE)).toBe(true);
});
