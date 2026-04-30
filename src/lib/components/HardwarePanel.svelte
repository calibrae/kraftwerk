<script>
  /*
   * Hardware passthrough panel: PCI + USB hostdevs attached to a VM,
   * with add (picker from host devices) and remove.
   *
   * Live changes use attach/detach with live+config flags so they apply
   * immediately to a running VM when QEMU supports it; falling back to
   * persistent-only when the user explicitly picks that in the picker.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState } from "$lib/stores/app.svelte.js";

  let { vmName } = $props();
  const appState = getState();

  let hostPci = $state([]);
  let hostUsb = $state([]);
  let hostMdevs = $state([]);
  let mdevTypes = $state([]);
  let attached = $state([]);
  let loading = $state(true);
  let err = $state(null);
  let busy = $state(false);

  // Picker modal state
  let pickerOpen = $state(null); // "pci" | "usb" | null
  let pickerUsbMode = $state("vendor"); // "vendor" | "address"
  let pickerApplyLive = $state(true);

  async function reload() {
    loading = true; err = null;
    try {
      const [p, u, a, m, mt] = await Promise.all([
        invoke("list_host_pci_devices"),
        invoke("list_host_usb_devices"),
        invoke("list_domain_hostdevs", { name: vmName }),
        invoke("list_host_mdevs").catch(() => []),
        invoke("list_host_mdev_types").catch(() => []),
      ]);
      hostPci = p; hostUsb = u; attached = a;
      hostMdevs = m; mdevTypes = mt;
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (vmName) reload();
  });

  let isRunning = $derived(appState.selectedVm?.state === "running");

  function bdf(d) {
    const hex = (n, w) => n.toString(16).padStart(w, "0");
    return `${hex(d.domain, 4)}:${hex(d.bus, 2)}:${hex(d.slot, 2)}.${hex(d.function, 1)}`;
  }
  function vid(d) {
    return `${d.vendor_id.toString(16).padStart(4, "0")}:${d.product_id.toString(16).padStart(4, "0")}`;
  }

  async function detach(dev) {
    if (!confirm("Detach this device? It will return control to the host.")) return;
    busy = true; err = null;
    try {
      await invoke("detach_hostdev", {
        name: vmName, dev,
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

  async function attachPci(device) {
    busy = true; err = null;
    try {
      await invoke("attach_hostdev", {
        name: vmName,
        dev: {
          kind: "pci",
          domain: device.domain, bus: device.bus,
          slot: device.slot, function: device.function,
          managed: true,
        },
        live: isRunning && pickerApplyLive,
        config: true,
      });
      pickerOpen = null;
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function attachUsb(device) {
    busy = true; err = null;
    const dev = pickerUsbMode === "vendor"
      ? { kind: "usb_vendor", vendor_id: device.vendor_id, product_id: device.product_id, managed: true }
      : { kind: "usb_address", bus: device.bus, device: device.device, managed: true };
    try {
      await invoke("attach_hostdev", {
        name: vmName, dev,
        live: isRunning && pickerApplyLive,
        config: true,
      });
      pickerOpen = null;
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  // Classify a PCI device by class code for a badge label.
  function pciClassLabel(cc) {
    if (cc == null) return null;
    const cls = (cc >> 16) & 0xff;
    switch (cls) {
      case 0x01: return "Storage";
      case 0x02: return "Network";
      case 0x03: return "GPU";
      case 0x04: return "Multimedia";
      case 0x06: return "Bridge";
      case 0x07: return "Serial";
      case 0x0C: return "USB";
      default: return null;
    }
  }

  // Is this host device already attached to the VM? Dims it in the picker.
  function isAttachedPci(device) {
    return attached.some(a =>
      a.kind === "pci" &&
      a.domain === device.domain && a.bus === device.bus &&
      a.slot === device.slot && a.function === device.function
    );
  }
  function isAttachedUsb(device) {
    return attached.some(a =>
      (a.kind === "usb_vendor" && a.vendor_id === device.vendor_id && a.product_id === device.product_id) ||
      (a.kind === "usb_address" && a.bus === device.bus && a.device === device.device)
    );
  }

  let sriovPfs = $derived(hostPci.filter((d) => d.sriov?.max_vfs != null));

  function describeAttached(d) {
    if (d.kind === "pci") return `PCI ${bdf(d)}`;
    if (d.kind === "usb_vendor") {
      return `USB ${d.vendor_id.toString(16).padStart(4, "0")}:${d.product_id.toString(16).padStart(4, "0")}`;
    }
    if (d.kind === "mdev") {
      return `mdev ${d.uuid}${d.model ? ` (${d.model})` : ""}${d.display ? " · display" : ""}`;
    }
    return `USB bus ${d.bus} device ${d.device}`;
  }

  function isAttachedMdev(m) {
    return attached.some(a => a.kind === "mdev" && a.uuid === m.uuid);
  }

  async function attachMdev(mdev) {
    busy = true; err = null;
    try {
      await invoke("attach_hostdev", {
        name: vmName,
        dev: {
          kind: "mdev",
          uuid: mdev.uuid,
          model: "vfio-pci",
          display: false,
        },
        live: isRunning && pickerApplyLive,
        config: true,
      });
      pickerOpen = null;
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="hardware">
  {#if loading}
    <p class="muted">Loading...</p>
  {:else}
    {#if err}<div class="error">{err}</div>{/if}

    <!-- Attached hostdevs -->
    <section>
      <div class="section-head">
        <h3>Attached Passthrough Devices</h3>
        <div class="head-actions">
          <button class="btn" onclick={() => pickerOpen = "pci"} disabled={busy}>+ PCI</button>
          <button class="btn" onclick={() => pickerOpen = "usb"} disabled={busy}>+ USB</button>
          <button class="btn" onclick={() => pickerOpen = "mdev"} disabled={busy || (hostMdevs.length === 0 && mdevTypes.length === 0)}>+ mdev</button>
          <button class="btn" onclick={reload} disabled={busy}>Refresh</button>
        </div>
      </div>

      {#if attached.length === 0}
        <div class="empty">No host devices attached to this VM.</div>
      {:else}
        <table>
          <thead>
            <tr><th>Type</th><th>Address</th><th>Managed</th><th></th></tr>
          </thead>
          <tbody>
            {#each attached as d, i (i)}
              <tr>
                <td><span class="kind-badge {d.kind}">{d.kind === "pci" ? "PCI" : d.kind === "mdev" ? "MDEV" : "USB"}</span></td>
                <td class="mono">{describeAttached(d)}</td>
                <td>{d.managed ? "yes" : "no"}</td>
                <td class="row-actions">
                  <button class="btn-tiny danger" onclick={() => detach(d)} disabled={busy}>Detach</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  {/if}
</div>

<!-- ── PCI picker ──────────────────────────────────────────────────── -->
{#if pickerOpen === "pci"}
  <div class="backdrop" onclick={() => pickerOpen = null} role="presentation">
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
      <h3>Select a PCI device</h3>
      <div class="warn-banner">
        Passing a PCI device through takes it away from the host. Devices
        bound to an active host driver (not <code>vfio-pci</code>) will
        need that driver unbound first — libvirt tries with
        <code>managed='yes'</code>.
      </div>

      {#if sriovPfs.length > 0}
        <details class="sriov-block">
          <summary>SR-IOV virtual functions ({sriovPfs.length} PF{sriovPfs.length === 1 ? "" : "s"})</summary>
          <p class="muted small">
            VFs are spawned by the host kernel on demand. Set
            <code>sriov_numvfs</code> in sysfs (root required); kraftwerk
            picks them up on Refresh and they pass through like any other
            PCI device.
          </p>
          <ul class="pf-list">
            {#each sriovPfs as pf}
              <li>
                <code class="mono">{bdf(pf)}</code>
                — {pf.product_name ?? pf.vendor_name}
                <span class="muted">({pf.sriov.virt_functions.length}/{pf.sriov.max_vfs} VFs)</span>
                <pre class="snippet"><code>ssh &lt;hypervisor&gt; "echo {pf.sriov.max_vfs} | sudo tee /sys/bus/pci/devices/0000:{pf.bus.toString(16).padStart(2,"0")}:{pf.slot.toString(16).padStart(2,"0")}.{pf.function.toString(16)}/sriov_numvfs"</code></pre>
              </li>
            {/each}
          </ul>
        </details>
      {/if}
      <div class="picker-list">
        <table>
          <thead>
            <tr><th>Address</th><th>Device</th><th>Vendor</th><th>Class</th><th>SR-IOV</th><th>Driver</th><th>IOMMU</th><th></th></tr>
          </thead>
          <tbody>
            {#each hostPci as d}
              {@const already = isAttachedPci(d)}
              <tr class:dim={already}>
                <td class="mono">{bdf(d)}</td>
                <td>{d.product_name || "—"} <span class="id">{d.vendor_id.toString(16).padStart(4, "0")}:{d.product_id.toString(16).padStart(4, "0")}</span></td>
                <td>{d.vendor_name}</td>
                <td>
                  {#if pciClassLabel(d.class_code)}
                    <span class="class-badge">{pciClassLabel(d.class_code)}</span>
                  {/if}
                </td>
                <td>
                  {#if d.sriov?.max_vfs != null}
                    <span class="sriov-badge pf">PF · {d.sriov.virt_functions.length}/{d.sriov.max_vfs}</span>
                  {:else if d.sriov?.phys_function}
                    <span class="sriov-badge vf">VF</span>
                  {/if}
                </td>
                <td class="mono" title={d.driver ?? ""}>{d.driver ?? "—"}</td>
                <td>{d.iommu_group ?? "—"}</td>
                <td class="row-actions">
                  {#if already}
                    <span class="muted">attached</span>
                  {:else}
                    <button class="btn-tiny" onclick={() => attachPci(d)} disabled={busy}>Attach</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="picker-foot">
        {#if isRunning}
          <label class="toggle">
            <input type="checkbox" bind:checked={pickerApplyLive} />
            <span>Apply to running VM now (live hot-plug)</span>
          </label>
        {/if}
        <button class="btn" onclick={() => pickerOpen = null}>Close</button>
      </div>
    </div>
  </div>
{/if}

<!-- ── USB picker ──────────────────────────────────────────────────── -->
{#if pickerOpen === "usb"}
  <div class="backdrop" onclick={() => pickerOpen = null} role="presentation">
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
      <h3>Select a USB device</h3>
      <div class="usb-mode">
        <label class="toggle"><input type="radio" value="vendor" bind:group={pickerUsbMode}/><span>By vendor:product (survives replug)</span></label>
        <label class="toggle"><input type="radio" value="address" bind:group={pickerUsbMode}/><span>By bus:device (fixed address)</span></label>
      </div>
      <div class="picker-list">
        <table>
          <thead>
            <tr><th>Address</th><th>Device</th><th>Vendor</th><th>Driver</th><th></th></tr>
          </thead>
          <tbody>
            {#each hostUsb as d}
              {@const already = isAttachedUsb(d)}
              <tr class:dim={already}>
                <td class="mono">{d.bus}:{d.device}</td>
                <td>{d.product_name || "—"} <span class="id">{vid(d)}</span></td>
                <td>{d.vendor_name}</td>
                <td class="mono">{d.driver ?? "—"}</td>
                <td class="row-actions">
                  {#if already}
                    <span class="muted">attached</span>
                  {:else}
                    <button class="btn-tiny" onclick={() => attachUsb(d)} disabled={busy}>Attach</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="picker-foot">
        {#if isRunning}
          <label class="toggle">
            <input type="checkbox" bind:checked={pickerApplyLive} />
            <span>Apply to running VM now (live hot-plug)</span>
          </label>
        {/if}
        <button class="btn" onclick={() => pickerOpen = null}>Close</button>
      </div>
    </div>
  </div>
{/if}

<!-- ── mdev picker ─────────────────────────────────────────────────── -->
{#if pickerOpen === "mdev"}
  <div class="backdrop" onclick={() => pickerOpen = null} role="presentation">
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
      <h3>Select a mediated device</h3>
      <div class="warn-banner">
        Mediated devices (vGPU, vfio-mdev) are slices of a parent PCI device
        that the host operator has pre-allocated. kraftwerk lists existing
        instances and the available types per parent — creating new
        instances is a host-side step (sysfs / <code>mdevctl</code>).
      </div>

      <div class="picker-list">
        <h4 class="sub-h">Active mdev instances</h4>
        {#if hostMdevs.length === 0}
          <p class="muted">No active mdevs on the host.</p>
        {:else}
          <table>
            <thead>
              <tr><th>UUID</th><th>Type</th><th>Parent</th><th>IOMMU</th><th></th></tr>
            </thead>
            <tbody>
              {#each hostMdevs as m}
                {@const already = isAttachedMdev(m)}
                <tr class:dim={already}>
                  <td class="mono">{m.uuid}</td>
                  <td class="mono">{m.type_id ?? "—"}</td>
                  <td class="mono">{m.parent ?? "—"}</td>
                  <td>{m.iommu_group ?? "—"}</td>
                  <td class="row-actions">
                    {#if already}
                      <span class="muted">attached</span>
                    {:else}
                      <button class="btn-tiny" onclick={() => attachMdev(m)} disabled={busy}>Attach</button>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}

        <h4 class="sub-h">Types advertised by host parents</h4>
        {#if mdevTypes.length === 0}
          <p class="muted">No mdev-capable parent devices on the host.</p>
        {:else}
          <table>
            <thead>
              <tr><th>Parent</th><th>Type</th><th>Name</th><th>API</th><th>Slots free</th></tr>
            </thead>
            <tbody>
              {#each mdevTypes as t}
                <tr>
                  <td class="mono">{t.parent}</td>
                  <td class="mono">{t.type_id}</td>
                  <td>{t.name ?? "—"}</td>
                  <td class="mono">{t.device_api ?? "—"}</td>
                  <td>{t.available_instances ?? "?"}</td>
                </tr>
              {/each}
            </tbody>
          </table>
          <p class="muted small mdev-hint">
            To create a new instance, on the hypervisor: <code>mdevctl start
            -u $(uuidgen) -p &lt;parent&gt; -t &lt;type&gt;</code> (or echo a
            UUID into <code>/sys/class/mdev_bus/&lt;parent&gt;/mdev_supported_types/&lt;type&gt;/create</code>).
            Then click Refresh here.
          </p>
        {/if}
      </div>

      <div class="picker-foot">
        {#if isRunning}
          <label class="toggle">
            <input type="checkbox" bind:checked={pickerApplyLive} />
            <span>Apply to running VM now (live hot-plug)</span>
          </label>
        {/if}
        <button class="btn" onclick={() => pickerOpen = null}>Close</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .hardware { display: flex; flex-direction: column; gap: 16px; }
  .muted { color: var(--text-muted); font-size: 13px; }
  .empty { padding: 16px; color: var(--text-muted); font-size: 13px; text-align: center;
           background: var(--bg-sidebar); border-radius: 6px; }
  .error { padding: 8px 12px; background: rgba(239,68,68,0.1);
           border: 1px solid rgba(239,68,68,0.3); border-radius: 6px;
           color: #ef4444; font-size: 12px; }

  section { background: var(--bg-surface); border: 1px solid var(--border);
            border-radius: 8px; padding: 16px; }
  .section-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; }
  h3 { margin: 0; font-size: 13px; font-weight: 600; color: var(--text-muted);
       text-transform: uppercase; letter-spacing: 0.05em; }
  .head-actions { display: flex; gap: 6px; }

  .btn { padding: 6px 12px; border: 1px solid var(--border); border-radius: 6px;
         background: var(--bg-button); color: var(--text); cursor: pointer;
         font-size: 12px; font-family: inherit; }
  .btn:hover { background: var(--bg-hover); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .btn-tiny { padding: 2px 8px; border: 1px solid var(--border); border-radius: 4px;
              background: var(--bg-button); color: var(--text); font-size: 11px; cursor: pointer; }
  .btn-tiny:hover { background: var(--bg-hover); }
  .btn-tiny.danger { color: #fca5a5; }
  .btn-tiny.danger:hover { background: #7f1d1d; border-color: #7f1d1d; }

  table { width: 100%; border-collapse: collapse; font-size: 12px; }
  thead th { text-align: left; padding: 6px 10px; color: var(--text-muted); font-weight: 500;
             border-bottom: 1px solid var(--border); font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; }
  tbody td { padding: 6px 10px; border-bottom: 1px solid var(--border); vertical-align: middle; }
  tbody tr:last-child td { border-bottom: none; }
  tbody tr.dim { opacity: 0.45; }
  .row-actions { text-align: right; }

  .mono { font-family: 'SF Mono', monospace; font-size: 11px; }
  .id { color: var(--text-muted); font-family: 'SF Mono', monospace; font-size: 10px; margin-left: 4px; }

  .kind-badge, .class-badge {
    display: inline-block; padding: 2px 8px; border-radius: 10px;
    font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em;
  }
  .kind-badge.pci { background: rgba(99,102,241,0.2); color: #a5b4fc; }
  .kind-badge.usb_vendor, .kind-badge.usb_address { background: rgba(16,185,129,0.2); color: #6ee7b7; }
  .kind-badge.mdev { background: rgba(217,70,239,0.2); color: #f0abfc; }
  .sriov-badge { display: inline-block; padding: 1px 6px; border-radius: 4px;
    font-size: 10px; font-weight: 600; }
  .sriov-badge.pf { background: rgba(59,130,246,0.2); color: #93c5fd; }
  .sriov-badge.vf { background: rgba(16,185,129,0.15); color: #6ee7b7; }
  .sriov-block { margin: 10px 20px 0; padding: 10px 12px; border: 1px solid var(--border);
    border-radius: 6px; background: rgba(0,0,0,0.15); }
  .sriov-block summary { cursor: pointer; font-size: 12px; color: var(--text-muted); }
  .pf-list { list-style: none; padding: 0; margin: 8px 0 0; display: flex;
    flex-direction: column; gap: 8px; }
  .snippet { margin: 4px 0 0; padding: 6px 8px; background: rgba(0,0,0,0.3);
    border-radius: 4px; overflow-x: auto; font-size: 11px; }
  .snippet code { font-family: 'SF Mono', monospace; }
  .sub-h { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
           color: var(--text-muted); margin: 12px 0 6px; font-weight: 600; }
  .sub-h:first-child { margin-top: 0; }
  .small { font-size: 11px; }
  .mdev-hint { margin-top: 8px; }
  .mdev-hint code { background: rgba(0,0,0,0.3); padding: 1px 4px; border-radius: 3px; font-size: 10px; }
  .class-badge { background: var(--bg-button); color: var(--text-muted); }

  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.6);
              display: flex; align-items: center; justify-content: center; z-index: 100; padding: 20px; }
  .dialog { background: var(--bg-surface); border: 1px solid var(--border);
            border-radius: 12px; width: 820px; max-width: 100%; max-height: 85vh;
            display: flex; flex-direction: column; box-shadow: 0 12px 40px rgba(0,0,0,0.4); }
  .dialog h3 { padding: 18px 20px 0; margin: 0; font-size: 15px; font-weight: 600; color: var(--text); text-transform: none; letter-spacing: 0; }
  .warn-banner { margin: 14px 20px 0; padding: 10px 12px;
                 background: rgba(251,191,36,0.1); border: 1px solid rgba(251,191,36,0.3);
                 border-radius: 6px; color: #fbbf24; font-size: 12px; }
  .warn-banner code { background: rgba(0,0,0,0.3); padding: 1px 4px; border-radius: 3px; font-size: 11px; }
  .picker-list { flex: 1; overflow-y: auto; padding: 14px 20px; }
  .picker-foot { padding: 12px 20px; border-top: 1px solid var(--border);
                 display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .usb-mode { padding: 14px 20px 0; display: flex; gap: 16px; }
  .toggle { display: flex; align-items: center; gap: 8px; cursor: pointer; font-size: 13px; }
  .toggle input { margin: 0; }
  .toggle span { color: var(--text); }
</style>
