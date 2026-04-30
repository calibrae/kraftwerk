<script>
  import { invoke } from "@tauri-apps/api/core";

  let { open = $bindable(false), info = null, onAccept = () => {}, onCancel = () => {} } = $props();

  let busy = $state(false);
  let err = $state(null);

  async function accept() {
    if (busy || !info?.keyscan_line) return;
    busy = true;
    err = null;
    try {
      if (info.status === "changed") {
        await invoke("forget_host_key", { host: info.host, port: info.port });
      }
      await invoke("accept_host_key", { keyscanLine: info.keyscan_line });
      open = false;
      onAccept();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  function cancel() {
    if (busy) return;
    open = false;
    onCancel();
  }

  function onKeyDown(e) {
    if (e.key === "Escape") cancel();
  }

  let title = $derived(
    info?.status === "changed"
      ? "Host key has CHANGED — possible MITM"
      : info?.status === "new"
        ? "Trust new host key?"
        : "SSH host key check"
  );
</script>

{#if open && info}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="backdrop" onclick={cancel} onkeydown={onKeyDown} role="dialog" aria-modal="true">
    <div class="dialog" class:danger={info.status === "changed"} onclick={(e) => e.stopPropagation()} onkeydown={onKeyDown} role="document">
      <h3>{title}</h3>

      {#if info.status === "changed"}
        <div class="warn">
          The host <code>{info.host}{info.port !== 22 ? `:${info.port}` : ""}</code> is presenting a different SSH key than what's stored in <code>~/.ssh/known_hosts</code>.
          OpenSSH gives this warning when a server is re-installed — but it is also exactly what a man-in-the-middle attack looks like.
          Only proceed if you know the host was rebuilt.
        </div>
      {:else}
        <p class="muted">
          Trusting this key is a one-time decision per host. The fingerprint below will be appended to <code>~/.ssh/known_hosts</code>; future connections won't prompt.
        </p>
      {/if}

      <dl>
        <dt>Host</dt><dd class="mono">{info.host}{info.port !== 22 ? `:${info.port}` : ""}</dd>
        <dt>Key type</dt><dd class="mono">{info.key_type ?? "?"}</dd>
        <dt>Fingerprint</dt><dd class="mono fp">{info.fingerprint ?? "?"}</dd>
      </dl>

      {#if err}
        <pre class="error">{err}</pre>
      {/if}

      <div class="actions">
        <button type="button" class="btn-cancel" onclick={cancel} disabled={busy}>Cancel</button>
        <button type="button" class={info.status === "changed" ? "btn-danger" : "btn-primary"} onclick={accept} disabled={busy}>
          {busy ? "Saving…" : info.status === "changed" ? "Replace key and trust" : "Trust this key"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex; align-items: center; justify-content: center;
    z-index: 150;
  }
  .dialog {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 24px;
    width: 520px;
    max-width: 92vw;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
  }
  .dialog.danger {
    border-color: rgba(239, 68, 68, 0.5);
    box-shadow: 0 8px 32px rgba(239, 68, 68, 0.25);
  }
  h3 { margin: 0 0 12px; font-size: 16px; font-weight: 600; }
  .dialog.danger h3 { color: #ef4444; }
  p.muted { color: var(--text-muted); font-size: 12px; margin: 0 0 12px; }
  .warn {
    margin: 0 0 12px;
    padding: 10px 12px;
    background: rgba(239, 68, 68, 0.10);
    border: 1px solid rgba(239, 68, 68, 0.35);
    border-radius: 6px;
    color: #ef4444;
    font-size: 12px;
    line-height: 1.4;
  }
  dl {
    margin: 8px 0 16px;
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 6px 12px;
    font-size: 12px;
  }
  dt { color: var(--text-muted); }
  dd { margin: 0; }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
  dd.fp { word-break: break-all; }
  code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; background: var(--bg-input); padding: 1px 4px; border-radius: 3px; }
  .error {
    margin: 0 0 12px;
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.10);
    border: 1px solid rgba(239, 68, 68, 0.30);
    border-radius: 6px;
    color: #ef4444;
    font-size: 11px;
    white-space: pre-wrap;
  }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .btn-cancel, .btn-primary, .btn-danger {
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
  .btn-danger { background: #dc2626; color: white; border-color: #dc2626; }
  .btn-danger:hover:not(:disabled) { filter: brightness(1.1); }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
