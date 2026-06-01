/**
 * Tier 1 — smoke tests.
 *
 * These exist to catch the class of regression that shipped in
 * v0.2.0/0.2.1/0.2.2: an uncaught JS error in a Svelte component
 * blanks the entire app. None of those failures could have made it
 * past a "does the page render at all" check.
 *
 * Every test in this file relies on the global `pageerror` /
 * `console.error` backstop in fixtures.js. The assertions in each
 * test target user-visible behaviour; the backstop covers the silent
 * "page is blank because something threw" mode.
 */

import { test, expect } from "./_setup/fixtures.js";

test("app boots without errors and renders the connections sidebar", async ({ page }) => {
  await page.goto("/");
  // The sidebar is always visible; if it's not, something at the root
  // of the app crashed.
  await expect(page.getByRole("heading", { name: /connections/i })).toBeVisible({ timeout: 10_000 });
  // The empty-state shows when the saved-connections list is empty
  // (our mock invoke returns []).
  await expect(page.getByText(/no connections/i)).toBeVisible();
});

test("clicking + opens the add-connection dialog", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /connections/i })).toBeVisible();

  // The "+" affordance — match a small button at the top of the sidebar.
  // There are several "+" looking buttons in the app; the sidebar's is
  // the one inside the CONNECTIONS heading row.
  const addBtn = page.getByRole("button", { name: /add connection/i }).first();
  await addBtn.click();

  await expect(page.getByRole("dialog")).toBeVisible();
});

test("dialog closes cleanly with the cancel/close affordance", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: /add connection/i }).first().click();
  await expect(page.getByRole("dialog")).toBeVisible();

  // Either a Cancel button or the × control closes it.
  const cancel = page.getByRole("button", { name: /^cancel$/i }).first();
  if (await cancel.isVisible()) {
    await cancel.click();
  } else {
    await page.keyboard.press("Escape");
  }
  // Dialog should be gone or, at minimum, no longer the focus target.
  await expect(page.getByRole("dialog")).toBeHidden();
});

test("no view tabs visible when not connected (single-pane fallback)", async ({ page }) => {
  await page.goto("/");
  // Without a connection, the Virtual Machines / Networks / Storage
  // tabs shouldn't appear at all — the main area shows the no-VM
  // placeholder rendered by VmDetail.
  await expect(page.getByRole("button", { name: /^virtual machines/i })).toBeHidden();
});

test("backend list calls happen on boot", async ({ page, mock }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /connections/i })).toBeVisible();

  // The connection-load happens during onMount. By the time the
  // sidebar is visible, the invoke should have been called.
  const call = await mock.lastCall("list_saved_connections");
  expect(call).not.toBeNull();
});

test("mocking a saved connection makes it show in the sidebar", async ({ page, mock }) => {
  // Register the mock BEFORE navigating so the onMount call sees it.
  await page.addInitScript({
    content: `
      const wait = setInterval(() => {
        if (window.__kraftwerkMock) {
          clearInterval(wait);
          window.__kraftwerkMock.command("list_saved_connections", () => ([
            {
              id: "11111111-1111-1111-1111-111111111111",
              display_name: "mock-hypervisor",
              uri: "qemu+ssh://test@example.invalid/system",
              auth_type: "SshAgent",
              last_connected: null,
            }
          ]));
        }
      }, 5);
    `,
  });
  await page.goto("/");
  await expect(page.getByText("mock-hypervisor")).toBeVisible({ timeout: 10_000 });
});
