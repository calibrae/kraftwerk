<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";

  let { vmName } = $props();

  let chains = $state([]);
  let loading = $state(false);
  let busy = $state({}); // disk -> bool, while a block job runs
  let jobs = $state({}); // disk -> { kind, cur, end, bandwidth }
  let err = $state(null);
  let pollTimer = null;
  let lastLoadedFor = null;

  async function load() {
    if (!vmName) return;
    loading = true;
    err = null;
    try {
      chains = await invoke("get_backing_chains", { name: vmName });
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      loading = false;
    }
  }

  async function pollJobs() {
    if (!vmName || chains.length === 0) return;
    let any = false;
    const next = {};
    for (const c of chains) {
      try {
        const info = await invoke("get_block_job", { name: vmName, disk: c.target });
        if (info) {
          next[c.target] = info;
          any = true;
        }
      } catch (_) { /* ignore per-disk poll errors */ }
    }
    jobs = next;
    if (!any && Object.keys(busy).some(k => busy[k])) {
      // A job we kicked off has finished — refresh chain.
      busy = {};
      await load();
    }
  }

  onMount(() => {
    load();
    pollTimer = setInterval(pollJobs, 1500);
  });
  onDestroy(() => { if (pollTimer) clearInterval(pollTimer); });

  $effect(() => {
    if (vmName && vmName !== lastLoadedFor) {
      lastLoadedFor = vmName;
      load();
    }
  });

  // Arm-to-confirm for destructive ops.
  let armed = $state(null);
  let armTimer = null;
  function arm(action) {
    armed = action;
    if (armTimer) clearTimeout(armTimer);
    armTimer = setTimeout(() => armed = null, 5000);
  }

  async function flatten(disk) {
    const key = `pull:${disk.target}`;
    if (armed !== key) { arm(key); return; }
    armed = null;
    busy[disk.target] = true;
    err = null;
    try {
      await invoke("block_pull", { name: vmName, disk: disk.target, bandwidthBps: 0 });
    } catch (e) {
      err = e?.message || String(e);
      busy[disk.target] = false;
    }
  }

  async function commit(disk) {
    const key = `commit:${disk.target}`;
    if (armed !== key) { arm(key); return; }
    armed = null;
    busy[disk.target] = true;
    err = null;
    try {
      // Active commit: top = "" (current), base = "" (immediate parent).
      // delete_after = false; libvirt only deletes for managed pool files
      // anyway and we want to be conservative.
      await invoke("block_commit", {
        name: vmName,
        disk: disk.target,
        top: null,
        base: null,
        bandwidthBps: 0,
        active: true,
        deleteAfter: false,
      });
    } catch (e) {
      err = e?.message || String(e);
      busy[disk.target] = false;
    }
  }

  async function cancel(disk) {
    busy[disk.target] = true;
    try {
      await invoke("block_job_abort", { name: vmName, disk: disk.target, pivot: false });
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy[disk.target] = false;
      await pollJobs();
    }
  }

  async function pivot(disk) {
    busy[disk.target] = true;
    try {
      await invoke("block_job_abort", { name: vmName, disk: disk.target, pivot: true });
      await load();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy[disk.target] = false;
    }
  }

  function pct(job) {
    if (!job || !job.end) return 0;
    return Math.min(100, (job.cur / job.end) * 100);
  }

  function leaf(p) {
    if (!p) return "—";
    const i = p.lastIndexOf("/");
    return i >= 0 ? p.slice(i + 1) : p;
  }
</script>

