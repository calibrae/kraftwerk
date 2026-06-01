<script>
  /*
   * In-process log viewer. Tails the backend's ring buffer via the
   * get_logs Tauri command, with a runtime verbose toggle that flips
   * the global log level between Info (default) and Debug.
   *
   * Polls every 1.5s while open. Pauses polling when closed.
   */
  import { invoke } from "$lib/invoke.js";

  let { open = $bindable(false) } = $props();

  let entries = $state([]);
  let level = $state("info");
  let minLevelFilter = $state(""); // "" = no filter
  let autoScroll = $state(true);
  let lastTs = $state(null);
  let pollHandle = null;
  let err = $state(null);

  const LEVELS = [
    { value: "", label: "All" },
    { value: "error", label: "Error" },
    { value: "warn", label: "Warn+" },
    { value: "info", label: "Info+" },
    { value: "debug", label: "Debug+" },
    { value: "trace", label: "Trace+" },
  ];

  async function poll() {
    try {
      const fresh = await invoke("get_logs", {
        afterTsMs: lastTs,
        minLevel: minLevelFilter || null,
      });
      if (fresh?.length) {
        entries = lastTs == null ? fresh : [...entries, ...fresh].slice(-2000);
        lastTs = fresh[fresh.length - 1].ts_ms;
      }
      err = null;
    } catch (e) {
      err = e?.message || String(e);
    }
  }

  async function refreshLevel() {
    try {
      level = await invoke("get_log_level");
    } catch (e) {
      // pre-init; ignore.
    }
  }

  async function setLevel(next) {
    try {
      await invoke("set_log_level", { level: next });
      level = next;
    } catch (e) {
      err = e?.message || String(e);
    }
  }

  async function toggleVerbose() {
    await setLevel(level === "debug" || level === "trace" ? "info" : "debug");
  }

  async function clearAll() {
    try {
      await invoke("clear_logs");
      entries = [];
      lastTs = null;
    } catch (e) {
      err = e?.message || String(e);
    }
  }

  // Track open transitions so the effect only runs side-effects on
  // an actual false→true / true→false flip — Svelte 5 may re-run
  // `$effect` more than once for the same value of `open` (writes to
  // state inside the effect, child rerenders, …), and we MUST NOT
  // kick off a second concurrent poll() — that races with the first
  // and duplicates entries.
  let wasOpen = false;
  $effect(() => {
    const isOpen = !!open;
    if (isOpen && !wasOpen) {
      entries = [];
      lastTs = null;
      refreshLevel();
      poll();
      pollHandle = setInterval(poll, 1500);
    } else if (!isOpen && wasOpen && pollHandle) {
      clearInterval(pollHandle);
      pollHandle = null;
    }
    wasOpen = isOpen;
  });

  function close() { open = false; }

  function fmtTime(ms) {
    const d = new Date(ms);
    return d.toLocaleTimeString("en-GB", { hour12: false }) + "." + String(d.getMilliseconds()).padStart(3, "0");
  }
  function levelClass(l) { return `lvl lvl-${l}`; }

  // Auto-scroll-to-bottom when new entries arrive AND autoScroll is on.
  let listEl;
  $effect(() => {
    void entries;
    if (autoScroll && listEl) listEl.scrollTop = listEl.scrollHeight;
  });
</script>

{#if open}
<div class="backdrop" onclick={close} role="presentation">
  <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
    <header>
      <h3>Application logs</h3>
      <div class="header-controls">
        <label class="toggle">
          <input type="checkbox" checked={level === "debug" || level === "trace"} onchange={toggleVerbose} />
          <span>Verbose (debug)</span>
        </label>
        <label class="level-pick">
          <span>Show</span>
          <select bind:value={minLevelFilter}>
            {#each LEVELS as l}<option value={l.value}>{l.label}</option>{/each}
          </select>
        </label>
        <label class="toggle">
          <input type="checkbox" bind:checked={autoScroll} />
          <span>Auto-scroll</span>
        </label>
        <button class="btn-tiny" onclick={clearAll}>Clear</button>
        <button class="x" onclick={close}>×</button>
      </div>
    </header>

    {#if err}<div class="err">{err}</div>{/if}

    <div class="list" bind:this={listEl}>
      {#if entries.length === 0}
        <div class="muted center">No log entries yet — interact with the app to generate some.</div>
      {:else}
        {#each entries as e (e.ts_ms + ":" + e.target + ":" + e.message)}
          <div class="row">
            <span class="ts">{fmtTime(e.ts_ms)}</span>
            <span class={levelClass(e.level)}>{e.level}</span>
            <span class="target">{e.target}</span>
            <span class="msg">{e.message}</span>
          </div>
        {/each}
      {/if}
    </div>

    <footer>
      <span class="muted small">level: <code>{level}</code> · buffered: {entries.length}</span>
    </footer>
  </div>
</div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.6);
    display: flex; align-items: center; justify-content: center; z-index: 200; padding: 20px; }
  .dialog { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 12px; width: 1000px; max-width: 100%; max-height: 90vh;
    display: flex; flex-direction: column; box-shadow: 0 12px 40px rgba(0,0,0,0.5); overflow: hidden; }
  header { padding: 12px 16px; border-bottom: 1px solid var(--border);
    display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  header h3 { margin: 0; font-size: 14px; }
  .header-controls { display: flex; align-items: center; gap: 12px; }
  .toggle { display: flex; align-items: center; gap: 6px; font-size: 12px; cursor: pointer; }
  .toggle input { margin: 0; }
  .level-pick { display: flex; align-items: center; gap: 4px; font-size: 11px; color: var(--text-muted); }
  .level-pick select {
    background: var(--bg-button); color: var(--text); border: 1px solid var(--border);
    border-radius: 4px; padding: 2px 6px; font-size: 12px; font-family: inherit;
  }
  .btn-tiny { padding: 2px 10px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--bg-button); color: var(--text); font-size: 11px; cursor: pointer; font-family: inherit; }
  .x { background: none; border: none; color: var(--text-muted); font-size: 20px; cursor: pointer; padding: 0 4px; }

  .list { flex: 1; overflow-y: auto; padding: 4px 0; font-family: 'SF Mono', 'Menlo', monospace; font-size: 11px; }
  .row { display: grid; grid-template-columns: 90px 50px 200px 1fr; gap: 8px;
    padding: 2px 16px; border-bottom: 1px solid rgba(255,255,255,0.04); }
  .row:hover { background: rgba(255,255,255,0.03); }
  .ts { color: var(--text-muted); }
  .target { color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .msg { white-space: pre-wrap; word-break: break-word; }
  .lvl { font-weight: 700; text-transform: uppercase; }
  .lvl-error { color: #ef4444; }
  .lvl-warn { color: #fbbf24; }
  .lvl-info { color: #60a5fa; }
  .lvl-debug { color: #34d399; }
  .lvl-trace { color: #a78bfa; }

  footer { padding: 8px 16px; border-top: 1px solid var(--border);
    display: flex; align-items: center; justify-content: space-between; }
  .muted { color: var(--text-muted); font-size: 11px; }
  .center { text-align: center; padding: 24px; }
  .small { font-size: 11px; }
  .err { padding: 8px 16px; background: rgba(239,68,68,0.1); color: #ef4444; font-size: 12px;
    border-bottom: 1px solid rgba(239,68,68,0.3); }
  code { background: rgba(0,0,0,0.3); padding: 0 4px; border-radius: 3px; font-family: 'SF Mono', monospace; }
</style>
