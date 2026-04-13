<script>
  import { onMount } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import VmDetail from "$lib/components/VmDetail.svelte";
  import NetworksView from "$lib/components/NetworksView.svelte";
  import ConnectionDialog from "$lib/components/ConnectionDialog.svelte";
  import CreateNetworkDialog from "$lib/components/CreateNetworkDialog.svelte";
  import StorageView from "$lib/components/StorageView.svelte";
  import CreatePoolDialog from "$lib/components/CreatePoolDialog.svelte";
  import CreateVolumeDialog from "$lib/components/CreateVolumeDialog.svelte";
  import VmWizard from "$lib/components/VmWizard.svelte";
  import { loadConnections, addConnection, connect, getState, clearError } from "$lib/stores/app.svelte.js";

  const appState = getState();
  let showConnectionDialog = $state(false);
  let showNetworkDialog = $state(false);
  let showPoolDialog = $state(false);
  let showVolumeDialog = $state(false);
  let showVmWizard = $state(false);
  let volumePoolName = $state("");
  let view = $state("vms"); // "vms" | "networks" | "storage"

  const DEV_CONNECTION = {
    name: "testhost",
    uri: "qemu+ssh://testuser@testhost/system",
    authType: "ssh_agent",
  };

  onMount(async () => {
    await loadConnections();
    if (appState.savedConnections.length === 0) {
      try {
        const conn = await addConnection(DEV_CONNECTION.name, DEV_CONNECTION.uri, DEV_CONNECTION.authType);
        await connect(conn.id);
      } catch (_) {}
    }
  });
</script>

<div class="app-layout">
  <Sidebar onAddConnection={() => showConnectionDialog = true} />

  <main class="main-area">
    {#if appState.isConnected}
      <div class="view-tabs">
        <button class="view-tab" class:active={view === "vms"} onclick={() => view = "vms"}>
          Virtual Machines <span class="count">{appState.vms.length}</span>
        </button>
        <button class="view-tab new-btn" onclick={() => showVmWizard = true} title="Create new VM">+ New VM</button>
        <button class="view-tab" class:active={view === "networks"} onclick={() => view = "networks"}>
          Networks <span class="count">{appState.networks.length}</span>
        </button>
        <button class="view-tab" class:active={view === "storage"} onclick={() => view = "storage"}>
          Storage <span class="count">{appState.pools.length}</span>
        </button>
      </div>
    {/if}

    <div class="view-content">
      {#if view === "networks" && appState.isConnected}
        <NetworksView onCreateNetwork={() => showNetworkDialog = true} />
      {:else if view === "storage" && appState.isConnected}
        <StorageView
          onCreatePool={() => showPoolDialog = true}
          onCreateVolume={(name) => { volumePoolName = name; showVolumeDialog = true; }}
        />
      {:else}
        <VmDetail />
      {/if}
    </div>
  </main>
</div>

<ConnectionDialog bind:open={showConnectionDialog} />
<CreateNetworkDialog bind:open={showNetworkDialog} />
<CreatePoolDialog bind:open={showPoolDialog} />
<CreateVolumeDialog bind:open={showVolumeDialog} poolName={volumePoolName} />
<VmWizard bind:open={showVmWizard} />

{#if appState.error}
  <div class="toast-error">
    <span>{appState.error?.message || JSON.stringify(appState.error)}</span>
    <button onclick={clearError}>&#x2715;</button>
  </div>
{/if}

<style>
  :global(*) { box-sizing: border-box; }

  :global(body) {
    margin: 0; padding: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    font-size: 14px; color: var(--text); background: var(--bg); overflow: hidden;
  }

  :global(:root) {
    --bg: #1a1a2e;
    --bg-sidebar: #16162a;
    --bg-surface: #1e1e3a;
    --bg-button: #252547;
    --bg-hover: #2a2a50;
    --bg-selected: #33335a;
    --bg-input: #1a1a35;
    --border: #2d2d55;
    --text: #e4e4f0;
    --text-muted: #8888aa;
    --accent: #6366f1;
    --accent-dim: rgba(99, 102, 241, 0.2);
  }

  .app-layout { display: flex; height: 100vh; overflow: hidden; }

  .main-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .view-tabs {
    display: flex;
    gap: 2px;
    padding: 12px 24px 0;
    background: var(--bg-sidebar);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .view-tab {
    padding: 8px 16px;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    font-size: 13px;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    margin-bottom: -1px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .view-tab:hover { color: var(--text); }

  .view-tab.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }

  .count {
    display: inline-block;
    padding: 1px 8px;
    background: var(--bg-button);
    border-radius: 10px;
    font-size: 11px;
    color: var(--text-muted);
  }

  .view-tab.active .count {
    background: var(--accent-dim);
    color: var(--text);
  }

  .view-tab.new-btn {
    margin-left: auto;
    color: var(--accent);
    font-weight: 500;
  }
  .view-tab.new-btn:hover { color: var(--text); background: var(--accent-dim); border-radius: 6px; margin-bottom: 2px; border-bottom-color: transparent; }

  .view-content {
    flex: 1;
    overflow: hidden;
    display: flex;
  }

  .view-content > :global(*) { flex: 1; }

  .toast-error {
    position: fixed;
    bottom: 16px; right: 16px;
    max-width: 400px;
    padding: 12px 16px;
    background: rgba(127, 29, 29, 0.95);
    border: 1px solid rgba(239, 68, 68, 0.4);
    border-radius: 8px;
    color: #fca5a5;
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 12px;
    z-index: 200;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  }

  .toast-error span { flex: 1; word-break: break-word; }
  .toast-error button {
    background: none; border: none; color: #fca5a5; cursor: pointer;
    font-size: 14px; padding: 0; flex-shrink: 0;
  }
</style>
