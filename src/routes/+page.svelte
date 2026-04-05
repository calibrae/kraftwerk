<script>
  import { onMount } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import VmDetail from "$lib/components/VmDetail.svelte";
  import ConnectionDialog from "$lib/components/ConnectionDialog.svelte";
  import { loadConnections, getState, clearError } from "$lib/stores/app.svelte.js";

  const state = getState();
  let showConnectionDialog = $state(false);

  onMount(() => {
    loadConnections();
  });
</script>

<div class="app-layout">
  <Sidebar onAddConnection={() => showConnectionDialog = true} />
  <VmDetail />
</div>

<ConnectionDialog bind:open={showConnectionDialog} />

{#if state.error}
  <div class="toast-error">
    <span>{state.error?.message || JSON.stringify(state.error)}</span>
    <button onclick={clearError}>&#x2715;</button>
  </div>
{/if}

<style>
  :global(*) {
    box-sizing: border-box;
  }

  :global(body) {
    margin: 0;
    padding: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    font-size: 14px;
    color: var(--text);
    background: var(--bg);
    overflow: hidden;
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

  .app-layout {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .toast-error {
    position: fixed;
    bottom: 16px;
    right: 16px;
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

  .toast-error span {
    flex: 1;
    word-break: break-word;
  }

  .toast-error button {
    background: none;
    border: none;
    color: #fca5a5;
    cursor: pointer;
    font-size: 14px;
    padding: 0;
    flex-shrink: 0;
  }
</style>
