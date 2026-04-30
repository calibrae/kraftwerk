<script>
  /*
   * Live migration dialog. Picks a destination connection from the
   * pool of currently-open ones, sets MigrationConfig knobs, kicks
   * off virDomainMigrate, and polls progress until completion or
   * cancellation.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false), vmName, sourceConnectionId } = $props();
  const appState = getState();

  // Form state
  let destId = $state("");
  let live = $state(true);
  let persistDest = $state(true);
  let undefineSource = $state(false);
  let autoConverge = $state(true);
  let bandwidthMib = $state(0);

  // Run state
  let running = $state(false);
  let progress = $state(null);
  let err = $state(null);
  let pollHandle = null;

  // Open connections excluding the source (can't migrate to self).
  let openIds = $state([]);
  let savedConnections = $derived(appState.savedConnections ?? []);
  let candidates = $derived(
    openIds
      .filter((id) => id !== sourceConnectionId)
      .map((id) => {
        const sc = savedConnections.find((c) => c.id === id);
        return sc ? { id, label: `${sc.display_name} (${sc.uri})` } : { id, label: id };
      }),
  );

  $effect(() => {
    if (open) refreshOpenConnections();
    else stopPolling();
  });

  async function refreshOpenConnections() {
    try {
      openIds = await invoke("list_open_connections");
      if (!destId && candidates.length > 0) destId = candidates[0].id;
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    }
  }

  function close() {
    if (running) return; // don't close mid-migration
    stopPolling();
    progress = null;
    err = null;
    open = false;
  }

  async function startMigration() {
    if (!destId) { err = "Pick a destination connection"; return; }
    err = null;
    running = true;
    progress = null;

    const cfg = {
      live,
      persist_dest: persistDest,
      undefine_source: undefineSource,
      auto_converge: autoConverge,
      bandwidth_mibs: Number(bandwidthMib) || 0,
      dest_uri: null,
      dest_xml: null,
      dest_name: null,
    };

    startPolling();

    try {
      await invoke("migrate_domain", {
        sourceConnectionId,
        destConnectionId: destId,
        name: vmName,
        config: cfg,
      });
      // Final poll to surface the completed state before stopping.
      await pollOnce();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      stopPolling();
      running = false;
    }
  }

  async function pollOnce() {
    try {
      progress = await invoke("get_migration_status", {
        sourceConnectionId,
        name: vmName,
      });
    } catch (_) { /* domain may have already moved off the source */ }
  }

  function startPolling() {
    if (pollHandle != null) return;
    pollHandle = setInterval(pollOnce, 1000);
  }

  function stopPolling() {
    if (pollHandle != null) {
      clearInterval(pollHandle);
      pollHandle = null;
    }
  }

  async function cancel() {
    if (!running) return;
    try {
      await invoke("cancel_migration", { sourceConnectionId, name: vmName });
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    }
  }

  function fmtBytes(n) {
    if (n == null) return "—";
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let v = Number(n), u = 0;
    while (v >= 1024 && u < units.length - 1) { v /= 1024; u++; }
    return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${units[u]}`;
  }
  function fmtMs(ms) {
    if (ms == null) return "—";
    if (ms < 1000) return `${ms} ms`;
    const s = Math.floor(ms / 1000), m = Math.floor(s / 60);
    return m > 0 ? `${m}m ${s % 60}s` : `${s}s`;
  }
  let percent = $derived(() => {
    if (!progress?.data_total || !progress?.data_processed) return null;
    return Math.min(100, (Number(progress.data_processed) / Number(progress.data_total)) * 100);
  });
</script>

{#if open}
<div class="backdrop" onclick={close} role="presentation">
  <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
    <header>
      <h3>Live migrate · {vmName}</h3>
      <button class="x" onclick={close} disabled={running}>×</button>
    </header>

    {#if !running && !progress}
      <div class="form">
        {#if candidates.length === 0}
          <div class="warn">
            No other connections are currently open. Connect to a second
            hypervisor first — its libvirtd has to be reachable from this
            app for migration.
          </div>
        {:else}
          <label><span>Destination</span>
            <select bind:value={destId}>
              {#each candidates as c}<option value={c.id}>{c.label}</option>{/each}
            </select>
          </label>
        {/if}

        <fieldset class="flags">
          <legend>Flags</legend>
          <label class="toggle"><input type="checkbox" bind:checked={live}/><span>Live (pre-copy memory while running)</span></label>
          <label class="toggle"><input type="checkbox" bind:checked={persistDest}/><span>Persist on destination</span></label>
          <label class="toggle"><input type="checkbox" bind:checked={undefineSource}/><span>Undefine on source after migration</span></label>
          <label class="toggle"><input type="checkbox" bind:checked={autoConverge}/><span>Auto-converge (CPU throttle when memory dirties faster than network)</span></label>
        </fieldset>

        <label>
          <span>Bandwidth cap (MiB/s, 0 = unlimited)</span>
          <input type="number" min="0" bind:value={bandwidthMib}/>
        </label>

        <div class="warn">
          Storage must be reachable from both hosts (NFS / iSCSI / Ceph).
          This dialog does not copy disks; storage migration is a separate
          flow.
        </div>
      </div>

      <footer>
        <button class="btn" onclick={close}>Cancel</button>
        <button class="btn primary" onclick={startMigration} disabled={candidates.length === 0}>Start migration</button>
      </footer>
    {:else}
      <div class="progress">
        <div class="phase">
          Phase: <strong>{progress?.phase ?? "starting"}</strong>
          {#if percent != null}<span class="muted small">· {percent.toFixed(1)}%</span>{/if}
        </div>
        {#if percent != null}
          <div class="bar"><div class="bar-fill" style="width: {percent}%"></div></div>
        {:else}
          <div class="bar indet"><div class="bar-fill"></div></div>
        {/if}
        <table class="stats">
          <tbody>
            <tr><td>Data</td><td>{fmtBytes(progress?.data_processed)} / {fmtBytes(progress?.data_total)}</td></tr>
            <tr><td>Memory</td><td>{fmtBytes(progress?.mem_processed)} / {fmtBytes(progress?.mem_total)}</td></tr>
            <tr><td>Elapsed</td><td>{fmtMs(progress?.time_elapsed_ms)}</td></tr>
            <tr><td>Remaining</td><td>{fmtMs(progress?.time_remaining_ms)}</td></tr>
            <tr><td>Downtime</td><td>{fmtMs(progress?.downtime_ms)}</td></tr>
          </tbody>
        </table>
        {#if progress?.error}<div class="warn err">{progress.error}</div>{/if}
        {#if err}<div class="warn err">{err}</div>{/if}
      </div>

      <footer>
        {#if running}
          <button class="btn danger" onclick={cancel}>Cancel migration</button>
          <span class="muted small">cancellation aborts the in-flight transfer; the guest stays on the source</span>
        {:else}
          <button class="btn" onclick={close}>Close</button>
        {/if}
      </footer>
    {/if}
  </div>
</div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.6);
    display: flex; align-items: center; justify-content: center; z-index: 200; padding: 20px; }
  .dialog { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 12px; width: 600px; max-width: 100%; max-height: 90vh;
    display: flex; flex-direction: column; box-shadow: 0 12px 40px rgba(0,0,0,0.5); }
  header { padding: 14px 18px; border-bottom: 1px solid var(--border);
    display: flex; align-items: center; justify-content: space-between; }
  header h3 { margin: 0; font-size: 14px; }
  .x { background: none; border: none; color: var(--text-muted); font-size: 22px; cursor: pointer; }
  .x:disabled { opacity: 0.3; cursor: not-allowed; }

  .form, .progress { padding: 16px 18px; display: flex; flex-direction: column; gap: 12px; overflow-y: auto; }
  .form label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; }
  .form label > span { color: var(--text-muted); font-size: 11px;
    text-transform: uppercase; letter-spacing: 0.05em; }
  .form select, .form input[type=number] {
    background: var(--bg-button); color: var(--text); border: 1px solid var(--border);
    border-radius: 4px; padding: 6px 8px; font-family: inherit; font-size: 13px;
  }
  .flags { border: 1px solid var(--border); border-radius: 6px; padding: 8px 12px;
    display: flex; flex-direction: column; gap: 4px; }
  .flags legend { padding: 0 6px; font-size: 11px; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em; }
  .toggle { flex-direction: row !important; align-items: center; gap: 8px; font-size: 13px; }
  .toggle span { text-transform: none !important; letter-spacing: 0 !important;
    color: var(--text) !important; font-size: 13px !important; }
  .warn { padding: 8px 10px; background: rgba(251,191,36,0.1); border: 1px solid rgba(251,191,36,0.3);
    color: #fbbf24; border-radius: 4px; font-size: 12px; }
  .warn.err { background: rgba(239,68,68,0.1); border-color: rgba(239,68,68,0.3); color: #ef4444; }
  .muted { color: var(--text-muted); }
  .small { font-size: 11px; }

  .phase { font-size: 13px; }
  .bar { height: 8px; background: rgba(0,0,0,0.3); border-radius: 4px; overflow: hidden; }
  .bar-fill { height: 100%; background: var(--accent); transition: width 0.3s; }
  .bar.indet .bar-fill { width: 30%; animation: ind 1.5s linear infinite; }
  @keyframes ind { 0% { margin-left: -30%; } 100% { margin-left: 100%; } }

  .stats { width: 100%; font-size: 12px; }
  .stats td { padding: 3px 8px; }
  .stats td:first-child { color: var(--text-muted); width: 110px; }

  footer { padding: 12px 18px; border-top: 1px solid var(--border);
    display: flex; gap: 8px; align-items: center; justify-content: flex-end; }
  footer .muted.small { margin-right: auto; max-width: 350px; line-height: 1.3; }
  .btn { padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; cursor: pointer; font-family: inherit; }
  .btn.primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn.danger { color: #fca5a5; }
  .btn.danger:hover { background: #7f1d1d; border-color: #7f1d1d; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
