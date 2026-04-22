<script>
  import { addConnection, connect, updateConnection } from "$lib/stores/app.svelte.js";

  // `editing` null = add mode; non-null SavedConnection = edit mode.
  let { open = $bindable(false), editing = $bindable(null) } = $props();

  let displayName = $state("");
  let uri = $state("qemu+ssh:///system");
  let authType = $state("ssh_agent");
  let saving = $state(false);
  let err = $state(null);

  // When `editing` becomes non-null (dialog opened for edit), prefill fields.
  $effect(() => {
    if (editing) {
      displayName = editing.display_name;
      uri = editing.uri;
      authType = editing.auth_type;
      err = null;
    }
  });

  function reset() {
    displayName = "";
    uri = "qemu+ssh:///system";
    authType = "ssh_agent";
    err = null;
    saving = false;
  }

  function close() {
    open = false;
    editing = null;
    reset();
  }

  async function handleSubmit(e) {
    e.preventDefault();
    if (!displayName.trim() || !uri.trim()) return;

    saving = true;
    err = null;
    try {
      if (editing) {
        await updateConnection(editing.id, displayName.trim(), uri.trim(), authType);
      } else {
        const conn = await addConnection(displayName.trim(), uri.trim(), authType);
        await connect(conn.id);
      }
      close();
    } catch (ex) {
      err = ex?.message || String(ex);
      saving = false;
    }
  }

  function handleKeydown(e) {
    if (e.key === "Escape") close();
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="backdrop" onclick={close} onkeydown={handleKeydown} role="dialog" aria-modal="true">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div class="dialog" onclick={(e) => e.stopPropagation()} onkeydown={handleKeydown}>
      <h3>{editing ? "Edit Connection" : "New Connection"}</h3>

      <form onsubmit={handleSubmit}>
        <label>
          <span>Display Name</span>
          <input type="text" bind:value={displayName} placeholder="My Server" autofocus />
        </label>

        <label>
          <span>URI</span>
          <input type="text" bind:value={uri} placeholder="qemu+ssh://user@host/system" />
        </label>

        <label>
          <span>Authentication</span>
          <select bind:value={authType}>
            <option value="ssh_agent">SSH Agent</option>
            <option value="ssh_key">SSH Key</option>
            <option value="password">Password</option>
          </select>
        </label>

        {#if err}
          <div class="error">{err}</div>
        {/if}

        <div class="actions">
          <button type="button" class="btn-cancel" onclick={close} disabled={saving}>Cancel</button>
          <button type="submit" class="btn-connect" disabled={saving || !displayName.trim() || !uri.trim()}>
            {saving ? (editing ? "Saving..." : "Connecting...") : (editing ? "Save" : "Connect")}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .dialog {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 24px;
    width: 400px;
    max-width: 90vw;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
  }

  h3 {
    margin: 0 0 20px;
    font-size: 16px;
    font-weight: 600;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  label span {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-muted);
  }

  input, select {
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text);
    font-size: 13px;
    font-family: inherit;
    outline: none;
  }

  input:focus, select:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-dim);
  }

  .error {
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    color: #ef4444;
    font-size: 12px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }

  .btn-cancel, .btn-connect {
    padding: 8px 16px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
  }

  .btn-cancel {
    background: var(--bg-button);
    color: var(--text);
  }

  .btn-cancel:hover { background: var(--bg-hover); }

  .btn-connect {
    background: var(--accent);
    color: white;
    border-color: var(--accent);
  }

  .btn-connect:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-connect:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
