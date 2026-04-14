<script>
  /*
   * Network interfaces panel (Round C).
   *
   * Lists `<interface>` devices attached to a VM, with add / edit / detach
   * and a link state quick-toggle. Running VMs get live + persistent
   * changes; shut-off VMs only persistent.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState } from "$lib/stores/app.svelte.js";

  let { vmName } = $props();
  const appState = getState();

  let nics = $state([]);
  let caps = $state(null);
  let loading = $state(true);
  let err = $state(null);
  let busy = $state(false);

  let editOpen = $state(null);   // null | "add" | {nic, idx}
  let editing = $state(null);    // working copy of NicConfig

  const DEFAULT_MODELS = ["virtio", "virtio-transitional", "e1000", "e1000e", "rtl8139", "pcnet", "ne2k_pci"];
  const DIRECT_MODES = ["bridge", "vepa", "private", "passthrough"];
  const SOURCE_KINDS = [
    { id: "network", label: "Virtual network" },
    { id: "bridge", label: "Host bridge" },
    { id: "direct", label: "Direct (macvtap)" },
    { id: "user", label: "User (SLIRP)" },
    { id: "vhostuser", label: "vhost-user" },
  ];

  async function reload() {
    loading = true; err = null;
    try {
      const [list, dc] = await Promise.all([
        invoke("list_domain_nics", { name: vmName }),
        invoke("get_domain_capabilities", {}).catch(() => null),
      ]);
      nics = list;
      caps = dc;
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { if (vmName) reload(); });

  let isRunning = $derived(appState.selectedVm?.state === "running");
  let models = $derived(caps?.devices?.nic_models?.length ? caps.devices.nic_models : DEFAULT_MODELS);

  function sourceLabel(source) {
    if (!source) return "—";
    switch (source.kind) {
      case "network": return `network: ${source.name}`;
      case "bridge":  return `bridge: ${source.name}`;
      case "direct":  return `direct: ${source.dev} (${source.mode})`;
      case "user":    return "user";
      case "vhostuser": return `vhost-user: ${source.socket_path}`;
      case "hostdev":
        if (source.addr?.addr_type === "pci") {
          const a = source.addr;
          return `PCI ${a.domain.toString(16).padStart(4,'0')}:${a.bus.toString(16).padStart(2,'0')}:${a.slot.toString(16).padStart(2,'0')}.${a.function.toString(16)}`;
        }
        if (source.addr?.addr_type === "usb") {
          return `USB bus ${source.addr.bus} dev ${source.addr.device}`;
        }
        return "hostdev";
      default: return JSON.stringify(source);
    }
  }

  function newNicTemplate() {
    return {
      source: { kind: "network", name: "default" },
      model: "virtio",
      mac: null,
      target_dev: null,
      link_state: null,
      mtu: null,
      boot_order: null,
      bandwidth_inbound: { average: null, peak: null, burst: null },
      bandwidth_outbound: { average: null, peak: null, burst: null },
      driver_queues: null,
      driver_txmode: null,
      filterref: null,
      vlan_tag: null,
      port_isolated: false,
    };
  }

  function openAdd() {
    editing = newNicTemplate();
    editOpen = "add";
  }

  function openEdit(nic, idx) {
    editing = JSON.parse(JSON.stringify(nic));
    // Normalise nulls so <input bind:value> works.
    editing.mac ??= "";
    editing.target_dev ??= "";
    editing.model ??= "";
    editing.link_state ??= "";
    editing.filterref ??= "";
    editing.driver_txmode ??= "";
    editOpen = { nic, idx };
  }

  function closeEditor() {
    editing = null;
    editOpen = null;
  }

  function sanitise(e) {
    // Convert "" back to null for optional string fields, 0/NaN to null for optional numbers.
    const out = JSON.parse(JSON.stringify(e));
    for (const k of ["mac", "target_dev", "model", "link_state", "filterref", "driver_txmode"]) {
      if (out[k] === "" || out[k] === undefined) out[k] = null;
    }
    for (const k of ["mtu", "boot_order", "driver_queues", "vlan_tag"]) {
      if (out[k] === "" || out[k] === undefined) out[k] = null;
      else if (typeof out[k] === "string") out[k] = parseInt(out[k], 10);
      if (Number.isNaN(out[k])) out[k] = null;
    }
    for (const bw of [out.bandwidth_inbound, out.bandwidth_outbound]) {
      for (const k of ["average", "peak", "burst"]) {
        if (bw[k] === "" || bw[k] === undefined) bw[k] = null;
        else if (typeof bw[k] === "string") bw[k] = parseInt(bw[k], 10);
        if (Number.isNaN(bw[k])) bw[k] = null;
      }
    }
    return out;
  }

  async function saveAdd() {
    busy = true; err = null;
    try {
      await invoke("add_domain_nic", {
        name: vmName,
        nic: sanitise(editing),
        live: isRunning,
        config: true,
      });
      closeEditor();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function saveEdit() {
    busy = true; err = null;
    try {
      await invoke("update_domain_nic", {
        name: vmName,
        nic: sanitise(editing),
        live: isRunning,
        config: true,
      });
      closeEditor();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function toggleLink(nic) {
    if (!nic.mac) {
      err = "Cannot toggle link: NIC has no stable MAC.";
      return;
    }
    const next = nic.link_state === "down" ? "up" : "down";
    busy = true; err = null;
    try {
      await invoke("update_domain_nic", {
        name: vmName,
        nic: { ...JSON.parse(JSON.stringify(nic)), link_state: next },
        live: isRunning,
        config: true,
      });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function detach(nic) {
    const key = nic.mac || nic.target_dev;
    if (!key) {
      err = "Cannot detach: no MAC or target dev.";
      return;
    }
    if (!confirm(`Detach NIC ${key}?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_domain_nic", {
        name: vmName,
        macOrTarget: key,
        live: isRunning,
        config: true,
      });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  function setSourceKind(kind) {
    if (!editing) return;
    switch (kind) {
      case "network":   editing.source = { kind, name: "default" }; break;
      case "bridge":    editing.source = { kind, name: "br0" }; break;
      case "direct":    editing.source = { kind, dev: "eth0", mode: "bridge" }; break;
      case "user":      editing.source = { kind }; break;
      case "vhostuser": editing.source = { kind, socket_path: "/var/run/vhost.sock", mode: "client" }; break;
      default: break;
    }
  }
</script>

<div class="nics-panel">
  {#if loading}
    <p class="muted">Loading...</p>
  {:else}
    {#if err}<div class="error">{err}</div>{/if}

    <section>
      <div class="section-head">
        <h3>Network Interfaces</h3>
        <div class="head-actions">
          <button class="btn" onclick={openAdd} disabled={busy}>+ Add NIC</button>
          <button class="btn" onclick={reload} disabled={busy}>Refresh</button>
        </div>
      </div>

      {#if nics.length === 0}
        <div class="empty">No network interfaces attached to this VM.</div>
      {:else}
        <table>
          <thead>
            <tr>
              <th>MAC</th>
              <th>Model</th>
              <th>Source</th>
              <th>Target</th>
              <th>Link</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each nics as nic, i (nic.mac ?? i)}
              <tr>
                <td class="mono">{nic.mac ?? "(auto)"}</td>
                <td>{nic.model ?? "—"}</td>
                <td>{sourceLabel(nic.source)}</td>
                <td class="mono">{nic.target_dev ?? "—"}</td>
                <td>
                  <span class="link {nic.link_state === 'down' ? 'down' : 'up'}">
                    {nic.link_state === "down" ? "down" : "up"}
                  </span>
                </td>
                <td class="row-actions">
                  <button class="btn-tiny" onclick={() => toggleLink(nic)} disabled={busy || !nic.mac}>
                    {nic.link_state === "down" ? "Link up" : "Link down"}
                  </button>
                  <button class="btn-tiny" onclick={() => openEdit(nic, i)} disabled={busy}>Edit</button>
                  <button class="btn-tiny danger" onclick={() => detach(nic)} disabled={busy}>Detach</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  {/if}
</div>

{#if editOpen && editing}
  <div class="modal-backdrop" onclick={closeEditor}>
    <div class="modal" onclick={(e) => e.stopPropagation()}>
      <h3>{editOpen === "add" ? "Add NIC" : "Edit NIC"}</h3>
      <div class="form-grid">
        <label>Source type</label>
        <select value={editing.source.kind} onchange={(e) => setSourceKind(e.target.value)}>
          {#each SOURCE_KINDS as k}
            <option value={k.id}>{k.label}</option>
          {/each}
        </select>

        {#if editing.source.kind === "network"}
          <label>Network name</label>
          <input bind:value={editing.source.name} placeholder="default" />
        {:else if editing.source.kind === "bridge"}
          <label>Bridge</label>
          <input bind:value={editing.source.name} placeholder="br0" />
        {:else if editing.source.kind === "direct"}
          <label>Device</label>
          <input bind:value={editing.source.dev} placeholder="eth0" />
          <label>Mode</label>
          <select bind:value={editing.source.mode}>
            {#each DIRECT_MODES as m}<option value={m}>{m}</option>{/each}
          </select>
        {:else if editing.source.kind === "vhostuser"}
          <label>Socket path</label>
          <input bind:value={editing.source.socket_path} placeholder="/var/run/vhost.sock" />
          <label>Mode</label>
          <select bind:value={editing.source.mode}>
            <option value="client">client</option>
            <option value="server">server</option>
          </select>
        {/if}

        <label>Model</label>
        <select bind:value={editing.model}>
          <option value="">(libvirt default)</option>
          {#each models as m}<option value={m}>{m}</option>{/each}
        </select>

        <label>MAC (optional)</label>
        <input bind:value={editing.mac} placeholder="auto-generated if blank" />

        <label>Target dev (optional)</label>
        <input bind:value={editing.target_dev} placeholder="vnetN (libvirt assigns)" />

        <label>Link state</label>
        <select bind:value={editing.link_state}>
          <option value="">default (up)</option>
          <option value="up">up</option>
          <option value="down">down</option>
        </select>

        <label>MTU</label>
        <input type="number" bind:value={editing.mtu} placeholder="1500" />

        <label>Boot order</label>
        <input type="number" bind:value={editing.boot_order} />

        <label>VLAN tag</label>
        <input type="number" bind:value={editing.vlan_tag} />

        <label>Driver queues</label>
        <input type="number" bind:value={editing.driver_queues} placeholder="virtio multi-queue" />

        <label>Driver txmode</label>
        <select bind:value={editing.driver_txmode}>
          <option value="">(default)</option>
          <option value="iothread">iothread</option>
          <option value="timer">timer</option>
          <option value="tap">tap</option>
        </select>

        <label>Filter ref</label>
        <input bind:value={editing.filterref} placeholder="e.g. clean-traffic" />

        <label>Port isolated</label>
        <input type="checkbox" bind:checked={editing.port_isolated} />

        <label>Bandwidth inbound (KiB/s avg / peak / burst)</label>
        <div class="bw-row">
          <input type="number" bind:value={editing.bandwidth_inbound.average} placeholder="avg" />
          <input type="number" bind:value={editing.bandwidth_inbound.peak} placeholder="peak" />
          <input type="number" bind:value={editing.bandwidth_inbound.burst} placeholder="burst" />
        </div>

        <label>Bandwidth outbound</label>
        <div class="bw-row">
          <input type="number" bind:value={editing.bandwidth_outbound.average} placeholder="avg" />
          <input type="number" bind:value={editing.bandwidth_outbound.peak} placeholder="peak" />
          <input type="number" bind:value={editing.bandwidth_outbound.burst} placeholder="burst" />
        </div>
      </div>
      <div class="modal-actions">
        <button class="btn" onclick={closeEditor} disabled={busy}>Cancel</button>
        {#if editOpen === "add"}
          <button class="btn primary" onclick={saveAdd} disabled={busy}>Add</button>
        {:else}
          <button class="btn primary" onclick={saveEdit} disabled={busy}>Save</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .nics-panel {
    padding: 16px;
    color: #e5e7eb;
  }
  section + section { margin-top: 18px; }
  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }
  .section-head h3 {
    font-size: 14px;
    margin: 0;
    color: #9ca3af;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .head-actions { display: flex; gap: 6px; }
  .empty { color: #6b7280; padding: 24px; text-align: center; }
  .error {
    background: #7f1d1d;
    color: #fecaca;
    padding: 8px 10px;
    border-radius: 4px;
    margin-bottom: 10px;
    font-family: ui-monospace, monospace;
    font-size: 12px;
  }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 6px 8px; border-bottom: 1px solid #1f2937; }
  th { color: #9ca3af; font-weight: 500; }
  .mono { font-family: ui-monospace, monospace; }
  .row-actions { display: flex; gap: 6px; justify-content: flex-end; }
  .link { padding: 1px 6px; border-radius: 3px; font-size: 11px; }
  .link.up { background: #064e3b; color: #6ee7b7; }
  .link.down { background: #7f1d1d; color: #fecaca; }
  .btn, .btn-tiny {
    background: #1f2937; color: #e5e7eb;
    border: 1px solid #374151;
    border-radius: 4px;
    padding: 4px 10px;
    cursor: pointer;
    font-size: 12px;
  }
  .btn-tiny { padding: 2px 8px; font-size: 11px; }
  .btn.primary { background: #1d4ed8; border-color: #1d4ed8; }
  .btn-tiny.danger { background: #7f1d1d; border-color: #991b1b; }
  .btn:disabled, .btn-tiny:disabled { opacity: 0.5; cursor: not-allowed; }
  .muted { color: #6b7280; }
  .modal-backdrop {
    position: fixed; inset: 0; background: rgba(0,0,0,0.6);
    display: flex; align-items: center; justify-content: center; z-index: 100;
  }
  .modal {
    background: #111827;
    border: 1px solid #374151;
    border-radius: 6px;
    padding: 18px;
    max-width: 560px;
    width: 100%;
    max-height: 90vh;
    overflow-y: auto;
  }
  .modal h3 { margin: 0 0 12px 0; }
  .form-grid {
    display: grid;
    grid-template-columns: 160px 1fr;
    gap: 6px 10px;
    align-items: center;
  }
  .form-grid input, .form-grid select {
    background: #0f172a;
    color: #e5e7eb;
    border: 1px solid #374151;
    border-radius: 3px;
    padding: 4px 6px;
    font-size: 12px;
    width: 100%;
  }
  .bw-row { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 4px; }
  .modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
</style>
