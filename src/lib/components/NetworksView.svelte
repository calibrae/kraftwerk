<script>
  import { getState, refreshNetworks, startNetwork, stopNetwork, deleteNetwork, setNetworkAutostart, getNetworkXml } from "$lib/stores/app.svelte.js";

  let { onCreateNetwork } = $props();
  const appState = getState();

  let selectedName = $state(null);
  let networkXml = $state(null);
  let busy = $state(false);

  const modeColors = {
    nat: "#6366f1",
    route: "#10b981",
    open: "#06b6d4",
    bridge: "#f59e0b",
    isolated: "#6b7280",
  };

  async function select(name) {
    selectedName = name;
    networkXml = null;
  }

  async function loadXml(name) {
    busy = true;
    networkXml = await getNetworkXml(name);
    busy = false;
  }

  async function confirmDelete(name) {
    if (!confirm(`Delete network "${name}"? This cannot be undone.`)) return;
    busy = true;
    await deleteNetwork(name);
    if (selectedName === name) selectedName = null;
    busy = false;
  }

  let selected = $derived(
    selectedName ? appState.networks.find((n) => n.name === selectedName) ?? null : null,
  );
</script>

<div class="networks-view">
  <div class="nw-header">
    <h2>Virtual Networks</h2>
    <div class="nw-actions">
      <button class="btn" onclick={refreshNetworks} disabled={busy}>Refresh</button>
      <button class="btn btn-primary" onclick={onCreateNetwork}>+ New Network</button>
    </div>
  </div>

  {#if appState.networks.length === 0}
    <div class="empty">
      <p>No networks defined on this hypervisor.</p>
      <button class="btn btn-primary" onclick={onCreateNetwork}>Create one</button>
    </div>
  {:else}
    <div class="nw-layout">
      <div class="nw-list">
        <table>
          <thead>
            <tr>
              <th></th>
              <th>Name</th>
              <th>Mode</th>
              <th>Bridge</th>
              <th>IPv4</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each appState.networks as net (net.name)}
              <tr class:selected={selectedName === net.name} onclick={() => select(net.name)}>
                <td>
                  <span class="state-dot" class:active={net.is_active}></span>
                </td>
                <td class="name">{net.name}</td>
                <td>
                  <span class="mode-badge" style="background: {modeColors[net.forward_mode] ?? '#6b7280'}">
                    {net.forward_mode}
                  </span>
                </td>
                <td class="mono">{net.bridge ?? '—'}</td>
                <td class="mono">{net.ipv4_summary ?? '—'}</td>
                <td class="row-actions">
                  {#if net.is_active}
                    <button class="btn-tiny" onclick={(e) => { e.stopPropagation(); stopNetwork(net.name); }}>Stop</button>
                  {:else}
                    <button class="btn-tiny start" onclick={(e) => { e.stopPropagation(); startNetwork(net.name); }}>Start</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

      {#if selected}
        <div class="nw-detail">
          <h3>{selected.name}</h3>
          <dl>
            <dt>UUID</dt><dd class="mono">{selected.uuid}</dd>
            <dt>State</dt><dd>{selected.is_active ? "Active" : "Inactive"}</dd>
            <dt>Persistent</dt><dd>{selected.is_persistent ? "Yes" : "No"}</dd>
            <dt>Forward Mode</dt><dd>{selected.forward_mode}</dd>
            {#if selected.bridge}<dt>Bridge</dt><dd class="mono">{selected.bridge}</dd>{/if}
            {#if selected.ipv4_summary}<dt>IPv4</dt><dd class="mono">{selected.ipv4_summary}</dd>{/if}
            {#if selected.ipv6_summary}<dt>IPv6</dt><dd class="mono">{selected.ipv6_summary}</dd>{/if}
            <dt>Autostart</dt>
            <dd>
              <label class="switch">
                <input
                  type="checkbox"
                  checked={selected.autostart}
                  onchange={(e) => setNetworkAutostart(selected.name, e.target.checked)}
                />
                <span>{selected.autostart ? "On" : "Off"}</span>
              </label>
            </dd>
          </dl>

          <div class="detail-actions">
            <button class="btn" onclick={() => loadXml(selected.name)} disabled={busy}>
              {networkXml ? "Reload XML" : "Show XML"}
            </button>
            <button class="btn btn-danger" onclick={() => confirmDelete(selected.name)} disabled={busy}>Delete</button>
          </div>

          {#if networkXml}
            <pre class="xml">{networkXml}</pre>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .networks-view { padding: 24px; height: 100vh; overflow-y: auto; }

  .nw-header {
    display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;
  }
  .nw-header h2 { margin: 0; font-size: 20px; font-weight: 600; }
  .nw-actions { display: flex; gap: 8px; }

  .btn {
    padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; font-family: inherit; cursor: pointer;
  }
  .btn:hover { background: var(--bg-hover); }
  .btn-primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn-primary:hover { filter: brightness(1.1); }
  .btn-danger { background: #7f1d1d; border-color: #7f1d1d; color: #fca5a5; }
  .btn-danger:hover { background: #991b1b; }

  .empty { text-align: center; padding: 60px 20px; color: var(--text-muted); }
  .empty p { margin: 0 0 16px; }

  .nw-layout { display: grid; grid-template-columns: 1fr 360px; gap: 20px; }

  .nw-list { background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
  table { width: 100%; border-collapse: collapse; }
  thead th { text-align: left; padding: 10px 12px; font-size: 11px; text-transform: uppercase;
    color: var(--text-muted); letter-spacing: 0.05em; background: var(--bg-sidebar); font-weight: 600; }
  tbody tr { cursor: pointer; border-top: 1px solid var(--border); }
  tbody tr:hover { background: var(--bg-hover); }
  tbody tr.selected { background: var(--bg-selected); }
  td { padding: 10px 12px; font-size: 13px; }
  .name { font-weight: 500; }
  .mono { font-family: 'SF Mono', 'Fira Code', monospace; font-size: 12px; }

  .state-dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: #6b7280; }
  .state-dot.active { background: #34d399; }

  .mode-badge {
    display: inline-block; padding: 2px 8px; border-radius: 10px;
    font-size: 10px; font-weight: 600; color: white; text-transform: uppercase;
  }

  .row-actions { text-align: right; }
  .btn-tiny { padding: 2px 8px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--bg-button); color: var(--text); font-size: 11px; cursor: pointer; }
  .btn-tiny:hover { background: var(--bg-hover); }
  .btn-tiny.start { background: #065f46; color: #34d399; border-color: #065f46; }
  .btn-tiny.start:hover { background: #047857; }

  .nw-detail { background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; padding: 16px; }
  .nw-detail h3 { margin: 0 0 12px; font-size: 15px; font-weight: 600; }
  dl { display: grid; grid-template-columns: 110px 1fr; gap: 6px 12px; margin: 0 0 16px; font-size: 12px; }
  dt { color: var(--text-muted); }
  dd { margin: 0; word-break: break-all; }

  .switch { display: flex; align-items: center; gap: 8px; cursor: pointer; }
  .switch input { margin: 0; }

  .detail-actions { display: flex; gap: 6px; }

  .xml {
    margin-top: 16px; padding: 12px; background: var(--bg-sidebar); border: 1px solid var(--border);
    border-radius: 6px; font-size: 11px; line-height: 1.45; white-space: pre;
    overflow-x: auto; max-height: 300px; overflow-y: auto;
    font-family: 'SF Mono', 'Fira Code', monospace;
  }
</style>
