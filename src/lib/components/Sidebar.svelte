<script>
  import { getState, connect, disconnect, selectVm, removeConnection, refreshVms } from "$lib/stores/app.svelte.js";

  let { onAddConnection = () => {}, onEditConnection = () => {}, onConnect = null } = $props();
  const appState = getState();

  const stateColors = {
    running: "#34d399",
    paused: "#fbbf24",
    shut_off: "#6b7280",
    crashed: "#ef4444",
    suspended: "#a78bfa",
    unknown: "#9ca3af",
  };

  const stateLabels = {
    running: "Running",
    paused: "Paused",
    shut_off: "Shut Off",
    crashed: "Crashed",
    suspended: "Suspended",
    unknown: "Unknown",
  };

  function connectionStatus(id) {
    return appState.connectionStates[id]?.status ?? "disconnected";
  }

  // Right-click context menu state.
  let menuFor = $state(null);       // connection id whose menu is open, or null
  let menuX = $state(0);
  let menuY = $state(0);

  function openMenu(ev, conn) {
    ev.preventDefault();
    menuFor = conn.id;
    menuX = ev.clientX;
    menuY = ev.clientY;
  }

  function closeMenu() { menuFor = null; }

  function editFromMenu(conn) {
    closeMenu();
    onEditConnection(conn);
  }

  async function handleConnect(id) {
    if (onConnect) await onConnect(id);
    else await connect(id);
  }

  async function handleDisconnect(id) {
    await disconnect(id);
  }
</script>

