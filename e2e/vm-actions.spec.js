/**
 * Tier 2 — VM lifecycle button contract.
 *
 * Asserts that clicking the action buttons on a selected VM dispatches
 * the right Tauri command with the right argument shape. Locks the
 * mapping from button to command so refactors in either layer have
 * to be done atomically.
 */

import { test, expect } from "./_setup/fixtures.js";

const HV_ID = "11111111-1111-1111-1111-111111111111";

async function bootIntoConnectedState(page, vms) {
  await page.addInitScript({
    content: `
      const list = ${JSON.stringify(vms)};
      const wait = setInterval(() => {
        if (window.__kraftwerkMock) {
          clearInterval(wait);
          window.__kraftwerkMock.command("list_saved_connections", () => ([{
            id: "${HV_ID}",
            display_name: "mock-hv",
            uri: "qemu+ssh://x/system",
            auth_type: "SshAgent",
            last_connected: null,
          }]));
          window.__kraftwerkMock.command("check_host_key", () => ({ status: "trusted" }));
          window.__kraftwerkMock.command("connect", () => list);
          window.__kraftwerkMock.command("list_domains", () => list);
          window.__kraftwerkMock.command("get_domain_stats", () => null);
          window.__kraftwerkMock.command("has_managed_save", () => false);
        }
      }, 5);
    `,
  });
  await page.goto("/");
  await page.getByText("mock-hv").click();
}

test("starting a shut-off VM invokes start_domain(name)", async ({ page, mock }) => {
  await bootIntoConnectedState(page, [
    { name: "vm-stopped", uuid: "u1", state: "shut_off", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: false },
  ]);
  await page.getByText("vm-stopped").click();
  await page.getByRole("button", { name: /^start$/i }).first().click();

  await expect.poll(async () => (await mock.lastCall("start_domain"))?.args?.name).toBe("vm-stopped");
});

test("shutdown button invokes shutdown_domain(name) on running VM", async ({ page, mock }) => {
  await bootIntoConnectedState(page, [
    { name: "vm-running", uuid: "u1", state: "running", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: false },
  ]);
  await page.getByText("vm-running").click();
  await page.getByRole("button", { name: /^shutdown$/i }).first().click();

  await expect.poll(async () => (await mock.lastCall("shutdown_domain"))?.args?.name).toBe("vm-running");
});

test("force-off invokes destroy_domain(name) after confirmation", async ({ page, mock }) => {
  // The force-off path may prompt via a confirm dialog; accept it.
  page.on("dialog", (d) => d.accept());
  await bootIntoConnectedState(page, [
    { name: "vm-running", uuid: "u1", state: "running", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: false },
  ]);
  await page.getByText("vm-running").click();
  await page.getByRole("button", { name: /force off|destroy/i }).first().click();

  await expect.poll(async () => (await mock.lastCall("destroy_domain"))?.args?.name).toBe("vm-running");
});

test("pause on running VM invokes suspend_domain", async ({ page, mock }) => {
  await bootIntoConnectedState(page, [
    { name: "vm-running", uuid: "u1", state: "running", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: false },
  ]);
  await page.getByText("vm-running").click();
  await page.getByRole("button", { name: /^pause$/i }).first().click();

  await expect.poll(async () => (await mock.lastCall("suspend_domain"))?.args?.name).toBe("vm-running");
});

test("resume on paused VM invokes resume_domain", async ({ page, mock }) => {
  await bootIntoConnectedState(page, [
    { name: "vm-paused", uuid: "u1", state: "paused", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: false },
  ]);
  await page.getByText("vm-paused").click();
  await page.getByRole("button", { name: /^resume$/i }).first().click();

  await expect.poll(async () => (await mock.lastCall("resume_domain"))?.args?.name).toBe("vm-paused");
});

test("template badge + 'Mark as template' visible on shut-off VM", async ({ page }) => {
  await bootIntoConnectedState(page, [
    { name: "tpl-base", uuid: "u1", state: "shut_off", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: true },
  ]);
  await page.getByText("tpl-base").click();
  await expect(page.getByText(/^TEMPLATE$/)).toBeVisible();
  await expect(page.getByRole("button", { name: /unmark template/i })).toBeVisible();
});

test("'Mark as template' on non-template invokes set_template_flag(mark:true)", async ({ page, mock }) => {
  await bootIntoConnectedState(page, [
    { name: "vm-blank", uuid: "u1", state: "shut_off", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: false },
  ]);
  await page.getByText("vm-blank").click();
  // The dynamic import in toggleTemplate eventually calls refreshVms
  // which logs an error against our event-listener shim; tolerate
  // that — what we care about is that set_template_flag fires first.
  const btn = page.getByRole("button", { name: /^mark as template$/i });
  await expect(btn).toBeVisible();
  await btn.click();

  await expect.poll(async () => (await mock.lastCall("set_template_flag"))?.args).toMatchObject({
    name: "vm-blank",
    mark: true,
  });
});
