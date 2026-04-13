<script>
  import { getState, refreshPools, refreshVolumes, startPool, stopPool, deletePool, refreshPoolVolumes, setPoolAutostart, deleteVolume } from "$lib/stores/app.svelte.js";

  let { onCreatePool, onCreateVolume } = $props();
  const appState = getState();

  let selectedPool = $state(null);
  let busy = $state(false);

  const typeColors = {
    dir: "#6366f1",
    netfs: "#10b981",
    logical: "#f59e0b",
    iscsi: "#ef4444",
  };

  async function selectPool(name) {
    selectedPool = name;
    if (!appState.volumesByPool[name]) {
      await refreshVolumes(name);
    }
  }

  async function confirmDeletePool(name) {
    if (!confirm(`Delete pool "${name}"? Volumes inside are NOT deleted from disk, but the pool definition is removed.`)) return;
    busy = true;
    await deletePool(name);
    if (selectedPool === name) selectedPool = null;
    busy = false;
  }

  async function confirmDeleteVolume(poolName, vol) {
    if (!confirm(`Delete volume "${vol.name}"? This removes the file from disk.`)) return;
    busy = true;
    await deleteVolume(poolName, vol.path);
    busy = false;
  }

  function formatBytes(b) {
    if (b === 0) return "—";
    if (b >= 1e12) return `${(b / 1e12).toFixed(2)} TB`;
    if (b >= 1e9) return `${(b / 1e9).toFixed(2)} GB`;
    if (b >= 1e6) return `${(b / 1e6).toFixed(1)} MB`;
    if (b >= 1e3) return `${(b / 1e3).toFixed(1)} KB`;
    return `${b} B`;
  }

  function usagePct(pool) {
    if (!pool.capacity) return 0;
    return Math.min(100, (pool.allocation / pool.capacity) * 100);
  }

  let selected = $derived(selectedPool ? appState.pools.find(p => p.name === selectedPool) ?? null : null);
  let volumes = $derived(selectedPool ? (appState.volumesByPool[selectedPool] ?? []) : []);
</script>

