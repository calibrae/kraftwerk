<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { getState } from "$lib/stores/app.svelte.js";

  const appState = getState();

  let host = $state(null);
  let mem = $state(null);
  let err = $state(null);
  let timer = null;

  // Secrets section
  let secrets = $state([]);
  let secretsBusy = $state(false);
  let secretsErr = $state(null);
  let armedDelete = $state(null);
  let armTimer = null;

  async function loadSecrets() {
    secretsBusy = true;
    secretsErr = null;
    try {
      secrets = await invoke("list_secrets");
    } catch (e) {
      secretsErr = e?.message || String(e);
    } finally {
      secretsBusy = false;
    }
  }

  function maybeDeleteSecret(uuid) {
    if (armedDelete !== uuid) {
      armedDelete = uuid;
      if (armTimer) clearTimeout(armTimer);
      armTimer = setTimeout(() => armedDelete = null, 5000);
      return;
    }
    armedDelete = null;
    secretsBusy = true;
    secretsErr = null;
    invoke("delete_secret", { uuid })
      .then(loadSecrets)
      .catch(e => { secretsErr = e?.message || String(e); })
      .finally(() => { secretsBusy = false; });
  }

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
    await loadSecrets();
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

  let memUsedKib = $derived(mem ? Math.max(0, mem.total_kib - mem.available_kib) : 0);
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
        <p class="muted small">{formatKib(mem.available_kib)} available · {memPct.toFixed(1)}% used</p>
        <p class="muted small">{formatKib(mem.free_kib)} free · {formatKib(mem.buffers_kib + mem.cached_kib)} reclaimable (buffers + cached)</p>
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

    <!-- Secrets card (full width) -->
    <section class="card wide">
      <h2>
        Secrets <span class="count">{secrets.length}</span>
        <button class="btn-tiny" onclick={loadSecrets} disabled={secretsBusy}>
          {secretsBusy ? "…" : "Refresh"}
        </button>
      </h2>
      {#if secretsErr}
        <pre class="err">{secretsErr}</pre>
      {/if}
      {#if secrets.length === 0}
        <p class="muted small">No secrets defined. LUKS volumes auto-create them on first encrypt.</p>
      {:else}
        <ul class="secrets">
          {#each secrets as s (s.uuid)}
            <li>
              <span class="mono small">{s.uuid.slice(0, 8)}…</span>
              <span class="usage-pill">{s.usage}</span>
              {#if s.usage_id}
                <span class="muted small mono">{s.usage_id}</span>
              {/if}
              {#if s.description}
                <span class="muted small">{s.description}</span>
              {/if}
              <span class="grow"></span>
              {#if !s.has_value}<span class="warn-pill">no value</span>{/if}
              {#if s.private}<span class="usage-pill">private</span>{/if}
              <button class="btn-tiny danger"
                class:armed={armedDelete === s.uuid}
                onclick={() => maybeDeleteSecret(s.uuid)} disabled={secretsBusy}>
                {armedDelete === s.uuid ? "Confirm" : "Delete"}
              </button>
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

  ul.secrets {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  ul.secrets li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-radius: 4px;
  }
  ul.secrets li:hover { background: var(--bg-hover); }
  .usage-pill {
    font-size: 10px;
    background: rgba(96, 165, 250, 0.15);
    color: #60a5fa;
    padding: 1px 6px;
    border-radius: 3px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .warn-pill {
    font-size: 10px;
    background: rgba(251, 191, 36, 0.15);
    color: #fbbf24;
    padding: 1px 6px;
    border-radius: 3px;
  }
  .btn-tiny {
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-button);
    color: var(--text);
    font-size: 11px;
    font-family: inherit;
    cursor: pointer;
    margin-left: 8px;
  }
  .btn-tiny:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-tiny:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-tiny.danger { color: #ef4444; }
  .btn-tiny.danger.armed {
    background: rgba(239, 68, 68, 0.18);
    color: #ef4444;
    border-color: #ef4444;
    font-weight: 500;
  }
  pre.err {
    margin: 0 0 8px;
    padding: 6px 10px;
    background: rgba(239, 68, 68, 0.10);
    border: 1px solid rgba(239, 68, 68, 0.30);
    border-radius: 4px;
    color: #ef4444;
    font-size: 11px;
    white-space: pre-wrap;
  }

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
