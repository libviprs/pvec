// The viewer's browser tests (ADR 0020). tools/viewer-test.sh runs them in the Playwright container on a docker
// host, after it has built the wasm module into viewer/. One worker: the page is small and the host is shared.
import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.VIEWER_PORT || 8754);

export default defineConfig({
  testDir: ".",
  testMatch: /issue-\d+-.*\.spec\.mjs$/,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: { baseURL: `http://127.0.0.1:${port}`, viewport: { width: 900, height: 700 } },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 900, height: 700 } } }],
  webServer: {
    command: `node serve.mjs ../.. ${port}`,
    url: `http://127.0.0.1:${port}/viewer/index.html`,
    reuseExistingServer: false,
  },
});
