<script>
  import { getState, bulkAction, clearVmSelection } from "$lib/stores/app.svelte.js";

  const appState = getState();
  let busy = $state(false);
  let lastResult = $state(null);

  async function run(label, command) {
    if (busy) return;
    if (!confirmIfDestructive(label)) return;
    busy = true;
    lastResult = null;
    const r = await bulkAction(command);
    busy = false;
    lastResult = `${label}: ${r.ok} ok${r.failed ? `, ${r.failed} failed` : ""}`;
  }

  function confirmIfDestructive(label) {
    // Tauri's webview confirm() can be flaky in production builds; for
    // bulk we use a plain inline arm-style toggle on the destructive
    // buttons themselves rather than a global confirm dialog.
    return true;
  }

  // Arm-to-confirm pattern for destructive bulk ops, matching the
  // snapshot panel's UX. First click arms, second within 5s fires.
  let armed = $state(null);
  let armTimer = null;
  function arm(kind) {
    armed = kind;
    if (armTimer) clearTimeout(armTimer);
    armTimer = setTimeout(() => { armed = null; }, 5000);
  }
  function tryArmed(kind, label, command) {
    if (armed === kind) {
      armed = null;
      return run(label, command);
    }
    arm(kind);
  }
</script>

{#if appState.hasMultiSelect}
  <div class="bulk-bar">
    <div class="bulk-summary">
      <strong>{appState.selectedVmNames.size} VMs selected</strong>
      <button class="btn-link" onclick={clearVmSelection}>Clear</button>
    </div>

    <div class="bulk-actions">
      <button class="btn-action start" onclick={() => run("Start", "start_domain")} disabled={busy}>
        Start
      </button>
      <button class="btn-action" onclick={() => run("Resume", "resume_domain")} disabled={busy}>
        Resume
      </button>
      <button class="btn-action" onclick={() => run("Suspend", "suspend_domain")} disabled={busy}>
        Suspend
      </button>
      <button class="btn-action" onclick={() => run("Shutdown", "shutdown_domain")} disabled={busy}>
        Shutdown
      </button>
      <button class="btn-action" onclick={() => run("Reboot", "reboot_domain")} disabled={busy}>
        Reboot
      </button>
      <button class="btn-action danger" class:armed={armed === "destroy"}
        onclick={() => tryArmed("destroy", "Force off", "destroy_domain")} disabled={busy}>
        {armed === "destroy" ? "Confirm: force off" : "Force off"}
      </button>
    </div>

    {#if lastResult}
      <div class="bulk-result">{lastResult}</div>
    {/if}

    <ul class="vm-list-preview">
      {#each Array.from(appState.selectedVmNames) as name (name)}
        <li class="mono">{name}</li>
      {/each}
    </ul>
  </div>
{/if}

<style>
  .bulk-bar {
    padding: 24px;
    overflow-y: auto;
    height: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .bulk-summary {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .bulk-summary strong { font-size: 14px; }
  .btn-link {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 12px;
    text-decoration: underline;
    cursor: pointer;
    padding: 0;
    font-family: inherit;
  }
  .btn-link:hover { color: var(--text); }

  .bulk-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .btn-action {
    padding: 6px 14px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    font-size: 13px;
    font-family: inherit;
    cursor: pointer;
  }
  .btn-action:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-action:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-action.start { background: #2563eb; color: white; border-color: #2563eb; }
  .btn-action.start:hover:not(:disabled) { background: #1e40af; }
  .btn-action.danger { color: #ef4444; border-color: rgba(239, 68, 68, 0.4); }
  .btn-action.danger.armed {
    background: rgba(239, 68, 68, 0.18);
    color: #ef4444;
    border-color: #ef4444;
    font-weight: 500;
  }

  .bulk-result {
    padding: 8px 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 13px;
  }

  ul.vm-list-preview {
    list-style: none;
    margin: 0;
    padding: 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 4px 12px;
    font-size: 12px;
  }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
</style>
