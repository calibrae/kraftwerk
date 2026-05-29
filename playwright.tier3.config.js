import { defineConfig } from "@playwright/test";

/**
 * Tier 3 config: launch the real built Tauri binary and assert it
 * boots without dying. No SvelteKit dev server, no webview mocking.
 *
 * Slow (~10s/test, plus cargo build the first time). Wired into a
 * separate npm script so the default `npm run test:e2e` stays fast.
 */
export default defineConfig({
  testDir: "./e2e",
  testMatch: /tier3-.*\.spec\.js$/,
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: "list",
  timeout: 60_000,
});