<section class="panel">
  <header class="ph">
    <h3>Backing chains</h3>
    <button class="btn-small" onclick={load} disabled={loading}>
      {loading ? "Loading…" : "Refresh"}
    </button>
  </header>
  {#if err}
    <pre class="err">{err}</pre>
  {/if}
  {#if chains.length === 0 && !loading}
    <p class="muted">No disks.</p>
  {/if}
  {#each chains as d (d.target)}
    <div class="disk" class:cdrom={d.device !== "disk"}>
      <header class="dh">
        <code class="target">{d.target}</code>
        <span class="dev">{d.device}</span>
        {#if d.readonly}<span class="ro">read-only</span>{/if}
        {#if d.chain.length > 0 && d.device === "disk"}
          <span class="grow"></span>
          {#if jobs[d.target]}
            <button class="btn-tiny" onclick={() => cancel(d)} disabled={busy[d.target] && !jobs[d.target]}>
              Cancel
            </button>
            {#if jobs[d.target].kind === "active_commit"}
              <button class="btn-tiny primary" onclick={() => pivot(d)} disabled={busy[d.target]}>
                Pivot
              </button>
            {/if}
          {:else}
            <button class="btn-tiny" class:armed={armed === `pull:${d.target}`}
              onclick={() => flatten(d)} disabled={busy[d.target]}>
              {armed === `pull:${d.target}` ? "Confirm: flatten" : "Flatten (block-pull)"}
            </button>
            <button class="btn-tiny" class:armed={armed === `commit:${d.target}`}
              onclick={() => commit(d)} disabled={busy[d.target]}>
              {armed === `commit:${d.target}` ? "Confirm: commit active" : "Commit active"}
            </button>
          {/if}
        {/if}
      </header>

      {#if jobs[d.target]}
        <div class="job">
          <span class="kind">{jobs[d.target].kind}</span>
          <div class="bar"><div class="bar-fill" style="width: {pct(jobs[d.target])}%"></div></div>
          <span class="muted small">{pct(jobs[d.target]).toFixed(1)}%</span>
        </div>
      {/if}

      <ol class="chain">
        <li class="link active">
          <span class="depth">0 (active)</span>
          <code class="path" title={d.source ?? ""}>{leaf(d.source)}</code>
          {#if d.source_format}<span class="fmt">{d.source_format}</span>{/if}
        </li>
        {#each d.chain as link (link.depth)}
          <li class="link">
            <span class="depth">{link.depth}</span>
            <code class="path" title={link.file}>{leaf(link.file)}</code>
            {#if link.format}<span class="fmt">{link.format}</span>{/if}
          </li>
        {/each}
      </ol>

      {#if d.chain.length === 0 && d.device === "disk" && !d.readonly}
        <p class="muted small">No backing chain — disk is a single image.</p>
      {/if}
    </div>
  {/each}
</section>

<style>
  .panel { padding: 16px; display: flex; flex-direction: column; gap: 12px; }
  .ph { display: flex; justify-content: space-between; align-items: center; }
  .ph h3 { margin: 0; font-size: 14px; font-weight: 600; }
  .muted { color: var(--text-muted); font-size: 12px; }
  .small { font-size: 11px; }

  .disk {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 12px;
  }
  .disk.cdrom { opacity: 0.7; }
  .dh { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  .target { font-weight: 600; }
  .dev {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .ro {
    font-size: 10px;
    background: rgba(107, 114, 128, 0.20);
    color: #9ca3af;
    padding: 1px 6px;
    border-radius: 3px;
  }
  .grow { flex: 1; }

  .btn-small, .btn-tiny {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
  }
  .btn-tiny { padding: 3px 10px; font-size: 11px; }
  .btn-small:hover:not(:disabled),
  .btn-tiny:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-tiny.primary {
    background: var(--accent); color: white; border-color: var(--accent);
  }
  .btn-tiny.armed {
    background: rgba(251, 191, 36, 0.18);
    border-color: #fbbf24;
    color: #fbbf24;
    font-weight: 500;
  }
  button:disabled { opacity: 0.5; cursor: not-allowed; }

  ol.chain {
    list-style: none;
    margin: 0;
    padding: 0;
    border-left: 2px solid var(--border);
    padding-left: 10px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .link {
    display: flex;
    gap: 8px;
    align-items: baseline;
    font-size: 12px;
  }
  .link.active {
    color: var(--text);
    font-weight: 500;
  }
  .link.active .depth { color: #34d399; }
  .depth {
    font-size: 10px;
    color: var(--text-muted);
    min-width: 70px;
    text-transform: lowercase;
  }
  .path {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    color: var(--text);
  }
  .fmt {
    font-size: 10px;
    color: var(--text-muted);
    background: var(--bg-input);
    padding: 1px 5px;
    border-radius: 3px;
  }

  .job {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 4px 0 8px;
  }
  .job .kind {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--accent, #60a5fa);
    min-width: 90px;
  }
  .bar {
    flex: 1;
    height: 5px;
    background: var(--bg-input);
    border-radius: 3px;
    overflow: hidden;
  }
  .bar-fill {
    height: 100%;
    background: linear-gradient(90deg, #60a5fa, #34d399);
    transition: width 0.4s ease;
  }

  pre.err {
    margin: 0;
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.10);
    border: 1px solid rgba(239, 68, 68, 0.30);
    border-radius: 6px;
    color: #ef4444;
    font-size: 11px;
    white-space: pre-wrap;
  }
</style>
