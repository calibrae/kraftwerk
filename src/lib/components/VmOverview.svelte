<script>
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Sparkline from "./Sparkline.svelte";

  let { vmName, running } = $props();

  const WINDOW = 60; // samples kept (≈ 60 seconds at 1 Hz)
  const INTERVAL_MS = 1000;

  // Live series
  let cpuPct = $state([]);           // %, 0..100
  let memPctSeries = $state([]);     // % of max
  let diskReadRate = $state([]);     // bytes/sec
  let diskWriteRate = $state([]);
  let netRxRate = $state([]);
  let netTxRate = $state([]);

  let lastSample = $state(null);
  let currentSample = $state(null);
  let err = $state(null);
  let timer = null;
  let aborted = false;

  function push(arr, v) {
    arr.push(v);
    if (arr.length > WINDOW) arr.shift();
    return arr;
  }

  async function tick() {
    if (aborted) return;
    try {
      const s = await invoke("get_domain_stats", { name: vmName });

      if (lastSample) {
        const dtSec = Math.max((s.timestamp_ms - lastSample.timestamp_ms) / 1000, 0.001);
        // CPU% = delta(cpu_time_ns) / (dt_sec * 1e9 * nrVcpus) * 100
        const cpuDelta = Math.max(s.cpu_time_ns - lastSample.cpu_time_ns, 0);
        const cpuPercent = Math.min(
          100,
          (cpuDelta / (dtSec * 1e9 * Math.max(s.vcpus, 1))) * 100,
        );
        cpuPct = push(cpuPct, cpuPercent);

        // Memory: use actual allocated vs max. Balloon RSS if available, else actual.
        const memUsed = s.memory_rss_kib > 0 ? s.memory_rss_kib : s.memory_actual_kib;
        const memMax = Math.max(s.memory_max_kib, 1);
        memPctSeries = push(memPctSeries, (memUsed / memMax) * 100);

        // Aggregate across all disks / NICs
        const prevDisk = lastSample.disks.reduce((acc, d) => {
          acc[d.device] = d; return acc;
        }, {});
        let rd = 0, wr = 0;
        for (const d of s.disks) {
          const prev = prevDisk[d.device];
          if (prev) {
            rd += Math.max(d.read_bytes - prev.read_bytes, 0);
            wr += Math.max(d.write_bytes - prev.write_bytes, 0);
          }
        }
        diskReadRate = push(diskReadRate, rd / dtSec);
        diskWriteRate = push(diskWriteRate, wr / dtSec);

        const prevIf = lastSample.interfaces.reduce((acc, i) => {
          acc[i.path] = i; return acc;
        }, {});
        let rx = 0, tx = 0;
        for (const i of s.interfaces) {
          const prev = prevIf[i.path];
          if (prev) {
            rx += Math.max(i.rx_bytes - prev.rx_bytes, 0);
            tx += Math.max(i.tx_bytes - prev.tx_bytes, 0);
          }
        }
        netRxRate = push(netRxRate, rx / dtSec);
        netTxRate = push(netTxRate, tx / dtSec);
      }

      lastSample = s;
      currentSample = s;
      err = null;
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    }
  }

  $effect(() => {
    // Reset when VM changes
    lastSample = null;
    currentSample = null;
    cpuPct = []; memPctSeries = []; diskReadRate = []; diskWriteRate = []; netRxRate = []; netTxRate = [];
    err = null;

    if (!running || !vmName) return;

    // Kick once immediately, then every INTERVAL_MS.
    tick();
    timer = setInterval(tick, INTERVAL_MS);
    return () => {
      if (timer) clearInterval(timer);
      timer = null;
    };
  });

  onDestroy(() => {
    aborted = true;
    if (timer) clearInterval(timer);
  });

  // Helpers for formatted current values
  function formatRate(bytesPerSec) {
    if (bytesPerSec < 1024) return `${bytesPerSec.toFixed(0)} B/s`;
    if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
    if (bytesPerSec < 1024 * 1024 * 1024) return `${(bytesPerSec / 1024 / 1024).toFixed(2)} MB/s`;
    return `${(bytesPerSec / 1024 / 1024 / 1024).toFixed(2)} GB/s`;
  }

  function formatKib(kib) {
    if (kib >= 1024 * 1024) return `${(kib / 1024 / 1024).toFixed(2)} GiB`;
    if (kib >= 1024) return `${(kib / 1024).toFixed(0)} MiB`;
    return `${kib} KiB`;
  }

  let latestCpu = $derived(cpuPct.length > 0 ? cpuPct[cpuPct.length - 1] : 0);
  let latestMemPct = $derived(memPctSeries.length > 0 ? memPctSeries[memPctSeries.length - 1] : 0);
  let latestDiskR = $derived(diskReadRate.length > 0 ? diskReadRate[diskReadRate.length - 1] : 0);
  let latestDiskW = $derived(diskWriteRate.length > 0 ? diskWriteRate[diskWriteRate.length - 1] : 0);
  let latestNetR = $derived(netRxRate.length > 0 ? netRxRate[netRxRate.length - 1] : 0);
  let latestNetT = $derived(netTxRate.length > 0 ? netTxRate[netTxRate.length - 1] : 0);
