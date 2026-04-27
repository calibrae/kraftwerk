<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, tick } from "svelte";

  let { vmName } = $props();

  let log = $state("");
  let lineCount = $state(200);
  let loading = $state(false);
  let err = $state(null);
  let logEl = $state(null);
  let autoScroll = $state(true);

  async function load() {
    if (!vmName || loading) return;
    loading = true;
    err = null;
    try {
      log = await invoke("get_qemu_log", { name: vmName, lines: lineCount });
      if (autoScroll) {
        await tick();
        if (logEl) logEl.scrollTop = logEl.scrollHeight;
      }
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  $effect(() => {
    if (vmName) load();
  });
</script>

<div class="panel">
  <header class="ph">
    <h3>qemu log <span class="muted small">/var/log/libvirt/qemu/{vmName}.log</span></h3>
    <div class="actions">
      <select bind:value={lineCount} disabled={loading}>
        <option value={100}>last 100</option>
        <option value={200}>last 200</option>
        <option value={500}>last 500</option>
        <option value={1000}>last 1000</option>
        <option value={5000}>last 5000</option>
      </select>
      <label class="cb">
        <input type="checkbox" bind:checked={autoScroll} />
        <span>follow</span>
      </label>
      <button class="btn-small" onclick={load} disabled={loading}>
        {loading ? "Loading..." : "Refresh"}
      </button>
    </div>
  </header>

  {#if err}
    <pre class="error">{err}</pre>
  {/if}

  <pre class="log mono" bind:this={logEl}>{log || (loading ? "Loading…" : "No log content yet.")}</pre>
</div>

<style>
  .panel { padding: 16px; display: flex; flex-direction: column; height: 100%; box-sizing: border-box; gap: 10px; }
  .ph { display: flex; justify-content: space-between; align-items: center; gap: 12px; flex-wrap: wrap; }
  .ph h3 { margin: 0; font-size: 14px; font-weight: 600; display: flex; align-items: center; gap: 8px; }
  .muted { color: var(--text-muted); font-weight: normal; }
  .small { font-size: 11px; }
  .actions { display: flex; gap: 8px; align-items: center; }
  .cb { display: flex; align-items: center; gap: 4px; font-size: 12px; }
  .cb input { margin: 0; }
  select, .btn-small {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
  }
  .btn-small:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-small:disabled { opacity: 0.5; cursor: not-allowed; }

  pre.log {
    flex: 1;
    overflow: auto;
    margin: 0;
    padding: 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 11px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    min-height: 300px;
  }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }

  .error {
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
