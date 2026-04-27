<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { getState } from "$lib/stores/app.svelte.js";

  let { vmName } = $props();
  const appState = getState();

  const SAMPLE_INTERVAL_MS = 5_000;
  const MAX_SAMPLES = 720; // 1h at 5s

  // Each entry: { t, cpuPct, ramPct, diskBps, netBps }
  let history = $state([]);
  let lastSample = null; // raw last DomainStatsSample for delta calc
  let err = $state(null);
  let busy = $state(false);
  let timer = null;
  let canvasRefs = $state([null, null, null, null]);

  let windowSize = $state(60); // samples to display: 12 = 1m, 60 = 5m, 180 = 15m, 720 = 1h
  const windowOptions = [
    { label: "1m", value: 12 },
    { label: "5m", value: 60 },
    { label: "15m", value: 180 },
    { label: "1h", value: 720 },
  ];

  let isRunning = $derived(appState.selectedVm?.state === "running");

  async function tick() {
    if (busy || !vmName) return;
    if (!isRunning) return;
    busy = true;
    try {
      const s = await invoke("get_domain_stats", { name: vmName });
      const point = computeDeltaPoint(s, lastSample);
      lastSample = s;
      if (point) {
        history = [...history, point].slice(-MAX_SAMPLES);
        redraw();
      }
      err = null;
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  function computeDeltaPoint(now, prev) {
    if (!prev) return null;
    const dtMs = now.timestamp_ms - prev.timestamp_ms;
    if (dtMs <= 0) return null;

    // CPU% = (cpu_time delta in ns) / (wall delta in ns) / vcpus * 100
    const cpuDeltaNs = Number(now.cpu_time_ns) - Number(prev.cpu_time_ns);
    const wallNs = dtMs * 1_000_000;
    const vcpus = Math.max(1, now.vcpus || 1);
    const cpuPct = Math.max(0, Math.min(100 * vcpus, (cpuDeltaNs / wallNs / vcpus) * 100));

    // RAM% — prefer balloon RSS if present, else hypervisor allocation.
    const rss = now.memory_rss_kib > 0 ? now.memory_rss_kib : now.memory_actual_kib;
    const max = now.memory_max_kib > 0 ? now.memory_max_kib : 1;
    const ramPct = (rss / max) * 100;

    // Aggregate disk I/O (bytes/sec).
    const sumDisk = (s) => s.disks.reduce((a, d) => a + Number(d.read_bytes) + Number(d.write_bytes), 0);
    const diskBps = Math.max(0, (sumDisk(now) - sumDisk(prev)) / (dtMs / 1000));

    // Aggregate net I/O (bytes/sec).
    const sumNet = (s) => s.interfaces.reduce((a, i) => a + Number(i.rx_bytes) + Number(i.tx_bytes), 0);
    const netBps = Math.max(0, (sumNet(now) - sumNet(prev)) / (dtMs / 1000));

    return { t: now.timestamp_ms, cpuPct, ramPct, diskBps, netBps };
  }

  function redraw() {
    const slice = history.slice(-windowSize);
    drawSeries(canvasRefs[0], slice.map(p => p.cpuPct),  100, "#34d399", v => `${v.toFixed(1)}%`);
    drawSeries(canvasRefs[1], slice.map(p => p.ramPct),  100, "#60a5fa", v => `${v.toFixed(1)}%`);
    const diskMax = Math.max(1024, ...slice.map(p => p.diskBps));
    drawSeries(canvasRefs[2], slice.map(p => p.diskBps), diskMax, "#fbbf24", v => fmtBps(v));
    const netMax = Math.max(1024, ...slice.map(p => p.netBps));
    drawSeries(canvasRefs[3], slice.map(p => p.netBps), netMax,  "#f472b6", v => fmtBps(v));
  }

  function drawSeries(canvas, values, yMax, color, fmt) {
    if (!canvas || values.length === 0) return;
    const dpr = window.devicePixelRatio || 1;
    const cssW = canvas.clientWidth;
    const cssH = canvas.clientHeight;
    if (canvas.width !== cssW * dpr || canvas.height !== cssH * dpr) {
      canvas.width = cssW * dpr;
      canvas.height = cssH * dpr;
    }
    const ctx = canvas.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);

    const padL = 4, padR = 4, padT = 6, padB = 4;
    const w = cssW - padL - padR;
    const h = cssH - padT - padB;

    // baseline
    ctx.strokeStyle = "rgba(148, 163, 184, 0.18)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padL, padT + h - 0.5);
    ctx.lineTo(padL + w, padT + h - 0.5);
    ctx.stroke();

    // line
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    const n = values.length;
    const stepX = n > 1 ? w / (n - 1) : 0;
    values.forEach((v, i) => {
      const x = padL + i * stepX;
      const y = padT + h - (v / yMax) * h;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.stroke();

    // current value
    const last = values[values.length - 1];
    ctx.fillStyle = color;
    ctx.font = "11px ui-monospace, SFMono-Regular, Menlo, monospace";
    ctx.textAlign = "right";
    ctx.fillText(fmt(last), cssW - 4, 12);
  }

  function fmtBps(b) {
    if (b >= 1024 * 1024 * 1024) return `${(b / 1024 / 1024 / 1024).toFixed(1)} GiB/s`;
    if (b >= 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MiB/s`;
    if (b >= 1024) return `${(b / 1024).toFixed(0)} KiB/s`;
    return `${Math.round(b)} B/s`;
  }

  function clear() {
    history = [];
    lastSample = null;
    redraw();
  }

  // Reactive: when window size changes, redraw immediately with cached data.
  $effect(() => {
    windowSize;
    redraw();
  });

  // Pause sampling when VM stops.
  $effect(() => {
    if (!isRunning && timer) {
      clearInterval(timer);
      timer = null;
    } else if (isRunning && !timer) {
      timer = setInterval(tick, SAMPLE_INTERVAL_MS);
      tick();
    }
  });

  onMount(() => {
    if (isRunning) {
      timer = setInterval(tick, SAMPLE_INTERVAL_MS);
      tick();
    }
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  $effect(() => {
    if (vmName) clear();
  });
</script>

<div class="panel">
  <header class="ph">
    <h3>Live metrics{#if !isRunning} <span class="muted small">(VM not running)</span>{/if}</h3>
    <div class="actions">
      {#each windowOptions as opt}
        <button class="btn-tab" class:active={windowSize === opt.value}
          onclick={() => windowSize = opt.value}>{opt.label}</button>
      {/each}
      <button class="btn-small" onclick={clear} title="Reset history">Clear</button>
    </div>
  </header>

  {#if err}
    <div class="error">{err}</div>
  {/if}

  {#if history.length < 2 && isRunning}
    <p class="muted">Collecting first samples… each chart needs two ticks for a delta.</p>
  {:else if history.length === 0}
    <p class="muted">No data yet. Start the VM to begin sampling.</p>
  {/if}

  <div class="grid">
    <section class="card">
      <h4>CPU</h4>
      <canvas bind:this={canvasRefs[0]}></canvas>
    </section>
    <section class="card">
      <h4>Memory</h4>
      <canvas bind:this={canvasRefs[1]}></canvas>
    </section>
    <section class="card">
      <h4>Disk I/O</h4>
      <canvas bind:this={canvasRefs[2]}></canvas>
    </section>
    <section class="card">
      <h4>Network I/O</h4>
      <canvas bind:this={canvasRefs[3]}></canvas>
    </section>
  </div>

  <p class="muted small">
    Sampling every 5s · {history.length}/{MAX_SAMPLES} samples retained ({Math.round(history.length * 5 / 60)} min)
    · showing last {Math.min(windowSize, history.length)} samples
  </p>
</div>

<style>
  .panel { padding: 16px; display: flex; flex-direction: column; gap: 12px; }
  .ph { display: flex; justify-content: space-between; align-items: center; }
  .ph h3 { margin: 0; font-size: 14px; font-weight: 600; }
  .actions { display: flex; gap: 4px; align-items: center; }

  .btn-tab {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text-muted);
    font-size: 11px;
    font-family: inherit;
    cursor: pointer;
  }
  .btn-tab:hover { color: var(--text); background: var(--bg-hover); }
  .btn-tab.active { color: var(--text); border-color: var(--accent); background: rgba(96, 165, 250, 0.10); }

  .btn-small {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    font-size: 11px;
    font-family: inherit;
    cursor: pointer;
    margin-left: 6px;
  }
  .btn-small:hover { background: var(--bg-hover); }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 12px;
  }
  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 12px 8px;
  }
  .card h4 {
    margin: 0 0 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  canvas {
    width: 100%;
    height: 80px;
    display: block;
  }

  .muted { color: var(--text-muted); }
  .small { font-size: 11px; }

  .error {
    padding: 6px 10px;
    background: rgba(239, 68, 68, 0.10);
    border: 1px solid rgba(239, 68, 68, 0.30);
    border-radius: 6px;
    color: #ef4444;
    font-size: 12px;
  }
</style>