</script>

<div class="overview">
  {#if !running}
    <div class="muted">
      VM is not running — no live metrics available.
    </div>
  {:else}
    {#if err}
      <div class="error">Metrics error: {err}</div>
    {/if}

    <div class="grid">
      <div class="card">
        <div class="card-header">
          <span class="card-title">CPU</span>
          <span class="card-value" style="color: #6366f1">{latestCpu.toFixed(1)}%</span>
        </div>
        <Sparkline values={cpuPct} max={100} color="#6366f1" suffix="%" />
        {#if currentSample}
          <div class="sub">{currentSample.vcpus} vCPU{currentSample.vcpus !== 1 ? "s" : ""}</div>
        {/if}
      </div>

      <div class="card">
        <div class="card-header">
          <span class="card-title">Memory</span>
          <span class="card-value" style="color: #10b981">{latestMemPct.toFixed(1)}%</span>
        </div>
        <Sparkline values={memPctSeries} max={100} color="#10b981" suffix="%" />
        {#if currentSample}
          <div class="sub">
            {formatKib(currentSample.memory_rss_kib > 0 ? currentSample.memory_rss_kib : currentSample.memory_actual_kib)}
            / {formatKib(currentSample.memory_max_kib)}
            {#if currentSample.memory_rss_kib === 0}<span class="note">(balloon driver inactive — showing allocated)</span>{/if}
          </div>
        {/if}
      </div>

      <div class="card">
        <div class="card-header">
          <span class="card-title">Disk Read</span>
          <span class="card-value" style="color: #f59e0b">{formatRate(latestDiskR)}</span>
        </div>
        <Sparkline values={diskReadRate} color="#f59e0b" />
        {#if currentSample}
          <div class="sub">{currentSample.disks.length} disk{currentSample.disks.length !== 1 ? "s" : ""}</div>
        {/if}
      </div>

      <div class="card">
        <div class="card-header">
          <span class="card-title">Disk Write</span>
          <span class="card-value" style="color: #ef4444">{formatRate(latestDiskW)}</span>
        </div>
        <Sparkline values={diskWriteRate} color="#ef4444" />
      </div>

      <div class="card">
        <div class="card-header">
          <span class="card-title">Network RX</span>
          <span class="card-value" style="color: #06b6d4">{formatRate(latestNetR)}</span>
        </div>
        <Sparkline values={netRxRate} color="#06b6d4" />
        {#if currentSample}
          <div class="sub">{currentSample.interfaces.length} interface{currentSample.interfaces.length !== 1 ? "s" : ""}</div>
        {/if}
      </div>

      <div class="card">
        <div class="card-header">
          <span class="card-title">Network TX</span>
          <span class="card-value" style="color: #a78bfa">{formatRate(latestNetT)}</span>
        </div>
        <Sparkline values={netTxRate} color="#a78bfa" />
      </div>
    </div>

    {#if currentSample && currentSample.interfaces.length > 0}
      <div class="details">
        <h4>Network Interfaces</h4>
        <table>
          <thead>
            <tr>
              <th>Device</th><th>MAC</th><th>Model</th><th>RX</th><th>TX</th>
            </tr>
          </thead>
          <tbody>
            {#each currentSample.interfaces as nic}
              <tr>
                <td class="mono">{nic.path}</td>
                <td class="mono">{nic.mac}</td>
                <td>{nic.model}</td>
                <td>{(nic.rx_bytes / 1e6).toFixed(2)} MB</td>
                <td>{(nic.tx_bytes / 1e6).toFixed(2)} MB</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    {#if currentSample && currentSample.disks.length > 0}
      <div class="details">
        <h4>Disks</h4>
        <table>
          <thead>
            <tr><th>Device</th><th>Read</th><th>Write</th><th>Reads</th><th>Writes</th></tr>
          </thead>
          <tbody>
            {#each currentSample.disks as d}
              <tr>
                <td class="mono">{d.device}</td>
                <td>{(d.read_bytes / 1e6).toFixed(2)} MB</td>
                <td>{(d.write_bytes / 1e6).toFixed(2)} MB</td>
                <td>{d.read_req.toLocaleString()}</td>
                <td>{d.write_req.toLocaleString()}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/if}
</div>

<style>
  .overview { display: flex; flex-direction: column; gap: 20px; }

  .muted {
    padding: 40px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  .error {
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    color: #ef4444;
    font-size: 12px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }

  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .card-header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .card-title {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 600;
  }

  .card-value {
    font-size: 15px;
    font-weight: 600;
    font-family: 'SF Mono', monospace;
  }

  .sub {
    font-size: 11px;
    color: var(--text-muted);
  }

  .note {
    font-style: italic;
    margin-left: 4px;
  }

  .details {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 14px;
  }

  h4 {
    margin: 0 0 10px;
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 600;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  thead th {
    text-align: left;
    padding: 6px 10px;
    color: var(--text-muted);
    font-weight: 500;
    border-bottom: 1px solid var(--border);
  }
  tbody td {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  tbody tr:last-child td { border-bottom: none; }
  .mono { font-family: 'SF Mono', monospace; font-size: 11px; }
</style>
