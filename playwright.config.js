import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright config for kraftwerk's E2E tests.
 *
 * Tier 1 (smoke): runs against the vanilla `vite dev` server. Tauri's
 * `invoke()` is shimmed in-page so command calls return plausible
 * mock data without a real hypervisor.
 *
 * The same setup will host Tier 2 (invoke contract) — the mock just
 * gets richer there.
 *
 * Tier 3 (real Tauri webview) uses tauri-driver and lives in a
 * different config (playwright.tauri.config.js) so it can be opt-in.
 */
export default defineConfig({
  testDir: "./e2e",
  testMatch: /.*\.spec\.js$/,
  fullyParallel: false,           // one webview, shared state
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI ? "github" : "list",

  use: {
    baseURL: "http://localhost:1420",
    trace: process.env.CI ? "on-first-retry" : "off",
    screenshot: "only-on-failure",
    // Inject the Tauri invoke mock before every page load. The mock
    // gives us deterministic responses for every backend command.
    // See e2e/_setup/mock-invoke.js for the implementation.
    contextOptions: {
      // bypassCSP needed because vite serves with a strict CSP that
      // would block the injected script.
      bypassCSP: true,
    },
  },

  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],

  // Boot `vite dev` ourselves so the suite can run from cold.
  webServer: {
    command: "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
    stdout: "ignore",
    stderr: "pipe",
  },
});
