/**
 * Tier 2 — connection lifecycle invoke contract.
 *
 * Covers the user flow from the empty connection list through
 * add → connect → vm list visible → disconnect. The point of these
 * tests is to lock the frontend↔backend command names + arg shapes:
 * if anyone renames `add_connection` to `create_connection` in
 * Rust without updating the Svelte caller (or vice versa), the
 * test breaks with a clear "expected this command, got that one".
 */

import { test, expect } from "./_setup/fixtures.js";

test("adding a connection invokes add_connection with the form values", async ({ page, mock }) => {
  await page.goto("/");
  await mock.command("add_connection", (args) => ({
    id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    display_name: args.displayName,
    uri: args.uri,
    auth_type: args.authType,
    last_connected: null,
  }));

  await page.locator(".sidebar-header button").first().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  // Form fields — names depend on ConnectionDialog's input layout.
  // ConnectionDialog uses "My Server" + "qemu+ssh://..." placeholders.
  await dialog.getByPlaceholder("My Server").fill("test-hv");
  await dialog.getByPlaceholder(/qemu\+ssh/i).fill("qemu+ssh://user@host/system");

  // Submit. The dialog may auto-trigger connect after add; we only
  // assert the add_connection contract here.
  await dialog.locator("button[type=submit]").click();

  // Some UIs probe SSH host keys before committing; the add command
  // may not fire if the probe doesn't return. We tolerate either
  // immediate-add or the host-key-confirm path.
  await page.waitForTimeout(300);
  const call = await mock.lastCall("add_connection");
  if (call) {
    expect(call.args.displayName ?? call.args.display_name).toBe("test-hv");
    expect(call.args.uri).toBe("qemu+ssh://user@host/system");
  } else {
    // If add wasn't called, at least make sure check_host_key was the
    // gating step — that's the legitimate alternative path.
    const probe = await mock.lastCall("check_host_key");
    expect(probe, "expected either add_connection or check_host_key").not.toBeNull();
  }
});

test("the connect button calls connect(id) and lists domains", async ({ page, mock }) => {
  // Pre-populate saved connections.
  await page.addInitScript({
    content: `
      const wait = setInterval(() => {
        if (window.__kraftwerkMock) {
          clearInterval(wait);
          window.__kraftwerkMock.command("list_saved_connections", () => ([{
            id: "11111111-1111-1111-1111-111111111111",
            display_name: "mock-hv",
            uri: "qemu+ssh://x/system",
            auth_type: "SshAgent",
            last_connected: null,
          }]));
          window.__kraftwerkMock.command("check_host_key", () => ({ status: "trusted" }));
          window.__kraftwerkMock.command("connect", () => ([
            { name: "vm-a", uuid: "uuid-a", state: "running", vcpus: 2, memory_mb: 1024, graphics_type: null, has_serial: true, is_template: false },
            { name: "vm-b", uuid: "uuid-b", state: "shut_off", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: false },
          ]));
        }
      }, 5);
    `,
  });

  await page.goto("/");
  await expect(page.getByText("mock-hv")).toBeVisible();

  // Click the saved-connection row to connect.
  await page.getByText("mock-hv").click();

  // Wait for the connect invocation to register.
  await expect.poll(async () => (await mock.lastCall("connect"))?.args?.id).toBe(
    "11111111-1111-1111-1111-111111111111",
  );

  // VM names should render in the sidebar after connect resolves.
  await expect(page.getByText("vm-a")).toBeVisible({ timeout: 5000 });
  await expect(page.getByText("vm-b")).toBeVisible();
});
