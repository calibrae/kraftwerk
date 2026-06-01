/**
 * Timeout + degradation wrapper around @tauri-apps/api/core::invoke.
 *
 * Background: libvirt RPCs over qemu+ssh can hang for ~75s when the
 * underlying TCP connection is unresponsive (host unreachable, packet
 * loss, sshd swapped). Tauri's command pipeline is happy to wait that
 * long; the UI is not — buttons stop responding, polls queue up, the
 * whole webview feels frozen.
 *
 * This wrapper enforces a per-call timeout on every invoke. When a
 * command exceeds the budget we reject with a TimeoutError so the
 * caller can mark the operation as failed and update UI state instead
 * of waiting indefinitely.
 *
 * Per-command timeouts live in COMMAND_TIMEOUTS. Long-running paths
 * (live migration, OVA import, image download, console.upload) get a
 * generous budget; everything else defaults to FAST_TIMEOUT_MS.
 *
 * The wrapper also surfaces "connection_degraded" via window event so
 * the polling layer can back off without each consumer re-implementing
 * that logic.
 */

import { invoke as rawInvoke } from "@tauri-apps/api/core";

const FAST_TIMEOUT_MS = 15_000;

// Operations that can legitimately take much longer than the default.
// Keep the keys here in sync with the Tauri command names in
// src-tauri/src/lib.rs::generate_handler!.
const COMMAND_TIMEOUTS = {
  migrate_domain: 30 * 60 * 1000,        // long-running blocking call
  import_ova: 30 * 60 * 1000,            // VMDK convert can be slow
  download_image: 30 * 60 * 1000,        // cloud image download
  upload_volume: 30 * 60 * 1000,         // stream upload
  create_vm: 5 * 60 * 1000,              // VM creation + install media
  clone_domain: 10 * 60 * 1000,          // full disk copy
  clone_from_template: 10 * 60 * 1000,   // ditto + cloud-init build
  build_cloud_init_iso: 60_000,          // ssh+genisoimage
  inspect_ova: 60_000,                   // local tar walk
  core_dump_domain: 5 * 60 * 1000,
  screenshot_domain: 60_000,
  // libvirt's connect/open does an SSH handshake — give it 30s before
  // we declare the connection dead.
  connect: 30_000,
  check_host_key: 30_000,
};

/**
 * Custom error for timeouts. UI code can `instanceof InvokeTimeoutError`
 * to distinguish a stalled backend from a command that returned a
 * legitimate Err.
 */
export class InvokeTimeoutError extends Error {
  constructor(cmd, ms) {
    super(`invoke('${cmd}') timed out after ${ms}ms`);
    this.name = "InvokeTimeoutError";
    this.cmd = cmd;
    this.timeoutMs = ms;
  }
}

/**
 * Drop-in for @tauri-apps/api/core::invoke that rejects with
 * InvokeTimeoutError when the backend takes too long.
 */
export async function invoke(cmd, args) {
  const ms = COMMAND_TIMEOUTS[cmd] ?? FAST_TIMEOUT_MS;
  let timeoutHandle;
  const timeout = new Promise((_, reject) => {
    timeoutHandle = setTimeout(() => {
      // Best-effort notice to whoever's listening (the polling layer
      // in app.svelte.js subscribes to this).
      if (typeof window !== "undefined") {
        window.dispatchEvent(new CustomEvent("kraftwerk:invoke-timeout", {
          detail: { cmd, ms },
        }));
      }
      reject(new InvokeTimeoutError(cmd, ms));
    }, ms);
  });
  try {
    return await Promise.race([rawInvoke(cmd, args), timeout]);
  } finally {
    clearTimeout(timeoutHandle);
  }
}
