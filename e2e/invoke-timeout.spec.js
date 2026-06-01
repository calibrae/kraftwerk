/**
 * Tier 2 — invoke timeout wrapper.
 *
 * Asserts that a backend command that never resolves does NOT freeze
 * the UI past the wrapper's configured budget. Locks in the contract
 * from src/lib/invoke.js: every invoke must reject with
 * InvokeTimeoutError when the backend hangs.
 *
 * We can't easily prove "the UI is responsive" without intermediate
 * UI to click, so the test exercises the wrapper directly via
 * page.evaluate.
 */

import { test, expect } from "./_setup/fixtures.js";

test("invoke wrapper rejects when backend never resolves", async ({ page, mock }) => {
  await page.goto("/");

  // Wire a list_saved_connections handler that never resolves. We
  // can't await the rawInvoke here from the test directly — instead
  // we drive the wrapper via page.evaluate.
  await mock.command("list_saved_connections", () => new Promise(() => {}));

  const { ok, error, elapsedMs } = await page.evaluate(async () => {
    // Override the timeout for the test path by routing through the
    // wrapper's exported invoke directly. Production calls go through
    // the COMMAND_TIMEOUTS table; we just need to prove the race works.
    const { invoke, InvokeTimeoutError } = await import("/src/lib/invoke.js");
    const t0 = performance.now();
    try {
      // pick a command we expect to land in the default FAST budget;
      // we shorten it in test via env? No — the default is 15s. To
      // avoid sitting around 15s, use a never-registered command with
      // a hand-rolled race. The wrapper's contract is what we want
      // to verify, not the specific timeout value. Instead, race
      // against a 17s ceiling — if the wrapper works it fires at 15s.
      await invoke("list_saved_connections");
      return { ok: true, error: null, elapsedMs: performance.now() - t0 };
    } catch (e) {
      return {
        ok: false,
        error: { name: e.name, message: e.message, isTimeout: e instanceof InvokeTimeoutError },
        elapsedMs: performance.now() - t0,
      };
    }
  });

  expect(ok).toBe(false);
  expect(error.isTimeout).toBe(true);
  expect(error.name).toBe("InvokeTimeoutError");
  // Wrapper budget is 15s for the default; allow a fudge factor.
  expect(elapsedMs).toBeGreaterThan(14_000);
  expect(elapsedMs).toBeLessThan(17_000);
});
