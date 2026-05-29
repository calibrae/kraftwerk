/**
 * Shared playwright fixtures.
 *
 * - `page` is the standard playwright page, but pre-wired with:
 *   - the Tauri invoke / event mock from mock-invoke.js installed as
 *     an init script (runs before any page navigation),
 *   - a `pageerror` listener that fails the test on any uncaught JS
 *     error (this is the Tier 1 backstop that would have caught the
 *     `vm is not defined` regression),
 *   - a `console.error` listener with the same semantics.
 *
 * - `mock` exposes the in-page __kraftwerkMock surface so tests can
 *   register per-command handlers without writing
 *   `page.evaluate(() => window.__kraftwerkMock.command(…))` boilerplate.
 */

import { test as base, expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const MOCK_SOURCE = fs.readFileSync(path.join(__dirname, "mock-invoke.js"), "utf8");

export const test = base.extend({
  page: async ({ page }, use) => {
    const pageErrors = [];
    const consoleErrors = [];

    page.on("pageerror", (err) => pageErrors.push(err));
    page.on("console", (msg) => {
      if (msg.type() === "error") consoleErrors.push(msg.text());
    });

    await page.addInitScript({ content: MOCK_SOURCE });

    await use(page);

    // Backstop assertions — surfaces would-be silent UI breakage.
    if (pageErrors.length) {
      throw new Error(
        `Page emitted ${pageErrors.length} uncaught error(s):\n` +
          pageErrors.map((e, i) => `  ${i + 1}. ${e.message}\n${e.stack ?? ""}`).join("\n"),
      );
    }
    // Tauri's event plugin logs an error when the runtime is mocked
    // — ignore that one specific message but treat anything else as a
    // failure.
    const realConsoleErrors = consoleErrors.filter(
      (m) => !m.includes("event.listen") && !m.includes("__TAURI_EVENT_PLUGIN_INTERNALS__"),
    );
    if (realConsoleErrors.length) {
      throw new Error(
        `Console reported ${realConsoleErrors.length} error(s):\n` +
          realConsoleErrors.map((m, i) => `  ${i + 1}. ${m}`).join("\n"),
      );
    }
  },

  mock: async ({ page }, use) => {
    const api = {
      // Register a per-command handler. MUST be called after page.goto()
      // — the mock surface only exists on the navigated page. For
      // pre-navigation registration use page.addInitScript with the
      // inline `window.__kraftwerkMock.command(...)` pattern.
      async command(name, handler) {
        await page.evaluate(
          ([n, src]) => {
            const fn = new Function("args", `return (${src})(args);`);
            window.__kraftwerkMock.command(n, fn);
          },
          [name, handler.toString()],
        );
      },
      async lastCall(name) {
        return page.evaluate((n) => window.__kraftwerkMock.lastCall(n), name);
      },
      async calls() {
        return page.evaluate(() => window.__kraftwerkMock.calls());
      },
      async reset() {
        await page.evaluate(() => window.__kraftwerkMock.reset());
      },
    };
    await use(api);
  },
});

export { expect };
