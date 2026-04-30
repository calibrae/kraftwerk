<script>
  /*
   * OVA / OVF import dialog. Takes a local .ova path, parses the OVF
   * for preview, then triggers an import that streams each VMDK from
   * the local tar through SSH+qemu-img into the chosen pool as qcow2,
   * and defines a new domain XML.
   *
   * No file picker — Tauri's dialog plugin isn't wired into this app
   * yet, so the operator pastes an absolute path. Drag-drop could be
   * a later polish step.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState, refreshVms, refreshPools } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false) } = $props();
  const appState = getState();

  let ovaPath = $state("");
  let metadata = $state(null);
  let inspectErr = $state(null);
  let busy = $state(false);
  let runErr = $state(null);
  let result = $state(null);

  let pools = $derived(appState.pools.filter(p => p.is_active && p.pool_type === "dir"));
  let networks = $derived(appState.networks.filter(n => n.is_active));

  let poolName = $state("");
  let networkName = $state("");
  let targetName = $state("");

  $effect(() => {
    if (open) {
      ovaPath = "";
      metadata = null;
      inspectErr = null;
      runErr = null;
      result = null;
      poolName = pools[0]?.name ?? "";
      networkName = networks[0]?.name ?? "";
      targetName = "";
    }
  });

  async function inspect() {
    inspectErr = null;
    metadata = null;
    if (!ovaPath.trim()) return;
    try {
      metadata = await invoke("inspect_ova", { ovaPath: ovaPath.trim() });
      targetName = metadata.name || "imported-vm";
    } catch (e) {
      inspectErr = e?.message || String(e);
    }
  }

  async function runImport() {
    if (busy) return;
    busy = true;
    runErr = null;
    try {
      const newName = await invoke("import_ova", {
        ovaPath: ovaPath.trim(),
        poolName,
        targetName: targetName.trim() || null,
        networkName: networkName || null,
      });
      result = newName;
      await refreshPools();
      await refreshVms();
    } catch (e) {
      runErr = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  function close() { if (!busy) open = false; }

  function fmtBytes(n) {
    if (n == null) return "—";
    const u = ["B", "KiB", "MiB", "GiB"];
    let v = Number(n), i = 0;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return `${v.toFixed(v >= 100 ? 0 : 1)} ${u[i]}`;
  }
</script>

{#if open}
<div class="backdrop" onclick={close} role="presentation">
  <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
    <header>
      <h3>Import OVA / OVF</h3>
      <button class="x" onclick={close} disabled={busy}>×</button>
    </header>

    <div class="body">
      {#if !result}
        <label>
          <span>Local .ova path</span>
          <div class="row">
            <input type="text" bind:value={ovaPath} placeholder="/Users/you/Downloads/appliance.ova" />
            <button class="btn" onclick={inspect} disabled={!ovaPath.trim() || busy}>Inspect</button>
          </div>
        </label>

        {#if inspectErr}<div class="err">{inspectErr}</div>{/if}

        {#if metadata}
          <fieldset>
            <legend>Detected</legend>
            <dl>
              <dt>Name</dt><dd>{metadata.name || "—"}</dd>
              <dt>vCPUs</dt><dd>{metadata.vcpus ?? "(default)"}</dd>
              <dt>Memory</dt><dd>{metadata.memory_mib ? `${metadata.memory_mib} MiB` : "(default)"}</dd>
              <dt>Disks</dt>
              <dd>
                <ul class="dlist">
                  {#each metadata.disks as d}
                    <li><code>{d.file_href ?? d.disk_id}</code> — {fmtBytes(d.capacity_bytes)}</li>
                  {/each}
                </ul>
              </dd>
              {#if metadata.networks?.length}
                <dt>Nets</dt><dd>{metadata.networks.join(", ")}</dd>
              {/if}
              {#if metadata.guest_os}
                <dt>Guest OS</dt><dd>{metadata.guest_os}</dd>
              {/if}
            </dl>
          </fieldset>

          <label>
            <span>Target name</span>
            <input type="text" bind:value={targetName} />
          </label>
          <label>
            <span>Pool (must be a dir-type, active pool)</span>
            <select bind:value={poolName}>
              {#each pools as p}<option value={p.name}>{p.name} — {p.target_path}</option>{/each}
            </select>
          </label>
          <label>
            <span>Network (one libvirt network for all NICs)</span>
            <select bind:value={networkName}>
              <option value="">(no network)</option>
              {#each networks as n}<option value={n.name}>{n.name}</option>{/each}
            </select>
          </label>

          {#if runErr}<div class="err">{runErr}</div>{/if}

          <p class="hint">
            Each VMDK is converted to qcow2 by <code>qemu-img convert</code>
            on the hypervisor host while we stream the tar entry over
            SSH stdin. Disks land directly in the pool target dir; no
            intermediate copy on either side.
          </p>
        {/if}
      {:else}
        <div class="ok">
          Imported as <strong>{result}</strong>. Defined but not started.
        </div>
      {/if}
    </div>

    <footer>
      <button class="btn" onclick={close} disabled={busy}>{result ? "Close" : "Cancel"}</button>
      {#if !result}
        <button class="btn primary" onclick={runImport}
          disabled={busy || !metadata || !poolName || !targetName.trim()}>
          {busy ? "Importing…" : "Import"}
        </button>
      {/if}
    </footer>
  </div>
</div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.6);
    display: flex; align-items: center; justify-content: center; z-index: 200; padding: 20px; }
  .dialog { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 12px; width: 580px; max-width: 100%; max-height: 90vh;
    display: flex; flex-direction: column; box-shadow: 0 12px 40px rgba(0,0,0,0.5); }
  header { padding: 14px 18px; border-bottom: 1px solid var(--border);
    display: flex; align-items: center; justify-content: space-between; }
  header h3 { margin: 0; font-size: 14px; }
  .x { background: none; border: none; color: var(--text-muted); font-size: 22px; cursor: pointer; }
  .body { padding: 16px 18px; display: flex; flex-direction: column; gap: 12px; overflow-y: auto; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; }
  label > span { color: var(--text-muted); font-size: 11px;
    text-transform: uppercase; letter-spacing: 0.05em; }
  .row { display: flex; gap: 8px; }
  .row input { flex: 1; }
  input[type=text], select {
    background: var(--bg-button); color: var(--text); border: 1px solid var(--border);
    border-radius: 4px; padding: 6px 8px; font-family: inherit; font-size: 13px;
  }
  fieldset { border: 1px solid var(--border); border-radius: 6px; padding: 10px 12px; }
  fieldset legend { padding: 0 6px; font-size: 11px; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em; }
  dl { display: grid; grid-template-columns: 80px 1fr; gap: 4px 12px; margin: 0; font-size: 12px; }
  dt { color: var(--text-muted); }
  dd { margin: 0; }
  .dlist { list-style: none; padding: 0; margin: 0; }
  .dlist code { font-family: 'SF Mono', monospace; font-size: 11px; }
  .err { padding: 8px 10px; background: rgba(239,68,68,0.1);
    border: 1px solid rgba(239,68,68,0.3); color: #ef4444; font-size: 12px;
    border-radius: 4px; white-space: pre-wrap; max-height: 160px; overflow: auto; }
  .ok { padding: 16px; background: rgba(16,185,129,0.1); color: #34d399;
    border: 1px solid rgba(16,185,129,0.3); border-radius: 6px; font-size: 13px; }
  .hint { font-size: 11px; color: var(--text-muted); margin: 0; line-height: 1.5; }
  .hint code { background: rgba(0,0,0,0.3); padding: 1px 4px; border-radius: 3px; font-size: 10px; }
  footer { padding: 12px 18px; border-top: 1px solid var(--border);
    display: flex; gap: 8px; justify-content: flex-end; }
  .btn { padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; cursor: pointer; font-family: inherit; }
  .btn.primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
