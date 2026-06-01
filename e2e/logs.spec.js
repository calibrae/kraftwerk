/**
 * Tier 2 — logs viewer invoke contract.
 *
 * Verifies the LogsDialog wires up to get_logs / set_log_level /
 * clear_logs as expected, and that the verbose toggle drives the
 * backend-side log level.
 */

import { test, expect } from "./_setup/fixtures.js";

const HV_ID = "11111111-1111-1111-1111-111111111111";

async function bootIntoConnectedState(page) {
  await page.addInitScript({
    content: `
      const wait = setInterval(() => {
        if (window.__kraftwerkMock) {
          clearInterval(wait);
          window.__kraftwerkMock.command("list_saved_connections", () => ([{
            id: "${HV_ID}", display_name: "mock-hv", uri: "qemu+ssh://x/system",
            auth_type: "SshAgent", last_connected: null,
          }]));
          window.__kraftwerkMock.command("check_host_key", () => ({ status: "trusted" }));
          window.__kraftwerkMock.command("connect", () => []);
          window.__kraftwerkMock.command("get_log_level", () => "info");
          // Honor after_ts_ms like the real backend, so the dialog's
          // incremental poll doesn't see the same entries every tick.
          window.__kraftwerkMock.command("get_logs", (args) => {
            const all = [
              { ts_ms: 1717000000000, level: "info",  target: "kraftwerk_lib", message: "boot" },
              { ts_ms: 1717000001000, level: "warn",  target: "kraftwerk_lib::libvirt", message: "slow probe" },
              { ts_ms: 1717000002000, level: "error", target: "kraftwerk_lib::libvirt", message: "dial failed" },
            ];
            const after = (args && (args.afterTsMs ?? args.after_ts_ms)) ?? null;
            return after == null ? all : all.filter(e => e.ts_ms > after);
          });
        }
      }, 5);
    `,
  });
  await page.goto("/");
  await page.getByText("mock-hv").click();
}

test("help button opens the logs dialog and renders entries", async ({ page }) => {
  await bootIntoConnectedState(page);
  await page.getByRole("button", { name: /show application logs/i }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  // Three mock entries — each level renders.
  await expect(dialog.getByText("boot")).toBeVisible();
  await expect(dialog.getByText("slow probe")).toBeVisible();
  await expect(dialog.getByText("dial failed")).toBeVisible();
});

test("verbose toggle calls set_log_level(debug)", async ({ page, mock }) => {
  await bootIntoConnectedState(page);
  await page.getByRole("button", { name: /show application logs/i }).click();

  const verboseCb = page.getByRole("dialog").getByText("Verbose (debug)").locator("..").locator("input");
  await verboseCb.check();

  await expect.poll(async () => (await mock.lastCall("set_log_level"))?.args?.level).toBe("debug");
});

test("clear button invokes clear_logs", async ({ page, mock }) => {
  await bootIntoConnectedState(page);
  await page.getByRole("button", { name: /show application logs/i }).click();
  await page.getByRole("dialog").getByRole("button", { name: /^clear$/i }).click();

  await expect.poll(async () => await mock.lastCall("clear_logs")).not.toBeNull();
});
