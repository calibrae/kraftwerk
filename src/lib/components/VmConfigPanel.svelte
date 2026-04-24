<script>
  import { invoke } from "@tauri-apps/api/core";
  import { getState, refreshVms } from "$lib/stores/app.svelte.js";

  let { vmName } = $props();
  const appState = getState();

  let config = $state(null);
  let loading = $state(false);
  let error = $state(null);
  let busy = $state(false);

  // Edit state
  let editVcpus = $state(0);
  let editMemoryMb = $state(0);        // current (balloon target)
  let editMaxMemoryMb = $state(0);     // max (boot-time memory)

  async function load() {
    if (!vmName) return;
    loading = true;
    error = null;
    try {
      config = await invoke("get_domain_config", { name: vmName, inactive: false });
      editVcpus = config.vcpus.current;
      editMemoryMb = Math.floor(config.current_memory.kib / 1024);
      editMaxMemoryMb = Math.floor(config.memory.kib / 1024);
    } catch (e) {
      error = e?.message || String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (vmName) load();
  });

  async function applyVcpus() {
    if (!vmName || busy) return;
    busy = true;
    error = null;
    try {
      const running = appState.selectedVm?.state === "running";
      await invoke("set_vcpus", {
        name: vmName,
        count: editVcpus,
        live: running,
        config: true,
      });
      await load();
      await refreshVms();
    } catch (e) {
      error = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  async function applyMemory() {
    if (!vmName || busy) return;
    busy = true;
    error = null;
    try {
      const running = appState.selectedVm?.state === "running";
      const oldMaxMb = Math.floor(config.memory.kib / 1024);
      // Either the user bumped the max explicitly, or set current above the
      // old max (which would otherwise be rejected by libvirt). In both cases,
      // push the max out first via config-only set_max_memory_mb.
      const targetMaxMb = Math.max(editMaxMemoryMb, editMemoryMb);
      if (targetMaxMb !== oldMaxMb) {
        await invoke("set_max_memory_mb", { name: vmName, memoryMb: targetMaxMb });
      }
      if (editMemoryMb * 1024 !== config.current_memory.kib) {
        await invoke("set_memory_mb", {
          name: vmName,
          memoryMb: editMemoryMb,
          live: running,
          config: true,
        });
      }
      await load();
      await refreshVms();
    } catch (e) {
      error = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  function formatKib(kib) {
    if (kib >= 1024 * 1024) return `${(kib / 1024 / 1024).toFixed(1)} GiB`;
    if (kib >= 1024) return `${(kib / 1024).toFixed(0)} MiB`;
    return `${kib} KiB`;
  }
</script>

<div class="config-panel">
  {#if loading && !config}
    <p class="muted">Loading configuration...</p>
  {:else if error && !config}
    <div class="error">{error}</div>
  {:else if config}
    {#if error}
      <div class="error">{error}</div>
    {/if}

    <section>
      <h3>General</h3>
      <dl class="info-list">
        <dt>Name</dt><dd>{config.name}</dd>
        <dt>UUID</dt><dd class="mono">{config.uuid}</dd>
        {#if config.title}<dt>Title</dt><dd>{config.title}</dd>{/if}
        {#if config.description}<dt>Description</dt><dd>{config.description}</dd>{/if}
      </dl>
    </section>

    <section>
      <h3>CPU</h3>
      <dl class="info-list">
        <dt>Mode</dt><dd>{config.cpu.mode || "(default)"}</dd>
        {#if config.cpu.model}<dt>Model</dt><dd>{config.cpu.model}</dd>{/if}
        {#if config.cpu.sockets}
          <dt>Topology</dt><dd>{config.cpu.sockets} sockets × {config.cpu.cores} cores × {config.cpu.threads} threads</dd>
        {/if}
        <dt>Maximum vCPUs</dt><dd>{config.vcpus.max}</dd>
      </dl>

      <div class="edit-row">
        <label>
          <span>Active vCPUs</span>
          <input type="number" min="1" max={config.vcpus.max} bind:value={editVcpus} disabled={busy} />
        </label>
        <button class="btn-apply" onclick={applyVcpus} disabled={busy || editVcpus === config.vcpus.current || editVcpus < 1 || editVcpus > config.vcpus.max}>
          {busy ? "Applying..." : "Apply"}
        </button>
      </div>
    </section>

    <section>
      <h3>Memory</h3>
      <dl class="info-list">
        <dt>Maximum</dt><dd>{formatKib(config.memory.kib)}</dd>
        <dt>Current</dt><dd>{formatKib(config.current_memory.kib)}</dd>
      </dl>

      <div class="edit-row">
        <label>
          <span>Current (MiB)</span>
          <input type="number" min="128" step="128" bind:value={editMemoryMb} disabled={busy} />
        </label>
        <label>
          <span>Max (MiB)</span>
          <input type="number" min="128" step="128" bind:value={editMaxMemoryMb} disabled={busy} />
        </label>
        <button class="btn-apply" onclick={applyMemory} disabled={busy || (editMemoryMb * 1024 === config.current_memory.kib && editMaxMemoryMb * 1024 === config.memory.kib)}>
          {busy ? "Applying..." : "Apply"}
        </button>
      </div>
      <p class="hint">Current must be &le; Max. If Current exceeds Max the Max is bumped automatically. Max changes only take effect on next boot.</p>
    </section>

    <section>
      <h3>Boot & Firmware</h3>
      <dl class="info-list">
        <dt>Firmware</dt><dd class="caps">{config.os.firmware}</dd>
        {#if config.os.machine}<dt>Machine</dt><dd>{config.os.machine}</dd>{/if}
        {#if config.os.arch}<dt>Architecture</dt><dd>{config.os.arch}</dd>{/if}
        {#if config.os.boot_order.length > 0}
          <dt>Boot Order</dt><dd>{config.os.boot_order.join(" → ")}</dd>
        {/if}
      </dl>
    </section>
  {/if}
</div>

<style>
  .config-panel {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  section {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px;
  }

  h3 {
    margin: 0 0 12px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .info-list {
    display: grid;
    grid-template-columns: 160px 1fr;
    gap: 8px 16px;
    margin: 0;
    font-size: 13px;
  }

  dt {
    color: var(--text-muted);
    font-weight: 500;
  }

  dd {
    margin: 0;
    word-break: break-all;
  }

  .mono { font-family: 'SF Mono', 'Fira Code', monospace; font-size: 12px; }
  .caps { text-transform: uppercase; }

  .edit-row {
    display: flex;
    gap: 12px;
    align-items: flex-end;
    margin-top: 14px;
    padding-top: 14px;
    border-top: 1px solid var(--border);
  }

  .edit-row label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
  }

  .edit-row label span {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  input[type="number"] {
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text);
    font-size: 13px;
    font-family: inherit;
    outline: none;
    width: 140px;
  }

  input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-dim);
  }

  .btn-apply {
    padding: 7px 16px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    background: var(--accent);
    color: white;
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
  }

  .btn-apply:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-apply:disabled { opacity: 0.4; cursor: not-allowed; }

  .hint {
    margin: 8px 0 0;
    font-size: 11px;
    color: var(--text-muted);
  }

  .error {
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    color: #ef4444;
    font-size: 12px;
  }

  .muted {
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
