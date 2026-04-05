<script>
  import { getState, startDomain, shutdownDomain, destroyDomain, suspendDomain, resumeDomain, rebootDomain, getDomainXml } from "$lib/stores/app.svelte.js";

  const state = getState();

  let domainXml = $state(null);
  let showXml = $state(false);
  let loadingXml = $state(false);

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

  function canStart(s) { return s === "shut_off" || s === "crashed"; }
  function canShutdown(s) { return s === "running"; }
  function canForceOff(s) { return ["running", "paused", "crashed", "suspended"].includes(s); }
  function canPause(s) { return s === "running"; }
  function canResume(s) { return s === "paused" || s === "suspended"; }
  function canReboot(s) { return s === "running"; }

  async function loadXml() {
    if (!state.selectedVm) return;
    loadingXml = true;
    domainXml = await getDomainXml(state.selectedVm.name);
    loadingXml = false;
    showXml = true;
  }

  function formatMemory(mb) {
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${mb} MB`;
  }
</script>

<div class="detail">
  {#if !state.selectedVm}
    <div class="empty-detail">
      {#if state.isConnected}
        <p>Select a VM from the sidebar</p>
      {:else}
        <p>Connect to a hypervisor to get started</p>
      {/if}
    </div>
  {:else}
    {@const vm = state.selectedVm}
    <div class="vm-header">
      <div class="vm-title-row">
        <span class="vm-state-badge" style="background: {stateColors[vm.state]}">
          {stateLabels[vm.state] ?? vm.state}
        </span>
        <h2>{vm.name}</h2>
      </div>
      <p class="vm-uuid">{vm.uuid}</p>
    </div>

    <div class="vm-info-grid">
      <div class="info-card">
        <span class="info-label">vCPUs</span>
        <span class="info-value">{vm.vcpus}</span>
      </div>
      <div class="info-card">
        <span class="info-label">Memory</span>
        <span class="info-value">{formatMemory(vm.memory_mb)}</span>
      </div>
      <div class="info-card">
        <span class="info-label">Graphics</span>
        <span class="info-value">{vm.graphics_type?.toUpperCase() ?? "None"}</span>
      </div>
      <div class="info-card">
        <span class="info-label">Serial</span>
        <span class="info-value">{vm.has_serial ? "Available" : "None"}</span>
      </div>
    </div>

    <div class="vm-actions">
      <h3>Actions</h3>
      <div class="action-row">
        {#if canStart(vm.state)}
          <button class="btn-action start" onclick={() => startDomain(vm.name)}>Start</button>
        {/if}
        {#if canShutdown(vm.state)}
          <button class="btn-action" onclick={() => shutdownDomain(vm.name)}>Shutdown</button>
        {/if}
        {#if canPause(vm.state)}
          <button class="btn-action" onclick={() => suspendDomain(vm.name)}>Pause</button>
        {/if}
        {#if canResume(vm.state)}
          <button class="btn-action start" onclick={() => resumeDomain(vm.name)}>Resume</button>
        {/if}
        {#if canReboot(vm.state)}
          <button class="btn-action" onclick={() => rebootDomain(vm.name)}>Reboot</button>
        {/if}
        {#if canForceOff(vm.state)}
          <button class="btn-action danger" onclick={() => destroyDomain(vm.name)}>Force Off</button>
        {/if}
      </div>
    </div>

    <div class="vm-xml-section">
      <div class="xml-header">
        <h3>Domain XML</h3>
        <button class="btn-small" onclick={loadXml} disabled={loadingXml}>
          {loadingXml ? "Loading..." : showXml ? "Refresh" : "Show XML"}
        </button>
      </div>
      {#if showXml && domainXml}
        <pre class="xml-content">{domainXml}</pre>
      {/if}
    </div>
  {/if}
</div>

<style>
  .detail {
    flex: 1;
    padding: 24px;
    overflow-y: auto;
    height: 100vh;
  }

  .empty-detail {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
    font-size: 14px;
  }

  .vm-header {
    margin-bottom: 24px;
  }

  .vm-title-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .vm-title-row h2 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
  }

  .vm-state-badge {
    display: inline-block;
    padding: 2px 10px;
    border-radius: 12px;
    font-size: 11px;
    font-weight: 600;
    color: #111;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .vm-uuid {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--text-muted);
    font-family: monospace;
  }

  .vm-info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 12px;
    margin-bottom: 24px;
  }

  .info-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .info-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .info-value {
    font-size: 18px;
    font-weight: 600;
  }

  .vm-actions {
    margin-bottom: 24px;
  }

  .vm-actions h3, .xml-header h3 {
    margin: 0 0 12px;
    font-size: 14px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .action-row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .btn-action {
    padding: 8px 16px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
  }

  .btn-action:hover { background: var(--bg-hover); }
  .btn-action.start { background: #065f46; color: #34d399; border-color: #065f46; }
  .btn-action.start:hover { background: #047857; }
  .btn-action.danger { background: #7f1d1d; color: #fca5a5; border-color: #7f1d1d; }
  .btn-action.danger:hover { background: #991b1b; }

  .xml-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .btn-small {
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    cursor: pointer;
    font-size: 12px;
    font-family: inherit;
  }

  .btn-small:hover { background: var(--bg-hover); }
  .btn-small:disabled { opacity: 0.5; cursor: not-allowed; }

  .xml-content {
    background: var(--bg-sidebar);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px;
    font-size: 12px;
    line-height: 1.5;
    overflow-x: auto;
    white-space: pre;
    font-family: 'SF Mono', 'Fira Code', monospace;
    max-height: 400px;
    overflow-y: auto;
  }
</style>
