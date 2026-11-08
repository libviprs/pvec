// Issue #274: the viewer shows the calibration block (ADR 0019) when a file has one: the scale with its provenance
// and uncertainty next to the manifest unit, the CRS id as it is, a scale bar at the effective scale, and the pointer
// in source units and in CRS units through the stored affine. Nothing is reprojected. tests/compat/viewer-calibrated.pvec
// is a 1:100 plan scale on a sheet in points (1 unit is 0.0254 / 72 * 100 m) with a declared georeference into
// EPSG:27700 whose affine is that scale turned by the 3-4-5 angle from (530000, 180000), and
// viewer-calibration-refused.pvec the same with the block damaged (crates/pvec-wasm/tests/issue_274_calibrated_fixture.rs).
import { test, expect } from "@playwright/test";

const SCALE = (0.0254 / 72) * 100;
const AFFINE = [SCALE * 0.8, -SCALE * 0.6, SCALE * 0.6, SCALE * 0.8, 530000, 180000];

async function open(page, file) {
  await page.goto(`/viewer/index.html?file=${file}`);
  await page.waitForFunction(() => window.pvecViewer && window.pvecViewer.settled === true, null, { timeout: 30_000 });
  await page.evaluate(() => window.pvecViewer.idle());
}

// The scale bar's length in metres, from its label.
function metres(label) {
  const m = /^([\d.]+) (km|m|mm)/.exec(label);
  expect(m, label).not.toBeNull();
  return Number(m[1]) * { km: 1000, m: 1, mm: 0.001 }[m[2]];
}

test("#274 an uncalibrated file says so, and its scale bar is in manifest units", async ({ page }) => {
  await open(page, "/tests/compat/viewer-page.pvec");
  const cal = page.locator("#calibration");
  await expect(cal).toContainText("Uncalibrated");
  await expect(cal).toContainText("Manifest: 1 unit is 1 m");
  await expect(page.locator("#scalebar .label")).toContainText("manifest unit");
});

test("#274 a calibrated file shows its plan scale with provenance and uncertainty, the manifest unit and the CRS id", async ({ page }) => {
  await open(page, "/tests/compat/viewer-calibrated.pvec");
  const cal = page.locator("#calibration");
  await expect(cal).toContainText("Scale 1:100 (plan scale): 1 unit is 35.28 mm, ±1%");
  await expect(cal).toContainText("Manifest: 1 unit is 1 m");
  await expect(cal).toContainText("Georeference: EPSG:27700 (projected, declared)");
  await expect(page.locator("#status")).not.toHaveClass(/error/);
});

test("#274 the pointer reads in source units and in CRS units through the stored affine", async ({ page }) => {
  await open(page, "/tests/compat/viewer-calibrated.pvec");
  const box = await page.locator("#view").boundingBox();
  const at = [Math.round(box.x + 300), Math.round(box.y + 200)];
  await page.mouse.move(at[0], at[1]);
  const [sx, sy] = await page.evaluate(([x, y]) => window.pvecViewer.sourceAt(x, y), [at[0] - box.x, at[1] - box.y]);
  const pointer = page.locator("#pointer");
  await expect(pointer).toContainText(`x ${sx.toFixed(2)}, y ${sy.toFixed(2)}`);
  const [a, b, c, d, tx, ty] = AFFINE;
  await expect(pointer).toContainText(`E ${(a * sx + b * sy + tx).toFixed(2)}, N ${(c * sx + d * sy + ty).toFixed(2)}`);
  await page.mouse.move(box.x + box.width + 50, box.y + 10);
  await expect(pointer).toHaveText("");
});

test("#274 the scale bar is as long on screen as it says, at the calibration's scale", async ({ page }) => {
  await open(page, "/tests/compat/viewer-calibrated.pvec");
  const label = await page.locator("#scalebar .label").textContent();
  expect(label).not.toContain("manifest unit");
  const width = (await page.locator("#scalebar .bar").boundingBox()).width;
  const scale = await page.evaluate(() => window.pvecViewer.view.scale);
  // CSS pixels times source units per pixel times metres per source unit
  expect(Math.abs((width / scale) * SCALE - metres(label)) / metres(label)).toBeLessThan(0.01);
});

test("#274 a refused calibration block shows its rule, and the page still draws in manifest units", async ({ page }) => {
  await open(page, "/tests/compat/viewer-calibration-refused.pvec");
  const cal = page.locator("#calibration");
  await expect(cal).toContainText("Calibration refused (CAL-01).");
  await expect(cal).toContainText("Manifest: 1 unit is 1 m");
  await expect(cal).not.toContainText("Georeference");
  await expect(page.locator("#status")).toContainText("4 entities");
  await expect(page.locator("#status")).not.toHaveClass(/error/);
  await expect(page.locator("#scalebar .label")).toContainText("manifest unit");
});

for (const scheme of ["light", "dark"]) {
  test(`#274 the readout, the pointer line and the scale bar keep their contrast in the ${scheme} theme`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await open(page, "/tests/compat/viewer-calibrated.pvec");
    const box = await page.locator("#view").boundingBox();
    await page.mouse.move(box.x + 200, box.y + 200);
    const pairs = await page.evaluate(() => {
      const bg = (el) => {
        for (; el; el = el.parentElement) {
          const c = getComputedStyle(el).backgroundColor;
          if (!c.endsWith(", 0)") && c !== "transparent") return c;
        }
        return "rgb(255, 255, 255)";
      };
      return ["calibration", "pointer"].map((id) => document.getElementById(id))
        .concat([document.querySelector("#scalebar .label")])
        .map((el) => [el.id || el.className, getComputedStyle(el).color, bg(el)]);
    });
    const lum = (c) => {
      const [r, g, b] = c.match(/[\d.]+/g).slice(0, 3).map((v) => {
        const s = Number(v) / 255;
        return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    for (const [what, fg, bg] of pairs) {
      const [hi, lo] = [lum(fg), lum(bg)].sort((x, y) => y - x);
      expect((hi + 0.05) / (lo + 0.05), `${what}: ${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
    }
  });
}
