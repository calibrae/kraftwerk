<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { getState } from "$lib/stores/app.svelte.js";

  const appState = getState();

  let host = $state(null);
  let mem = $state(null);
  let err = $state(null);
  let timer = null;

  async function loadHost() {
    try {
      host = await invoke("get_host_info");
    } catch (e) {
      err = e?.message || String(e);
    }
  }

  async function loadMem() {
    try {
      mem = await invoke("get_host_memory");
    } catch (_) {
      // memory probe is best-effort; don't blank the dashboard if it fails
    }
  }

  onMount(async () => {
    await loadHost();
    await loadMem();
    timer = setInterval(loadMem, 5000);
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  function formatKib(kib) {
    if (kib >= 1024 * 1024 * 1024) return `${(kib / 1024 / 1024 / 1024).toFixed(1)} TiB`;
    if (kib >= 1024 * 1024) return `${(kib / 1024 / 1024).toFixed(1)} GiB`;
    if (kib >= 1024) return `${(kib / 1024).toFixed(0)} MiB`;
    return `${kib} KiB`;
  }

  function formatBytes(b) { return formatKib(b / 1024); }

  let vmCounts = $derived.by(() => {
    const vms = appState.vms ?? [];
    const counts = { running: 0, paused: 0, shut_off: 0, crashed: 0, suspended: 0, unknown: 0, total: vms.length };
    for (const v of vms) counts[v.state] = (counts[v.state] ?? 0) + 1;
    return counts;
  });

  let memUsedKib = $derived(mem ? Math.max(0, mem.total_kib - mem.free_kib) : 0);
  let memPct = $derived(mem && mem.total_kib > 0 ? (memUsedKib / mem.total_kib) * 100 : 0);

  let totalPoolCapacity = $derived((appState.pools ?? []).reduce((acc, p) => acc + (p.capacity ?? 0), 0));
  let totalPoolAllocation = $derived((appState.pools ?? []).reduce((acc, p) => acc + (p.allocation ?? 0), 0));
</script>

<div class="dashboard">
  <header class="hd">
    <h1>{host?.hostname ?? "Hypervisor"}</h1>
    <span class="sub">{host?.hypervisor_type ?? ""}{host?.libvirt_version ? ` · libvirt ${host.libvirt_version}` : ""}</span>
  </header>

  {#if err}
    <div class="error">Failed to load host info: {err}</div>
  {/if}

  <div class="grid">
    <!-- Host hardware card -->
    <section class="card">
      <h2>Host</h2>
      {#if host}
        <dl>
          <dt>CPU</dt><dd class="mono">{host.cpu_model || "—"}</dd>
          <dt>Topology</dt><dd>{host.cpu_sockets} × {host.cpu_cores_per_socket} × {host.cpu_threads_per_core} ({host.cpu_count} active{host.cpu_mhz ? `, ${host.cpu_mhz} MHz` : ""})</dd>
          <dt>NUMA nodes</dt><dd>{host.numa_nodes}</dd>
          <dt>Total RAM</dt><dd>{formatKib(host.memory_kib)}</dd>
        </dl>
      {:else}
        <p class="muted">Loading…</p>
      {/if}
    </section>

    <!-- Live memory card -->
    <section class="card">
      <h2>Memory</h2>
      {#if mem}
        <div class="bar-wrap">
          <div class="bar"><div class="bar-fill" style="width: {memPct}%"></div></div>
          <div class="bar-label">
            <span>{formatKib(memUsedKib)} used</span>
            <span class="muted">of {formatKib(mem.total_kib)}</span>
          </div>
        </div>
        <p class="muted small">{formatKib(mem.free_kib)} free · {memPct.toFixed(1)}%</p>
      {:else}
        <p class="muted">—</p>
      {/if}
    </section>

    <!-- VMs card -->
    <section class="card">
      <h2>VMs <span class="count">{vmCounts.total}</span></h2>
      <div class="chips">
        {#if vmCounts.running > 0}<span class="chip running">{vmCounts.running} running</span>{/if}
        {#if vmCounts.paused > 0}<span class="chip paused">{vmCounts.paused} paused</span>{/if}
        {#if vmCounts.shut_off > 0}<span class="chip shut">{vmCounts.shut_off} shut off</span>{/if}
        {#if vmCounts.suspended > 0}<span class="chip paused">{vmCounts.suspended} suspended</span>{/if}
        {#if vmCounts.crashed > 0}<span class="chip crashed">{vmCounts.crashed} crashed</span>{/if}
        {#if vmCounts.unknown > 0}<span class="chip">{vmCounts.unknown} unknown</span>{/if}
        {#if vmCounts.total === 0}<span class="muted">No VMs defined</span>{/if}
      </div>
    </section>

    <!-- Networks card -->
    <section class="card">
      <h2>Networks <span class="count">{(appState.networks ?? []).length}</span></h2>
      {#if (appState.networks ?? []).length === 0}
        <p class="muted">None</p>
      {:else}
        <ul class="dense">
          {#each (appState.networks ?? []).slice(0, 10) as n (n.uuid)}
            <li>
              <span class="dot" class:active={n.is_active}></span>
              <span class="mono">{n.name}</span>
              <span class="muted small">{n.forward_mode}{n.bridge ? ` · ${n.bridge}` : ""}</span>
            </li>
          {/each}
          {#if (appState.networks ?? []).length > 10}
            <li class="muted small">+{(appState.networks ?? []).length - 10} more</li>
          {/if}
        </ul>
      {/if}
    </section>

    <!-- Storage card (full width) -->
    <section class="card wide">
      <h2>
        Storage <span class="count">{(appState.pools ?? []).length}</span>
        {#if totalPoolCapacity > 0}
          <span class="muted small">{formatBytes(totalPoolAllocation)} / {formatBytes(totalPoolCapacity)}</span>
        {/if}
      </h2>
      {#if (appState.pools ?? []).length === 0}
        <p class="muted">No pools</p>
      {:else}
        <ul class="pools">
          {#each (appState.pools ?? []) as p (p.uuid)}
            {@const pct = p.capacity > 0 ? (p.allocation / p.capacity) * 100 : 0}
            <li>
              <div class="pool-row">
                <span class="dot" class:active={p.is_active}></span>
                <span class="mono">{p.name}</span>
                <span class="muted small">{p.pool_type}{p.target_path ? ` · ${p.target_path}` : ""}</span>
                <span class="grow"></span>
                <span class="muted small">{formatBytes(p.allocation)} / {formatBytes(p.capacity)} · {p.num_volumes} vols</span>
              </div>
              <div class="bar slim"><div class="bar-fill" style="width: {pct}%"></div></div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
</div>

<style>
  .dashboard {
    padding: 24px;
    overflow-y: auto;
    height: 100%;
    box-sizing: border-box;
  }
  .hd { display: flex; align-items: baseline; gap: 12px; margin-bottom: 20px; }
  .hd h1 { margin: 0; font-size: 22px; font-weight: 600; }
  .hd .sub { color: var(--text-muted); font-size: 12px; }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 16px;
  }
  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 16px;
  }
  .card.wide { grid-column: 1 / -1; }
  .card h2 {
    margin: 0 0 12px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .count {
    background: var(--bg-input);
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
    color: var(--text);
    text-transform: none;
    letter-spacing: 0;
  }
  dl { margin: 0; display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; font-size: 13px; }
  dt { color: var(--text-muted); }
  dd { margin: 0; }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
  .muted { color: var(--text-muted); }
  .small { font-size: 11px; }

  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip {
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 12px;
    background: var(--bg-input);
    color: var(--text);
  }
  .chip.running { background: rgba(52, 211, 153, 0.15); color: #34d399; }
  .chip.paused  { background: rgba(251, 191, 36, 0.15); color: #fbbf24; }
  .chip.shut    { background: rgba(107, 114, 128, 0.20); color: #9ca3af; }
  .chip.crashed { background: rgba(239, 68, 68, 0.15); color: #ef4444; }

  .dot {
    display: inline-block;
    width: 8px; height: 8px;
    border-radius: 50%;
    background: #6b7280;
    margin-right: 6px;
  }
  .dot.active { background: #34d399; }

  ul.dense { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
  ul.dense li { display: flex; align-items: center; gap: 8px; }

  ul.pools { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 10px; }
  ul.pools li { display: flex; flex-direction: column; gap: 4px; }
  .pool-row { display: flex; align-items: center; gap: 8px; font-size: 13px; }
  .grow { flex: 1; }

  .bar-wrap { display: flex; flex-direction: column; gap: 4px; }
  .bar {
    height: 8px;
    background: var(--bg-input);
    border-radius: 4px;
    overflow: hidden;
  }
  .bar.slim { height: 6px; }
  .bar-fill {
    height: 100%;
    background: linear-gradient(90deg, #60a5fa, #34d399);
    transition: width 0.4s ease;
  }
  .bar-label { display: flex; justify-content: space-between; font-size: 12px; }

  .error {
    margin-bottom: 12px;
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    color: #ef4444;
    font-size: 12px;
  }
</style>
