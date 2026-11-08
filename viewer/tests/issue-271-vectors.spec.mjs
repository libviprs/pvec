// Issue #271: the viewer opens a .pvec whole and draws its vectors in source space, y up, with pan and zoom.
// The file is tests/compat/viewer-page.pvec (crates/pvec-wasm/tests/issue_271_viewer_fixture.rs says what is in it).
import { test, expect } from "@playwright/test";

const PAGE = "/viewer/index.html?file=/tests/compat/viewer-page.pvec";

async function open(page, url = PAGE) {
  await page.goto(url);
  await page.waitForFunction(() => window.pvecViewer && window.pvecViewer.settled === true, null, { timeout: 30_000 });
  await page.evaluate(() => window.pvecViewer.idle());
}

// The colours the page draws with, from its own tokens, as [r, g, b].
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

// The pixels of the canvas within `r` CSS pixels of the screen position of source point (x, y).
async function around(page, x, y, r = 2) {
  return page.evaluate(
    ([x, y, r]) => {
      const v = window.pvecViewer;
      const [sx, sy] = v.toScreen(x, y);
      const canvas = document.getElementById("view");
      const k = canvas.width / canvas.clientWidth;
      const ctx = canvas.getContext("2d");
      const d = Math.ceil(r * k);
      const img = ctx.getImageData(Math.round(sx * k) - d, Math.round(sy * k) - d, 2 * d + 1, 2 * d + 1).data;
      const out = [];
      for (let i = 0; i < img.length; i += 4) out.push([img[i], img[i + 1], img[i + 2]]);
      return out;
    },
    [x, y, r],
  );
}

const close = (a, b, tol = 40) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]) <= tol;
const has = (pixels, colour) => pixels.some((p) => close(p, colour));
const all = (pixels, colour) => pixels.every((p) => close(p, colour, 8));

test("#271 opens the page fixture and says what it holds", async ({ page }) => {
  await open(page);
  const status = page.locator("#status");
  await expect(status).toContainText("viewer-page.pvec");
  await expect(status).toContainText("4 entities");
  await expect(status).toContainText("1 raster");
  await expect(status).not.toHaveClass(/error/);
});

test("#271 draws the frame, the corner mark, the line and the circle where the source puts them", async ({ page }) => {
  await open(page);
  const { canvas, stroke } = await tokens(page);
  for (const [x, y, what] of [
    [306, 0, "bottom edge of the frame"],
    [0, 396, "left edge of the frame"],
    [612, 600, "right edge of the frame"],
    [306, 792, "top edge of the frame"],
    [50, 772, "top of the corner mark"],
    [80, 740, "side of the corner mark"],
    [50, 550, "the line, left of the raster"],
    [386, 200, "the circle, east"],
    [306, 120, "the circle, south"],
  ]) {
    expect(has(await around(page, x, y), stroke), `${what} at (${x}, ${y})`).toBe(true);
  }
  for (const [x, y, what] of [
    [306, 200, "the centre of the circle"],
    [50, 300, "empty paper"],
    [560, 740, "the top right, where there is no mark"],
  ]) {
    expect(all(await around(page, x, y, 1), canvas), `${what} at (${x}, ${y})`).toBe(true);
  }
});

test("#271 source y is up: the top of the page is at the top of the screen", async ({ page }) => {
  await open(page);
  const [top, bottom] = await page.evaluate(() => [window.pvecViewer.toScreen(0, 792), window.pvecViewer.toScreen(0, 0)]);
  expect(top[1]).toBeLessThan(bottom[1]);
  const box = await page.locator("#view").boundingBox();
  // fitted: the whole page is on screen
  for (const [sx, sy] of [top, bottom]) {
    expect(sx).toBeGreaterThanOrEqual(0);
    expect(sy).toBeGreaterThanOrEqual(0);
    expect(sx).toBeLessThanOrEqual(box.width);
    expect(sy).toBeLessThanOrEqual(box.height);
  }
});

test("#271 zooming keeps the source point under the pointer, and dragging pans by the pointer's move", async ({ page }) => {
  await open(page);
  const box = await page.locator("#view").boundingBox();
  const at = [box.x + 300, box.y + 250];
  const before = await page.evaluate(([x, y]) => window.pvecViewer.sourceAt(x, y), [300, 250]);
  const scale0 = await page.evaluate(() => window.pvecViewer.view.scale);
  await page.mouse.move(at[0], at[1]);
  await page.mouse.wheel(0, -400);
  await page.evaluate(() => window.pvecViewer.idle());
  const scale1 = await page.evaluate(() => window.pvecViewer.view.scale);
  expect(scale1).toBeGreaterThan(scale0);
  const after = await page.evaluate(([x, y]) => window.pvecViewer.sourceAt(x, y), [300, 250]);
  expect(Math.abs(after[0] - before[0])).toBeLessThan(1e-6 * 612);
  expect(Math.abs(after[1] - before[1])).toBeLessThan(1e-6 * 792);

  await page.mouse.down();
  await page.mouse.move(at[0] + 100, at[1] + 40, { steps: 5 });
  await page.mouse.up();
  await page.evaluate(() => window.pvecViewer.idle());
  const moved = await page.evaluate(([x, y]) => window.pvecViewer.sourceAt(x, y), [400, 290]);
  expect(Math.abs(moved[0] - before[0])).toBeLessThan(1e-6 * 612);
  expect(Math.abs(moved[1] - before[1])).toBeLessThan(1e-6 * 792);
});

test("#271 Fit brings the whole page back after a zoom", async ({ page }) => {
  await open(page);
  const fitted = await page.evaluate(() => ({ ...window.pvecViewer.view }));
  await page.mouse.move(400, 400);
  await page.mouse.wheel(0, -800);
  await page.getByRole("button", { name: "Fit" }).click();
  await page.evaluate(() => window.pvecViewer.idle());
  const again = await page.evaluate(() => ({ ...window.pvecViewer.view }));
  expect(again.scale).toBeCloseTo(fitted.scale, 9);
  expect(again.cx).toBeCloseTo(fitted.cx, 6);
  expect(again.cy).toBeCloseTo(fitted.cy, 6);
});

test("#271 a file that is not a PVec is refused with the module's code, and nothing is drawn", async ({ page }) => {
  await open(page, "/viewer/index.html?file=/viewer/index.html");
  const status = page.locator("#status");
  await expect(status).toHaveClass(/error/);
  await expect(status).toContainText("-4");
});

test("#271 the dark theme draws light strokes on a dark canvas", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await open(page);
  const { canvas, stroke } = await tokens(page);
  expect(canvas[0] + canvas[1] + canvas[2]).toBeLessThan(150);
  expect(has(await around(page, 306, 0), stroke)).toBe(true);
  expect(all(await around(page, 50, 300, 1), canvas)).toBe(true);
});