<aside class="sidebar">
  <div class="sidebar-header">
    <h2>Connections</h2>
    <button class="btn-icon" onclick={onAddConnection} title="Add connection">+</button>
  </div>

  {#if appState.savedConnections.length === 0}
    <div class="empty">
      <p>No connections</p>
      <button class="btn-small" onclick={onAddConnection}>Add one</button>
    </div>
  {:else}
    <ul class="connection-list">
      {#each appState.savedConnections as conn (conn.id)}
        {@const status = connectionStatus(conn.id)}
        <li class="connection-item" class:current={appState.selectedConnectionId === conn.id}>
          <div
            class="connection-header"
            role="button"
            tabindex="0"
            onclick={() => handleConnect(conn.id)}
            onkeydown={(e) => (e.key === "Enter" || e.key === " ") && handleConnect(conn.id)}
            oncontextmenu={(e) => openMenu(e, conn)}
          >
            <span class="status-dot" class:connected={status === "connected"} class:connecting={status === "connecting"} class:error={status === "error"}></span>
            <span class="connection-name">{conn.display_name}</span>
            <div class="connection-actions" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="toolbar" tabindex="-1">
              {#if status === "connected"}
                <button class="btn-tiny" onclick={() => refreshVms()} title="Refresh">&#8635;</button>
                <button class="btn-tiny" onclick={() => handleDisconnect(conn.id)} title="Disconnect">&#x2715;</button>
              {:else if status === "connecting"}
                <span class="spinner"></span>
              {:else}
                <button class="btn-tiny" onclick={() => handleConnect(conn.id)} title="Connect">&#9654;</button>
                <button class="btn-tiny danger" onclick={() => removeConnection(conn.id)} title="Remove">&#128465;</button>
              {/if}
            </div>
          </div>

          {#if status === "connected" && appState.selectedConnectionId === conn.id}
            <ul class="vm-list">
              {#each appState.vms as vm (vm.name)}
                <li
                  class="vm-item"
                  class:selected={appState.selectedVmName === vm.name}
                  class:multi-selected={appState.selectedVmNames.has(vm.name) && appState.selectedVmName !== vm.name}
                  onclick={(e) => selectVm(vm.name, e)}
                  role="button"
                  tabindex="0"
                  onkeydown={(e) => e.key === 'Enter' && selectVm(vm.name, e)}
                >
                  <span class="vm-state-dot" style="background: {stateColors[vm.state] ?? '#9ca3af'}"></span>
                  <span class="vm-name">{vm.name}</span>
                  <span class="vm-state-label">{stateLabels[vm.state] ?? vm.state}</span>
                </li>
              {:else}
                <li class="vm-empty">No VMs found</li>
              {/each}
            </ul>
          {/if}

          {#if status === "error"}
            <div class="connection-error">
              {appState.connectionStates[conn.id]?.message ?? "Connection failed"}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</aside>

{#if menuFor}
  <!-- Backdrop catches click-outside to close -->
  <div class="menu-backdrop" onclick={closeMenu} onkeydown={(e) => e.key === "Escape" && closeMenu()} role="presentation"></div>
  <div class="ctx-menu" style="left: {menuX}px; top: {menuY}px;" role="menu">
    {#each appState.savedConnections.filter(c => c.id === menuFor) as conn (conn.id)}
      <button class="ctx-item" onclick={() => editFromMenu(conn)}>Edit / Properties</button>
      <button class="ctx-item danger" onclick={() => { closeMenu(); removeConnection(conn.id); }}>Remove</button>
    {/each}
  </div>
{/if}


<style>
  .sidebar {
    width: 260px;
    min-width: 260px;
    height: 100vh;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .sidebar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px;
    border-bottom: 1px solid var(--border);
  }

  .sidebar-header h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
  }

  .btn-icon {
    width: 28px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 16px;
    font-weight: 500;
  }

  .btn-icon:hover { background: var(--bg-hover); }

  .empty {
    padding: 24px 16px;
    text-align: center;
    color: var(--text-muted);
  }

  .empty p { margin: 0 0 12px; font-size: 13px; }

  .btn-small {
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    cursor: pointer;
    font-size: 12px;
  }

  .btn-small:hover { background: var(--bg-hover); }

  .connection-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .connection-item {
    border-bottom: 1px solid var(--border);
  }

  .connection-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-muted);
    flex-shrink: 0;
  }

  .status-dot.connected { background: #34d399; }
  .status-dot.connecting { background: #fbbf24; animation: pulse 1s infinite; }
  .status-dot.error { background: #ef4444; }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .connection-name {
    flex: 1;
    font-size: 13px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .connection-actions {
    display: flex;
    gap: 4px;
    align-items: center;
  }

  .btn-tiny {
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    padding: 0;
  }

  .btn-tiny:hover { background: var(--bg-hover); color: var(--text); }
  .btn-tiny.danger:hover { color: #ef4444; }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--border);
    border-top-color: var(--text);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .vm-list {
    list-style: none;
    margin: 0;
    padding: 0 0 4px;
  }

  .vm-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 16px 6px 32px;
    cursor: pointer;
    font-size: 13px;
    border-radius: 4px;
    margin: 0 8px;
  }

  .vm-item:hover { background: var(--bg-hover); }
  .vm-item.selected { background: var(--bg-selected); }
  .vm-item.multi-selected {
    background: rgba(96, 165, 250, 0.10);
    border-left: 2px solid #60a5fa;
    padding-left: 6px;
  }

  .vm-state-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .vm-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .vm-state-label {
    font-size: 11px;
    color: var(--text-muted);
  }

  .vm-empty {
    padding: 8px 32px;
    font-size: 12px;
    color: var(--text-muted);
  }

  .connection-error {
    padding: 4px 16px 8px 32px;
    font-size: 11px;
    color: #ef4444;
  }

  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 200;
  }
  .ctx-menu {
    position: fixed;
    z-index: 201;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 4px;
    min-width: 160px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
    display: flex;
    flex-direction: column;
  }
  .ctx-item {
    padding: 6px 12px;
    background: transparent;
    color: var(--text);
    border: none;
    text-align: left;
    font-size: 13px;
    font-family: inherit;
    border-radius: 4px;
    cursor: pointer;
  }
  .ctx-item:hover { background: var(--bg-hover); }
  .ctx-item.danger { color: #ef4444; }
</style>
