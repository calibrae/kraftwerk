/**
 * Tier 2 — phase 5 + phase 6 dialog invoke contracts.
 *
 * Each dialog ships an invoke against a specific Tauri command. These
 * tests open the dialog, fill the form, submit, and verify the
 * outgoing invoke matches the command name + arg shape the backend
 * expects.
 */

import { test, expect } from "./_setup/fixtures.js";

const HV_ID = "11111111-1111-1111-1111-111111111111";
const HV_ID_2 = "22222222-2222-2222-2222-222222222222";

async function bootIntoConnectedState(page, opts = {}) {
  const { vms = [], pools = [], networks = [], extraConnections = [] } = opts;
  await page.addInitScript({
    content: `
      const vms = ${JSON.stringify(vms)};
      const pools = ${JSON.stringify(pools)};
      const networks = ${JSON.stringify(networks)};
      const extra = ${JSON.stringify(extraConnections)};
      const wait = setInterval(() => {
        if (window.__kraftwerkMock) {
          clearInterval(wait);
          window.__kraftwerkMock.command("list_saved_connections", () => ([
            { id: "${HV_ID}", display_name: "mock-hv", uri: "qemu+ssh://x/system", auth_type: "SshAgent", last_connected: null },
            ...extra,
          ]));
          window.__kraftwerkMock.command("check_host_key", () => ({ status: "trusted" }));
          window.__kraftwerkMock.command("connect", () => vms);
          window.__kraftwerkMock.command("list_domains", () => vms);
          window.__kraftwerkMock.command("list_storage_pools", () => pools);
          window.__kraftwerkMock.command("list_networks", () => networks);
          window.__kraftwerkMock.command("list_open_connections", () => ["${HV_ID}", ...extra.map(c => c.id)]);
          window.__kraftwerkMock.command("get_domain_stats", () => null);
          window.__kraftwerkMock.command("has_managed_save", () => false);
          window.__kraftwerkMock.command("inspect_ova", () => ({
            name: "imported-vm", vcpus: 4, memory_mib: 4096,
            disks: [{ disk_id: "d1", file_href: "disk1.vmdk", capacity_bytes: 42949672960, format: "vmdk" }],
            networks: ["VM Network"], guest_os: "ubuntu64Guest",
          }));
          window.__kraftwerkMock.command("import_ova", () => "imported-vm");
        }
      }, 5);
    `,
  });
  await page.goto("/");
  await page.getByText("mock-hv").click();
}

test("clone-from-template dialog calls clone_from_template with cloud-init payload", async ({ page, mock }) => {
  await bootIntoConnectedState(page, {
    vms: [{
      name: "ubuntu-tpl", uuid: "u1", state: "shut_off", vcpus: 1, memory_mb: 512,
      graphics_type: null, has_serial: false, is_template: true,
    }],
  });
  await page.getByText("ubuntu-tpl").click();
  await page.getByRole("button", { name: /instantiate.*cloud-init/i }).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByLabel(/new vm name/i).fill("ubuntu-vm1");
  await dialog.getByLabel(/hostname/i).fill("vm1");
  await dialog.getByLabel(/ssh authorized_keys/i).fill("ssh-ed25519 AAAA me@host");
  await dialog.getByRole("button", { name: /instantiate/i }).click();

  await expect.poll(async () => (await mock.lastCall("clone_from_template"))?.args?.templateName).toBe("ubuntu-tpl");
  const call = await mock.lastCall("clone_from_template");
  expect(call.args.options).toMatchObject({ target_name: "ubuntu-vm1" });
  expect(call.args.cloudInit).toMatchObject({
    hostname: "vm1",
    ssh_authorized_keys: ["ssh-ed25519 AAAA me@host"],
  });
});

test("OVA import inspect → import flow", async ({ page, mock }) => {
  await bootIntoConnectedState(page, {
    pools: [{ name: "default", is_active: true, pool_type: "dir", target_path: "/var/lib/libvirt/images", capacity: 0, allocation: 0, available: 0, autostart: true }],
    networks: [{ name: "default", is_active: true }],
  });

  // Open the OVA import dialog from the top-level toolbar.
  await page.getByRole("button", { name: /import ova/i }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  await dialog.getByPlaceholder(/\.ova/i).fill("/tmp/test.ova");
  await dialog.getByRole("button", { name: /^inspect$/i }).click();

  await expect.poll(async () => (await mock.lastCall("inspect_ova"))?.args?.ovaPath).toBe("/tmp/test.ova");
  // After inspect, the import button becomes available.
  await expect(dialog.getByText(/imported-vm/i)).toBeVisible();

  await dialog.getByRole("button", { name: /^import$/i }).click();
  await expect.poll(async () => (await mock.lastCall("import_ova"))?.args?.ovaPath).toBe("/tmp/test.ova");
  const importCall = await mock.lastCall("import_ova");
  expect(importCall.args.poolName).toBe("default");
});

test("migrate dialog lists open connections as destinations", async ({ page }) => {
  await bootIntoConnectedState(page, {
    vms: [{ name: "vm-running", uuid: "u1", state: "running", vcpus: 1, memory_mb: 512, graphics_type: null, has_serial: false, is_template: false }],
    extraConnections: [
      { id: HV_ID_2, display_name: "second-hv", uri: "qemu+ssh://y/system", auth_type: "SshAgent", last_connected: null },
    ],
  });
  await page.getByText("vm-running").click();
  await page.getByRole("button", { name: /^migrate$/i }).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  // Options inside a <select> aren't "visible" in the playwright
  // sense until the dropdown is open; assert by inspecting the
  // option's text instead.
  const optionText = await dialog.locator("select option").allTextContents();
  expect(optionText.some((t) => /second-hv/i.test(t))).toBe(true);
});
