<script>
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import RFB from "$lib/vendor/novnc/core/rfb.js";

  let { vmName, onClose } = $props();

  let containerEl = $state(null);
  let connected = $state(false);
  let error = $state(null);
  let rfb = null;
  let wsPort = null;

  onMount(async () => {
    try {
      wsPort = await invoke("open_vnc", { name: vmName });
      const url = `ws://127.0.0.1:${wsPort}`;
      rfb = new RFB(containerEl, url, {});
      rfb.viewOnly = false;
      rfb.scaleViewport = true;
      rfb.resizeSession = false;
      rfb.background = "#000";

      rfb.addEventListener("connect", () => { connected = true; });
      rfb.addEventListener("disconnect", (e) => {
        connected = false;
        if (e.detail && !e.detail.clean) {
          error = `Disconnected: ${e.detail.reason || "unknown"}`;
        }
      });
      rfb.addEventListener("credentialsrequired", () => {
        // FD-based VNC tunnel has no auth (libvirt strips it); if this fires,
        // something is unexpected — send an empty password.
        rfb.sendCredentials({ password: "" });
      });
    } catch (e) {
      error = e?.message || JSON.stringify(e);
    }
  });

  onDestroy(async () => {
    try { if (rfb) rfb.disconnect(); } catch (_) {}
    rfb = null;
    try { await invoke("close_vnc"); } catch (_) {}
  });

  function sendCtrlAltDel() {
    if (rfb) rfb.sendCtrlAltDel();
  }
</script>

<div class="vnc-container">
  <div class="vnc-toolbar">
    <span class="title">
      VNC Console — {vmName}
      {#if connected}
        <span class="badge connected">Connected</span>
      {:else if error}
        <span class="badge err">Error</span>
      {:else}
        <span class="badge connecting">Connecting...</span>
      {/if}
    </span>
    <div class="actions">
      {#if connected}
        <button class="btn" onclick={sendCtrlAltDel} title="Send Ctrl+Alt+Del">Ctrl+Alt+Del</button>
      {/if}
      <button class="btn btn-close" onclick={onClose}>Disconnect</button>
    </div>
  </div>

  {#if error}
    <div class="err-banner">{error}</div>
  {/if}

  <div class="vnc-screen" bind:this={containerEl}></div>
</div>

<style>
  .vnc-container { display: flex; flex-direction: column; height: 100%; background: #000; }
  .vnc-toolbar {
    display: flex; justify-content: space-between; align-items: center;
    padding: 8px 16px; background: var(--bg-surface);
    border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .title { font-size: 13px; font-weight: 500; display: flex; align-items: center; gap: 8px; }

  .badge { display: inline-block; padding: 1px 8px; border-radius: 10px; font-size: 11px; }
  .connected { background: rgba(52, 211, 153, 0.15); color: #34d399; }
  .connecting { background: rgba(251, 191, 36, 0.15); color: #fbbf24; }
  .err { background: rgba(239, 68, 68, 0.15); color: #ef4444; }

  .actions { display: flex; gap: 6px; }
  .btn {
    padding: 4px 12px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); cursor: pointer; font-size: 12px; font-family: inherit;
  }
  .btn:hover { background: var(--bg-hover); }
  .btn-close:hover { background: #7f1d1d; color: #fca5a5; border-color: #7f1d1d; }

  .err-banner {
    padding: 8px 16px; background: rgba(239, 68, 68, 0.1);
    border-bottom: 1px solid rgba(239, 68, 68, 0.3);
    color: #ef4444; font-size: 12px; flex-shrink: 0;
  }

  .vnc-screen { flex: 1; overflow: hidden; background: #000; }
  .vnc-screen :global(canvas) { display: block; }
</style>
