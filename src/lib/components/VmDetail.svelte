<script>
  import { getState, startDomain, shutdownDomain, destroyDomain, suspendDomain, resumeDomain, rebootDomain, getDomainXml } from "$lib/stores/app.svelte.js";
  import SerialConsole from "./SerialConsole.svelte";
  import SnapshotsPanel from "./SnapshotsPanel.svelte";
  import RawXmlPanel from "./RawXmlPanel.svelte";
  import MetricsGraphs from "./MetricsGraphs.svelte";
  import QemuLogPanel from "./QemuLogPanel.svelte";
  import HypervisorDashboard from "./HypervisorDashboard.svelte";
  import BulkVmActions from "./BulkVmActions.svelte";
  import CloneVmDialog from "./CloneVmDialog.svelte";
  import VmConfigPanel from "./VmConfigPanel.svelte";
  import HardwarePanel from "./HardwarePanel.svelte";
  import BootPanel from "./BootPanel.svelte";
  import DisksPanel from "./DisksPanel.svelte";
  import NicsPanel from "./NicsPanel.svelte";
  import DisplayPanel from "./DisplayPanel.svelte";
  import VirtioPanel from "./VirtioPanel.svelte";
  import CharDevicesPanel from "./CharDevicesPanel.svelte";
  import FilesystemPanel from "./FilesystemPanel.svelte";
  import ControllersPanel from "./ControllersPanel.svelte";
  import CpuTunePanel from "./CpuTunePanel.svelte";
  import VmOverview from "./VmOverview.svelte";

  const appState = getState();

  let domainXml = $state(null);
  let showXml = $state(false);
  let loadingXml = $state(false);
  let showConsole = $state(false);
  let showVnc = $state(false);
  let VncConsole = $state(null);
  let loadingVnc = $state(false);
  let SpiceConsole = $state(null);
  let loadingSpice = $state(false);
  let showSpice = $state(false);
  let activeTab = $state("overview"); // "overview" | "config"
  let showClone = $state(false);

  // Tear down any open console when switching VMs — otherwise the old
  // SpiceConsole/VncConsole/SerialConsole component stays alive with the
  // previous VM's SSH tunnel + graphics channel.
  $effect(() => {
    // Read selectedVm.name so the effect re-runs on VM change.
    const _name = appState.selectedVm?.name;
    showSpice = false;
    showVnc = false;
    showConsole = false;
  });

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
    if (!appState.selectedVm) return;
    loadingXml = true;
    domainXml = await getDomainXml(appState.selectedVm.name);
    loadingXml = false;
    showXml = true;
  }

  function formatMemory(mb) {
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${mb} MB`;
  }

  function closeConsole() {
    showConsole = false;
  }

  function closeVnc() {
    showVnc = false;
  }

  function closeSpice() {
    showSpice = false;
  }
</script>

<div class="detail">
  {#if appState.hasMultiSelect}
    <BulkVmActions />
  {:else if showSpice && SpiceConsole && appState.selectedVm}
    {#key appState.selectedVm.name}
      <SpiceConsole vmName={appState.selectedVm.name} onClose={closeSpice} />
    {/key}
  {:else if showVnc && VncConsole && appState.selectedVm}
    {#key appState.selectedVm.name}
      <VncConsole vmName={appState.selectedVm.name} onClose={closeVnc} />
    {/key}
  {:else if showConsole && appState.selectedVm}
    {#key appState.selectedVm.name}
      <SerialConsole vmName={appState.selectedVm.name} onClose={closeConsole} />
    {/key}
  {:else if !appState.selectedVm}
    {#if appState.isConnected}
      {#key appState.selectedConnectionId}
        <HypervisorDashboard />
      {/key}
    {:else}
      <div class="empty-detail">
        <p>Connect to a hypervisor to get started</p>
      </div>
    {/if}
  {:else}
    {@const vm = appState.selectedVm}
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
        {#if vm.state === "shut_off"}
          <button class="btn-action" onclick={() => showClone = true}>Clone</button>
        {/if}
        {#if vm.has_serial && vm.state === "running"}
          <button class="btn-action console" onclick={() => showConsole = true}>Serial Console</button>
        {/if}
        {#if vm.graphics_type === "vnc" && vm.state === "running"}
          <button class="btn-action console" onclick={async () => {
            if (!VncConsole) {
              loadingVnc = true;
              try { VncConsole = (await import("./VncConsole.svelte")).default; } catch (e) { console.error(e); }
              loadingVnc = false;
            }
            showVnc = true;
          }} disabled={loadingVnc}>{loadingVnc ? "Loading VNC..." : "VNC Console"}</button>
        {/if}
        {#if vm.graphics_type === "spice" && vm.state === "running"}
          <button class="btn-action console" onclick={async () => {
            if (!SpiceConsole) {
              loadingSpice = true;
              try { SpiceConsole = (await import("./SpiceConsole.svelte")).default; } catch (e) { console.error(e); }
              loadingSpice = false;
            }
            showSpice = true;
          }} disabled={loadingSpice}>{loadingSpice ? "Loading SPICE..." : "SPICE Console"}</button>
        {/if}
      </div>
    </div>

    <div class="tabs-layout">
    <div class="tab-bar">
      <button class="tab" class:active={activeTab === "overview"} onclick={() => activeTab = "overview"}>Overview</button>
      <button class="tab" class:active={activeTab === "graphs"} onclick={() => activeTab = "graphs"}>Graphs</button>
      <button class="tab" class:active={activeTab === "config"} onclick={() => activeTab = "config"}>Configuration</button>
      <button class="tab" class:active={activeTab === "hardware"} onclick={() => activeTab = "hardware"}>Hardware</button>
      <button class="tab" class:active={activeTab === "disks"} onclick={() => activeTab = "disks"}>Disks</button>
      <button class="tab" class:active={activeTab === "network"} onclick={() => activeTab = "network"}>Network</button>
      <button class="tab" class:active={activeTab === "boot"} onclick={() => activeTab = "boot"}>Boot</button>
      <button class="tab" class:active={activeTab === "display"} onclick={() => activeTab = "display"}>Display</button>
      <button class="tab" class:active={activeTab === "devices"} onclick={() => activeTab = "devices"}>Devices</button>
      <button class="tab" class:active={activeTab === "comms"} onclick={() => activeTab = "comms"}>Communication</button>
      <button class="tab" class:active={activeTab === "filesystems"} onclick={() => activeTab = "filesystems"}>Filesystems</button>
      <button class="tab" class:active={activeTab === "controllers"} onclick={() => activeTab = "controllers"}>Controllers</button>
      <button class="tab" class:active={activeTab === "tuning"} onclick={() => activeTab = "tuning"}>Tuning</button>
      <button class="tab" class:active={activeTab === "snapshots"} onclick={() => activeTab = "snapshots"}>Snapshots</button>
      <button class="tab" class:active={activeTab === "log"} onclick={() => activeTab = "log"}>Log</button>
      <button class="tab" class:active={activeTab === "xml"} onclick={() => activeTab = "xml"}>XML</button>
    </div>

    <div class="tab-content">
      {#if activeTab === "overview"}
        <VmOverview vmName={vm.name} running={vm.state === "running"} />
      {:else if activeTab === "graphs"}
        <MetricsGraphs vmName={vm.name} />
      {:else if activeTab === "config"}
        <VmConfigPanel vmName={vm.name} />
      {:else if activeTab === "hardware"}
        <HardwarePanel vmName={vm.name} />
      {:else if activeTab === "disks"}
        <DisksPanel vmName={vm.name} />
      {:else if activeTab === "network"}
        <NicsPanel vmName={vm.name} />
      {:else if activeTab === "boot"}
        <BootPanel vmName={vm.name} />
      {:else if activeTab === "display"}
        <DisplayPanel vmName={vm.name} />
      {:else if activeTab === "devices"}
        <VirtioPanel vmName={vm.name} />
      {:else if activeTab === "comms"}
        <CharDevicesPanel vmName={vm.name} />
      {:else if activeTab === "filesystems"}
        <FilesystemPanel vmName={vm.name} />
      {:else if activeTab === "controllers"}
        <ControllersPanel vmName={vm.name} />
      {:else if activeTab === "tuning"}
        <CpuTunePanel vmName={vm.name} />
      {:else if activeTab === "snapshots"}
        <SnapshotsPanel vmName={vm.name} />
      {:else if activeTab === "log"}
        <QemuLogPanel vmName={vm.name} />
      {:else if activeTab === "xml"}
        <RawXmlPanel vmName={vm.name} />
      {/if}
    </div>
    </div>
  {/if}
  <CloneVmDialog bind:open={showClone} source={appState.selectedVm} />
</div>

<style>
  .detail {
    flex: 1;
    padding: 0;
    overflow-y: auto;
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  .detail > :not(.empty-detail):not(:global(.console-container)):not(.tabs-layout) {
    padding: 0 24px;
  }

  .detail > :first-child {
    padding-top: 24px;
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
    padding: 24px 24px 0;
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
    padding: 0 24px;
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
    padding: 0 24px;
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
  .btn-action.console { background: #1e3a5f; color: #93c5fd; border-color: #1e3a5f; }
  .btn-action.console:hover { background: #1e40af; }


  .tabs-layout {
    display: flex;
    flex: 1;
    min-height: 0;
    gap: 0;
  }

  .tab-bar {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 160px;
    flex-shrink: 0;
    padding: 8px 8px 8px 24px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }

  .tab {
    padding: 7px 12px;
    background: transparent;
    border: none;
    border-left: 2px solid transparent;
    color: var(--text-muted);
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
    text-align: left;
    border-radius: 0 6px 6px 0;
  }

  .tab:hover {
    color: var(--text);
    background: var(--bg-hover);
  }

  .tab.active {
    color: var(--text);
    background: var(--bg-surface);
    border-left-color: var(--accent);
    font-weight: 500;
  }

  .tab-content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 0 24px 24px;
  }

  .vm-xml-section {
    padding: 0 24px 24px;
  }

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