<div class="storage-view">
  <div class="header">
    <h2>Storage Pools</h2>
    <div class="actions">
      <button class="btn" onclick={refreshPools} disabled={busy}>Refresh</button>
      <button class="btn btn-primary" onclick={onCreatePool}>+ New Pool</button>
    </div>
  </div>

  {#if appState.pools.length === 0}
    <div class="empty">
      <p>No storage pools on this hypervisor.</p>
      <button class="btn btn-primary" onclick={onCreatePool}>Create one</button>
    </div>
  {:else}
    <div class="layout">
      <div class="pool-list">
        <table>
          <thead>
            <tr>
              <th></th>
              <th>Name</th>
              <th>Type</th>
              <th>Usage</th>
              <th>Capacity</th>
              <th>Vols</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each appState.pools as pool (pool.name)}
              <tr class:selected={selectedPool === pool.name} onclick={() => selectPool(pool.name)}>
                <td>
                  <span class="state-dot" class:active={pool.is_active}></span>
                </td>
                <td class="name">{pool.name}</td>
                <td>
                  <span class="type-badge" style="background: {typeColors[pool.pool_type] ?? '#6b7280'}">
                    {pool.pool_type}
                  </span>
                </td>
                <td class="usage-cell">
                  <div class="usage-bar">
                    <div class="usage-fill" style="width: {usagePct(pool)}%"></div>
                  </div>
                  <span class="usage-text">{formatBytes(pool.allocation)} / {formatBytes(pool.capacity)}</span>
                </td>
                <td class="mono">{formatBytes(pool.available)} free</td>
                <td>{pool.num_volumes}</td>
                <td class="row-actions">
                  {#if pool.is_active}
                    <button class="btn-tiny" onclick={(e) => { e.stopPropagation(); stopPool(pool.name); }}>Stop</button>
                  {:else}
                    <button class="btn-tiny start" onclick={(e) => { e.stopPropagation(); startPool(pool.name); }}>Start</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

      {#if selected}
        <div class="detail">
          <div class="detail-header">
            <h3>{selected.name}</h3>
            <div class="detail-actions">
              {#if selected.is_active}
                <button class="btn-sm" onclick={() => refreshPoolVolumes(selected.name)}>Rescan</button>
                <button class="btn-sm btn-primary" onclick={() => onCreateVolume(selected.name)}>+ Volume</button>
              {/if}
              <button class="btn-sm btn-danger" onclick={() => confirmDeletePool(selected.name)}>Delete Pool</button>
            </div>
          </div>

          <dl class="info">
            <dt>Type</dt><dd>{selected.pool_type}</dd>
            <dt>State</dt><dd>{selected.is_active ? "Active" : "Inactive"}</dd>
            {#if selected.target_path}<dt>Path</dt><dd class="mono">{selected.target_path}</dd>{/if}
            <dt>Capacity</dt><dd>{formatBytes(selected.capacity)}</dd>
            <dt>Used</dt><dd>{formatBytes(selected.allocation)} ({usagePct(selected).toFixed(1)}%)</dd>
            <dt>Available</dt><dd>{formatBytes(selected.available)}</dd>
            <dt>Autostart</dt>
            <dd>
              <label class="switch">
                <input type="checkbox" checked={selected.autostart}
                  onchange={(e) => setPoolAutostart(selected.name, e.target.checked)} />
                <span>{selected.autostart ? "On" : "Off"}</span>
              </label>
            </dd>
          </dl>

          <h4>Volumes ({volumes.length})</h4>
          {#if volumes.length === 0}
            <div class="vol-empty">
              {selected.is_active ? "No volumes in this pool." : "Pool is inactive — start it to see volumes."}
            </div>
          {:else}
            <table class="vol-table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Format</th>
                  <th>Capacity</th>
                  <th>Used</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {#each volumes as vol (vol.path)}
                  <tr>
                    <td class="name mono">{vol.name}</td>
                    <td><span class="fmt-badge">{vol.format}</span></td>
                    <td>{formatBytes(vol.capacity)}</td>
                    <td>{formatBytes(vol.allocation)}</td>
                    <td class="row-actions">
                      <button class="btn-tiny danger" onclick={() => confirmDeleteVolume(selected.name, vol)}>Delete</button>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .storage-view { padding: 24px; height: 100vh; overflow-y: auto; }
  .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
  .header h2 { margin: 0; font-size: 20px; font-weight: 600; }
  .actions { display: flex; gap: 8px; }

  .btn, .btn-sm {
    border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-family: inherit; cursor: pointer;
  }
  .btn { padding: 7px 14px; font-size: 13px; }
  .btn-sm { padding: 4px 10px; font-size: 12px; }
  .btn:hover, .btn-sm:hover { background: var(--bg-hover); }
  .btn-primary, .btn-sm.btn-primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn-primary:hover, .btn-sm.btn-primary:hover { filter: brightness(1.1); }
  .btn-danger, .btn-sm.btn-danger { background: #7f1d1d; border-color: #7f1d1d; color: #fca5a5; }
  .btn-danger:hover, .btn-sm.btn-danger:hover { background: #991b1b; }

  .empty { text-align: center; padding: 60px 20px; color: var(--text-muted); }
  .empty p { margin: 0 0 16px; }

  .layout { display: grid; grid-template-columns: 1fr 400px; gap: 20px; }

  .pool-list { background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
  table { width: 100%; border-collapse: collapse; }
  thead th { text-align: left; padding: 10px 12px; font-size: 11px; text-transform: uppercase;
    color: var(--text-muted); letter-spacing: 0.05em; background: var(--bg-sidebar); font-weight: 600; }
  tbody tr { cursor: pointer; border-top: 1px solid var(--border); }
  tbody tr:hover { background: var(--bg-hover); }
  tbody tr.selected { background: var(--bg-selected); }
  td { padding: 10px 12px; font-size: 13px; vertical-align: middle; }
  .name { font-weight: 500; }
  .mono { font-family: 'SF Mono', 'Fira Code', monospace; font-size: 12px; }

  .state-dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: #6b7280; }
  .state-dot.active { background: #34d399; }

  .type-badge, .fmt-badge {
    display: inline-block; padding: 2px 8px; border-radius: 10px;
    font-size: 10px; font-weight: 600; color: white; text-transform: uppercase;
  }
  .fmt-badge { background: #4b5563; }

  .usage-cell { min-width: 180px; }
  .usage-bar { height: 6px; background: var(--bg-sidebar); border-radius: 3px; overflow: hidden; }
  .usage-fill { height: 100%; background: var(--accent); transition: width 0.3s; }
  .usage-text { font-size: 11px; color: var(--text-muted); font-family: 'SF Mono', monospace; }

  .row-actions { text-align: right; }
  .btn-tiny { padding: 2px 8px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--bg-button); color: var(--text); font-size: 11px; cursor: pointer; }
  .btn-tiny:hover { background: var(--bg-hover); }
  .btn-tiny.start { background: #065f46; color: #34d399; border-color: #065f46; }
  .btn-tiny.start:hover { background: #047857; }
  .btn-tiny.danger { color: #fca5a5; }
  .btn-tiny.danger:hover { background: #7f1d1d; border-color: #7f1d1d; }

  .detail { background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; padding: 16px; }
  .detail-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; }
  .detail-header h3 { margin: 0; font-size: 15px; font-weight: 600; }
  .detail-actions { display: flex; gap: 6px; flex-wrap: wrap; }

  .info { display: grid; grid-template-columns: 90px 1fr; gap: 5px 12px; margin: 0 0 16px; font-size: 12px; }
  dt { color: var(--text-muted); }
  dd { margin: 0; word-break: break-all; }

  .switch { display: flex; align-items: center; gap: 8px; cursor: pointer; }
  .switch input { margin: 0; }

  h4 { margin: 12px 0 8px; font-size: 12px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; font-weight: 600; }

  .vol-empty { padding: 16px; color: var(--text-muted); font-size: 12px; text-align: center; background: var(--bg-sidebar); border-radius: 6px; }

  .vol-table {
    background: var(--bg-sidebar); border-radius: 6px; overflow: hidden;
  }
  .vol-table thead th { background: var(--bg); padding: 6px 10px; font-size: 10px; }
  .vol-table td { padding: 6px 10px; font-size: 12px; }
</style>
