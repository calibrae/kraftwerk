<script>
  import { invoke } from "@tauri-apps/api/core";
  import { refreshVms } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false), source = null } = $props();

  let targetName = $state("");
  let randomizeMacs = $state(true);
  let startAfter = $state(false);
  let busy = $state(false);
  let err = $state(null);

  $effect(() => {
    if (open && source) {
      targetName = `${source.name}-clone`;
      err = null;
      busy = false;
    }
  });

  function close() {
    open = false;
    targetName = "";
    err = null;
  }

  async function doClone(e) {
    e.preventDefault();
    if (!targetName.trim() || busy) return;
    busy = true;
    err = null;
    try {
      await invoke("clone_domain", {
        source: source.name,
        targetName: targetName.trim(),
        randomizeMacs,
        startAfter,
      });
      await refreshVms();
      close();
    } catch (ex) {
      err = ex?.message || String(ex);
      busy = false;
    }
  }

  function onKeyDown(e) {
    if (e.key === "Escape") close();
  }
</script>

{#if open && source}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="backdrop" onclick={close} onkeydown={onKeyDown} role="dialog" aria-modal="true">
    <div class="dialog" onclick={(e) => e.stopPropagation()} onkeydown={onKeyDown} role="document">
      <h3>Clone {source.name}</h3>
      <form onsubmit={doClone}>
        <label>
          <span>New VM name</span>
          <input type="text" bind:value={targetName} placeholder="my-clone" autofocus />
        </label>
        <label class="cb">
          <input type="checkbox" bind:checked={randomizeMacs} />
          <span>Strip MAC addresses (libvirt assigns fresh ones)</span>
        </label>
        <label class="cb">
          <input type="checkbox" bind:checked={startAfter} />
          <span>Start the clone after creation</span>
        </label>

        <p class="hint">
          Only shut-off VMs can be cloned safely. Each r/w disk is full-copy duplicated
          in the same storage pool via <code>virStorageVolCreateXMLFrom</code>; CD-ROMs
          and readonly disks pass through. UUID is regenerated automatically.
        </p>

        {#if err}
          <pre class="error">{err}</pre>
        {/if}

        <div class="actions">
          <button type="button" class="btn-cancel" onclick={close} disabled={busy}>Cancel</button>
          <button type="submit" class="btn-primary" disabled={busy || !targetName.trim()}>
            {busy ? "Cloning..." : "Clone"}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex; align-items: center; justify-content: center;
    z-index: 100;
  }
  .dialog {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 24px;
    width: 460px;
    max-width: 90vw;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
  }
  h3 { margin: 0 0 20px; font-size: 16px; font-weight: 600; }
  form { display: flex; flex-direction: column; gap: 12px; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; }
  label span { color: var(--text-muted); font-weight: 500; }
  label.cb { flex-direction: row; align-items: center; gap: 8px; }
  label.cb input { margin: 0; }
  label.cb span { color: var(--text); font-weight: normal; }
  input[type="text"] {
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text);
    font-size: 13px;
    font-family: inherit;
    outline: none;
  }
  input[type="text"]:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-dim);
  }
  .hint { font-size: 11px; color: var(--text-muted); margin: 0; }
  .hint code {
    font-size: 11px;
    background: var(--bg-input);
    padding: 1px 4px;
    border-radius: 3px;
  }
  .error {
    margin: 0;
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    color: #ef4444;
    font-size: 12px;
    white-space: pre-wrap;
    max-height: 160px;
    overflow: auto;
  }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .btn-cancel, .btn-primary {
    padding: 8px 16px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
  }
  .btn-cancel { background: var(--bg-button); color: var(--text); }
  .btn-cancel:hover { background: var(--bg-hover); }
  .btn-primary { background: var(--accent); color: white; border-color: var(--accent); }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled, .btn-cancel:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
