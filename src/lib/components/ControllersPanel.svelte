<script>
  /*
   * Controllers (Round H) editor.
   *
   * Groups controllers by type: USB / SCSI / virtio-serial / IDE-SATA /
   * PCI (read-only). Edit dialog per type with the right model list.
   *
   * Persistent-only — most controller model changes require restart.
   */
  import { invoke } from "@tauri-apps/api/core";

  let { vmName } = $props();

  let controllers = $state([]);
  let loading = $state(true);
  let busy = $state(false);
  let err = $state(null);
  let editing = $state(null); // { mode: 'edit'|'add', cfg, original }
  let lastChangeAt = $state(null);

  // Hardcoded model fallbacks — may be overridden from DomainCaps later.
  const USB_MODELS = [
    "qemu-xhci", "nec-xhci",
    "ehci", "ich9-ehci1",
    "ich9-uhci1", "ich9-uhci2", "ich9-uhci3",
    "piix3-uhci", "piix4-uhci", "vt82c686b-uhci",
    "none",
  ];
  const SCSI_MODELS = [
    "virtio-scsi", "virtio-transitional", "virtio-non-transitional",
    "lsilogic", "lsisas1068", "lsisas1078", "buslogic", "ibmvscsi", "vmpvscsi",
  ];
  const VSERIAL_MODELS = ["virtio", "virtio-transitional", "virtio-non-transitional"];
  const PCI_MODELS = [
    "pcie-root", "pcie-root-port", "pcie-switch-upstream-port",
    "pcie-switch-downstream-port", "pci-bridge", "dmi-to-pci-bridge",
    "pci-root", "pcie-expander-bus", "pcie-to-pci-bridge",
  ];

  async function reload() {
    loading = true; err = null;
    try {
      controllers = await invoke("list_controllers", { name: vmName });
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { if (vmName) reload(); });

  function groupFor(t) {
    if (t === "usb") return "USB";
    if (t === "scsi") return "SCSI";
    if (t === "virtio-serial") return "virtio-serial";
    if (t === "ide" || t === "sata" || t === "fdc") return "IDE / SATA / FDC";
    if (t === "pci") return "PCI (read-only)";
    return "Other";
  }

  const GROUPS = [
    "USB", "SCSI", "virtio-serial", "IDE / SATA / FDC", "PCI (read-only)", "Other",
  ];

  let grouped = $derived.by(() => {
    const map = {};
    for (const c of controllers) {
      const g = groupFor(c.type ?? c.controller_type);
      (map[g] ||= []).push(c);
    }
    // Stable sort within each group by index
    for (const g of Object.keys(map)) {
      map[g].sort((a, b) => a.index - b.index);
    }
    return map;
  });

  // Normalize for snake_case / renamed `type` in serde output
  function t(c) { return c.type ?? c.controller_type; }

  function modelsFor(type) {
    switch (type) {
      case "usb": return USB_MODELS;
      case "scsi": return SCSI_MODELS;
      case "virtio-serial": return VSERIAL_MODELS;
      case "pci": return PCI_MODELS;
      default: return [];
    }
  }

  function openEdit(cfg) {
    editing = {
      mode: "edit",
      original: JSON.parse(JSON.stringify(cfg)),
      cfg: JSON.parse(JSON.stringify(cfg)),
    };
  }

  function openAdd(type) {
    const usedIdx = controllers
      .filter((c) => t(c) === type)
      .map((c) => c.index);
    let index = 0;
    while (usedIdx.includes(index)) index++;
    editing = {
      mode: "add",
      original: null,
      cfg: {
        type,
        index,
        model: modelsFor(type)[0] ?? null,
        ports: null,
        vectors: null,
        queues: null,
        iothread: null,
        ioeventfd: null,
        event_idx: null,
        chassis: null,
        slot: null,
        bus: null,
        function: null,
        target_port: null,
      },
    };
  }

  function closeDialog() { editing = null; }

  async function save() {
    if (!editing) return;
    busy = true; err = null;
    try {
      const cfg = editing.cfg;
      // Clean empty strings / NaN from numeric inputs
      for (const k of ["ports", "vectors", "queues", "iothread"]) {
        if (cfg[k] === "" || Number.isNaN(cfg[k])) cfg[k] = null;
        if (cfg[k] != null) cfg[k] = Number(cfg[k]);
      }
      if (editing.mode === "edit") {
        await invoke("update_controller", {
          name: vmName,
          controllerType: t(editing.original),
          index: editing.original.index,
          controller: cfg,
        });
      } else {
        await invoke("add_controller", {
          name: vmName,
          controller: cfg,
          live: false,
          config: true,
        });
      }
      lastChangeAt = Date.now();
      editing = null;
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function remove(cfg) {
    if (!confirm(`Remove ${t(cfg)} controller #${cfg.index}?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_controller", {
        name: vmName,
        controllerType: t(cfg),
        index: cfg.index,
        live: false,
        config: true,
      });
      lastChangeAt = Date.now();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  function fmtBdf(c) {
    if (c.bus == null && c.slot == null && c.function == null) return "";
    const hex = (n) => (n ?? 0).toString(16).padStart(2, "0");
    return `${hex(c.bus)}:${hex(c.slot)}.${c.function ?? 0}`;
  }
</script>

<div class="controllers">
  {#if loading}
    <p class="muted">Loading…</p>
  {:else if err}
    <div class="error">{err}</div>
  {:else}
    <div class="notice">
      Changes apply to the persistent config. Most controller changes only take effect on next boot.
    </div>

    {#each GROUPS as g}
      {#if grouped[g]?.length || g === "USB" || g === "SCSI" || g === "virtio-serial"}
        <section>
          <div class="section-head">
            <h3>{g}</h3>
            {#if g === "USB"}
              <button class="btn-tiny" onclick={() => openAdd("usb")} disabled={busy}>+ Add USB</button>
            {:else if g === "SCSI"}
              <button class="btn-tiny" onclick={() => openAdd("scsi")} disabled={busy}>+ Add SCSI</button>
            {:else if g === "virtio-serial"}
              <button class="btn-tiny" onclick={() => openAdd("virtio-serial")} disabled={busy}>+ Add virtio-serial</button>
            {/if}
          </div>

          {#if grouped[g]?.length}
            <table>
              <thead>
                <tr>
                  <th>Type</th>
                  <th>Index</th>
                  <th>Model</th>
                  <th>Ports</th>
                  <th>Queues / IOThread</th>
                  {#if g.startsWith("PCI")}
                    <th>Chassis</th>
                    <th>Addr</th>
                  {/if}
                  <th style="text-align: right;"></th>
                </tr>
              </thead>
              <tbody>
                {#each grouped[g] as c (t(c) + "#" + c.index)}
                  <tr>
                    <td><code>{t(c)}</code></td>
                    <td>{c.index}</td>
                    <td>{c.model ?? "-"}</td>
                    <td>{c.ports ?? "-"}</td>
                    <td>
                      {#if c.queues != null || c.iothread != null}
                        {c.queues ?? "-"} / {c.iothread ?? "-"}
                      {:else}-{/if}
                    </td>
                    {#if g.startsWith("PCI")}
                      <td>{c.chassis ?? "-"}</td>
                      <td><code>{fmtBdf(c)}</code></td>
                    {/if}
                    <td style="text-align: right;">
                      {#if t(c) !== "pci"}
                        <span class="badge-restart">restart required</span>
                        <button class="btn-tiny" onclick={() => openEdit(c)} disabled={busy}>Edit</button>
                        <button class="btn-tiny danger" onclick={() => remove(c)} disabled={busy}>Remove</button>
                      {:else}
                        <span class="muted small">managed by libvirt</span>
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <p class="muted">No {g} controllers.</p>
          {/if}
        </section>
      {/if}
    {/each}
  {/if}

  {#if editing}
    <div class="dialog-backdrop" role="dialog" aria-modal="true">
      <div class="dialog">
        <h3>
          {editing.mode === "add" ? "Add" : "Edit"} {t(editing.cfg)} controller
        </h3>

        <label>
          <span>Index</span>
          <input type="number" min="0" bind:value={editing.cfg.index}
            disabled={editing.mode === "edit" || busy} />
        </label>

        <label>
          <span>Model</span>
          <select bind:value={editing.cfg.model} disabled={busy}>
            <option value={null}>(default)</option>
            {#each modelsFor(t(editing.cfg)) as m}
              <option value={m}>{m}</option>
            {/each}
          </select>
        </label>

        {#if t(editing.cfg) === "usb"}
          <label>
            <span>Ports (xhci, max 15)</span>
            <input type="number" min="0" max="15" bind:value={editing.cfg.ports} disabled={busy} />
          </label>
        {:else if t(editing.cfg) === "virtio-serial"}
          <label>
            <span>Ports (max 31)</span>
            <input type="number" min="0" max="31" bind:value={editing.cfg.ports} disabled={busy} />
          </label>
          <label>
            <span>Vectors</span>
            <input type="number" min="0" bind:value={editing.cfg.vectors} disabled={busy} />
          </label>
        {:else if t(editing.cfg) === "scsi"}
          <label>
            <span>Queues (virtio-scsi only)</span>
            <input type="number" min="0" bind:value={editing.cfg.queues} disabled={busy} />
          </label>
          <label>
            <span>IOThread (virtio-scsi only)</span>
            <input type="number" min="0" bind:value={editing.cfg.iothread} disabled={busy} />
          </label>
          <label class="toggle">
            <input type="checkbox"
              checked={editing.cfg.ioeventfd === true}
              onchange={(e) => editing.cfg.ioeventfd = e.currentTarget.checked ? true : null}
              disabled={busy} />
            <span>ioeventfd</span>
          </label>
          <label class="toggle">
            <input type="checkbox"
              checked={editing.cfg.event_idx === true}
              onchange={(e) => editing.cfg.event_idx = e.currentTarget.checked ? true : null}
              disabled={busy} />
            <span>event_idx</span>
          </label>
        {/if}

        {#if err}<div class="error">{err}</div>{/if}

        <div class="actions">
          <button class="btn" onclick={closeDialog} disabled={busy}>Cancel</button>
          <button class="btn btn-primary" onclick={save} disabled={busy}>
            {busy ? "Saving…" : (editing.mode === "add" ? "Add" : "Save")}
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .controllers { display: flex; flex-direction: column; gap: 16px; }
  .muted { color: var(--text-muted); font-size: 13px; }
  .small { font-size: 11px; }
  .error { padding: 8px 12px; background: rgba(239,68,68,0.1);
    border: 1px solid rgba(239,68,68,0.3); border-radius: 6px;
    color: #ef4444; font-size: 12px; }
  .notice { padding: 8px 12px; background: rgba(251,191,36,0.1);
    border: 1px solid rgba(251,191,36,0.3); border-radius: 6px;
    color: #fbbf24; font-size: 12px; }

  section { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 8px; padding: 14px; }
  .section-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
  h3 { margin: 0; font-size: 11px; font-weight: 600; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em; }

  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 6px 8px; border-bottom: 1px solid var(--border); }
  th { color: var(--text-muted); font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; font-weight: 600; }
  code { font-family: 'SF Mono', monospace; background: var(--bg-sidebar); padding: 1px 6px; border-radius: 3px; font-size: 12px; }

  .badge-restart {
    display: inline-block;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 10px;
    background: rgba(251,191,36,0.15);
    color: #fbbf24;
    border: 1px solid rgba(251,191,36,0.3);
    margin-right: 4px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .btn-tiny { padding: 2px 8px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--bg-button); color: var(--text); font-size: 11px; cursor: pointer; font-family: inherit; margin-left: 4px; }
  .btn-tiny:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-tiny:disabled { opacity: 0.35; cursor: not-allowed; }
  .btn-tiny.danger:hover { background: #7f1d1d; border-color: #7f1d1d; color: #fca5a5; }

  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; margin-bottom: 10px; }
  label > span { font-size: 11px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  input[type="number"], input:not([type]), select {
    padding: 6px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit;
    outline: none;
  }
  input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }
  .toggle { flex-direction: row; align-items: center; gap: 8px; font-size: 13px; cursor: pointer; }
  .toggle span { text-transform: none; letter-spacing: normal; color: var(--text); }

  .dialog-backdrop {
    position: fixed; inset: 0; background: rgba(0,0,0,0.6);
    display: flex; align-items: center; justify-content: center; z-index: 100;
  }
  .dialog {
    background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 10px; padding: 20px; min-width: 340px; max-width: 480px;
    display: flex; flex-direction: column; gap: 4px;
  }
  .dialog h3 {
    margin-bottom: 14px; color: var(--text); font-size: 14px;
    text-transform: none; letter-spacing: normal;
  }
  .actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 8px; }
  .btn { padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; cursor: pointer; font-family: inherit; }
  .btn:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
