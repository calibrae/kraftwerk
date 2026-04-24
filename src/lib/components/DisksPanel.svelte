<script>
  /*
   * Disk/CD-ROM editor.
   *
   * Shows all disks on the VM. You can:
   *   - Add a new disk: pick a volume in a pool, or provide a raw path.
   *   - Edit per-row: bus / cache / io / discard / readonly / ...
   *   - Detach per-row.
   *   - For CD-ROMs: eject (clear media) or insert (pick ISO path).
   *
   * All actions go live+config by default. Some changes (CD-ROM on SATA,
   * for instance) only apply to config — the backend silently falls back
   * when live hotplug is refused. We display the error if any.
   */
  import { invoke } from "@tauri-apps/api/core";

  let { vmName } = $props();

  let disks = $state([]);
  let pools = $state([]);
  let volumesByPool = $state({});
  let loading = $state(true);
  let err = $state(null);
  let busy = $state(false);

  // "Add Disk" dialog state
  let showAdd = $state(false);
  let addMode = $state("existing"); // "existing" | "new"
  let addPool = $state("default");
  let addVolume = $state("");
  let addNewName = $state("");
  let addNewSizeGB = $state(10);
  let addNewFormat = $state("qcow2");
  let addPath = $state("");
  let addDevice = $state("disk"); // disk | cdrom
  let addBus = $state("virtio");
  let addTarget = $state("");
  let addCache = $state("none");

  // Edit dialog state
  let editing = $state(null); // a full DiskConfig copy, or null.

  async function reload() {
    loading = true; err = null;
    try {
      disks = await invoke("list_domain_disks", { name: vmName });
      pools = await invoke("list_storage_pools");
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  async function loadVolumes(poolName) {
    if (!poolName || volumesByPool[poolName]) return;
    try {
      const vols = await invoke("list_volumes", { poolName });
      volumesByPool = { ...volumesByPool, [poolName]: vols };
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    }
  }

  $effect(() => { if (vmName) reload(); });
  $effect(() => { if (addPool) loadVolumes(addPool); });

  const BUS_CHOICES = ["virtio", "sata", "scsi", "ide", "usb"];
  const BUS_TARGET_PREFIX = {
    virtio: "vd", sata: "sd", scsi: "sd", ide: "hd", usb: "sd", fdc: "fd",
  };
  const CACHE_CHOICES = ["default", "none", "writethrough", "writeback", "directsync", "unsafe"];
  const IO_CHOICES = ["default", "native", "threads", "io_uring"];
  const DISCARD_CHOICES = ["default", "unmap", "ignore"];
  const DETECT_ZEROES_CHOICES = ["default", "off", "on", "unmap"];

  function nextTargetFor(bus) {
    const prefix = BUS_TARGET_PREFIX[bus] || "vd";
    const taken = new Set(disks.map(d => d.target));
    for (let i = 0; i < 26; i++) {
      const name = prefix + String.fromCharCode("a".charCodeAt(0) + i);
      if (!taken.has(name)) return name;
    }
    return prefix + "zz";
  }

  function sizeOf(disk) {
    // If the source is a volume, we can try looking it up in volumesByPool.
    if (disk.source?.kind === "volume") {
      const vols = volumesByPool[disk.source.pool] || [];
      const v = vols.find(x => x.name === disk.source.volume);
      if (v?.capacity_bytes) return humanSize(v.capacity_bytes);
    }
    return "—";
  }

  function sourceLabel(s) {
    if (!s || s.kind === "none") return "(empty)";
    if (s.kind === "file") return s.path;
    if (s.kind === "block") return s.dev;
    if (s.kind === "volume") return `${s.pool}/${s.volume}`;
    if (s.kind === "network") return `${s.protocol}:${s.name}`;
    return JSON.stringify(s);
  }

  function humanSize(b) {
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let i = 0;
    while (b >= 1024 && i < units.length - 1) { b /= 1024; i++; }
    return `${b.toFixed(1)} ${units[i]}`;
  }

  function openAdd() {
    showAdd = true;
    addMode = "existing";
    addPool = pools[0]?.name || "default";
    addVolume = "";
    addNewName = "disk-" + Date.now().toString(36) + ".qcow2";
    addNewSizeGB = 10;
    addNewFormat = "qcow2";
    addPath = "";
    addDevice = "disk";
    addBus = "virtio";
    addTarget = nextTargetFor(addBus);
    addCache = "none";
  }

  async function submitAdd() {
    if (!addTarget) addTarget = nextTargetFor(addBus);
    busy = true; err = null;
    try {
      let source;
      if (addMode === "existing") {
        if (addPath.trim()) {
          source = { kind: "file", path: addPath.trim() };
        } else {
          source = { kind: "volume", pool: addPool, volume: addVolume };
        }
      } else {
        // Create the volume first.
        const volXml =
          `<volume>\n` +
          `  <name>${escapeXml(addNewName)}</name>\n` +
          `  <capacity unit='bytes'>${Math.round(addNewSizeGB * 1024 * 1024 * 1024)}</capacity>\n` +
          `  <target><format type='${escapeXml(addNewFormat)}'/></target>\n` +
          `</volume>`;
        await invoke("create_volume", { poolName: addPool, xml: volXml });
        source = { kind: "volume", pool: addPool, volume: addNewName };
      }
      const disk = {
        device: addDevice,
        bus: addBus,
        target: addTarget,
        source,
        driver_name: "qemu",
        driver_type: addDevice === "cdrom" ? "raw" : (addNewFormat || "qcow2"),
        cache: addCache === "default" ? null : addCache,
        io: null, discard: null, detect_zeroes: null,
        serial: null, readonly: addDevice === "cdrom",
        shareable: false, removable: false,
        rotation_rate: null, iothread: null, boot_order: null,
      };
      await invoke("add_domain_disk", { name: vmName, disk, live: true, config: true });
      showAdd = false;
      await reload();
    } catch (e) {
      // Fallback: try config-only (for e.g. SATA CD-ROM hotplug refusal).
      if (String(e).includes("cannot be hotplugged")) {
        try {
          // re-try without live
          const disk2 = buildDiskFromForm();
          await invoke("add_domain_disk", { name: vmName, disk: disk2, live: false, config: true });
          showAdd = false;
          await reload();
          return;
        } catch (e2) {
          err = e2?.message || JSON.stringify(e2);
        }
      }
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  function buildDiskFromForm() {
    let source;
    if (addMode === "existing") {
      source = addPath.trim()
        ? { kind: "file", path: addPath.trim() }
        : { kind: "volume", pool: addPool, volume: addVolume };
    } else {
      source = { kind: "volume", pool: addPool, volume: addNewName };
    }
    return {
      device: addDevice, bus: addBus, target: addTarget, source,
      driver_name: "qemu",
      driver_type: addDevice === "cdrom" ? "raw" : "qcow2",
      cache: addCache === "default" ? null : addCache,
      io: null, discard: null, detect_zeroes: null,
      serial: null, readonly: addDevice === "cdrom",
      shareable: false, removable: false,
      rotation_rate: null, iothread: null, boot_order: null,
    };
  }

  function escapeXml(s) {
    return String(s).replace(/[&<>"']/g, c =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&apos;" }[c]));
  }

  function openEdit(d) {
    editing = JSON.parse(JSON.stringify(d));
    editing._origBus = d.bus;
    if (editing.cache == null) editing.cache = "default";
    if (editing.io == null) editing.io = "default";
    if (editing.discard == null) editing.discard = "default";
    if (editing.detect_zeroes == null) editing.detect_zeroes = "default";
  }

  async function submitEdit() {
    if (!editing) return;
    if (editing._origBus && editing._origBus !== editing.bus) {
      if (!confirm(`Changing disk bus from ${editing._origBus} to ${editing.bus} almost always prevents the guest from finding its root device on next boot. Only do this for uninstalled/wiped disks. Continue?`)) {
        return;
      }
    }
    busy = true; err = null;
    const patched = { ...editing };
    delete patched._origBus;
    if (patched.cache === "default") patched.cache = null;
    if (patched.io === "default") patched.io = null;
    if (patched.discard === "default") patched.discard = null;
    if (patched.detect_zeroes === "default") patched.detect_zeroes = null;
    try {
      await invoke("update_domain_disk", { name: vmName, disk: patched, live: true, config: true });
      editing = null;
      await reload();
    } catch (e) {
      // CD-ROM media on some buses only works with config-only.
      try {
        await invoke("update_domain_disk", { name: vmName, disk: patched, live: false, config: true });
        editing = null;
        await reload();
      } catch (e2) {
        err = e2?.message || JSON.stringify(e2);
      }
    } finally {
      busy = false;
    }
  }

  async function detach(d) {
    if (isBootDisk(d)) {
      if (!confirm("Removing the boot disk will crash the VM immediately. Continue?")) return;
    }
    if (!confirm(`Detach disk ${d.target}?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_domain_disk", {
        name: vmName, targetDev: d.target, live: true, config: true,
      });
      await reload();
    } catch (e) {
      try {
        await invoke("remove_domain_disk", {
          name: vmName, targetDev: d.target, live: false, config: true,
        });
        await reload();
      } catch (e2) {
        err = e2?.message || JSON.stringify(e2);
      }
    } finally {
      busy = false;
    }
  }

  async function ejectMedia(d) {
    const replaced = { ...d, source: { kind: "none" } };
    busy = true; err = null;
    try {
      await invoke("update_domain_disk", { name: vmName, disk: replaced, live: true, config: true });
      await reload();
    } catch (e) {
      try {
        await invoke("update_domain_disk", { name: vmName, disk: replaced, live: false, config: true });
        await reload();
      } catch (e2) {
        err = e2?.message || JSON.stringify(e2);
      }
    } finally {
      busy = false;
    }
  }

  async function insertMedia(d) {
    const path = prompt("Enter ISO path on the hypervisor host:");
    if (!path) return;
    const replaced = { ...d, source: { kind: "file", path } };
    busy = true; err = null;
    try {
      await invoke("update_domain_disk", { name: vmName, disk: replaced, live: true, config: true });
      await reload();
    } catch (e) {
      try {
        await invoke("update_domain_disk", { name: vmName, disk: replaced, live: false, config: true });
        await reload();
      } catch (e2) {
        err = e2?.message || JSON.stringify(e2);
      }
    } finally {
      busy = false;
    }
  }

  function busMismatch(bus, target) {
    const p = BUS_TARGET_PREFIX[bus];
    if (!p || !target) return false;
    return !target.startsWith(p);
  }

  // Heuristic: a disk is the boot disk if it has boot_order===1, OR if
  // it's on a conventional boot target (vda/sda/hda/xvda) and no other
  // disk has an explicit boot_order.
  const BOOT_TARGETS = new Set(["vda", "sda", "hda", "xvda"]);
  function isBootDisk(disk) {
    if (disk.boot_order === 1) return true;
    const anyExplicit = disks.some((d) => typeof d.boot_order === "number");
    if (anyExplicit) return false;
    return BOOT_TARGETS.has(disk.target);
  }
</script>

<div class="panel">
  <div class="panel-header">
    <h3>Disks</h3>
    <button class="btn-primary" onclick={openAdd} disabled={busy}>Add Disk</button>
  </div>

  {#if err}
    <div class="err">{err}</div>
  {/if}

  {#if loading}
    <div class="muted">Loading…</div>
  {:else if disks.length === 0}
    <div class="muted">No disks attached.</div>
  {:else}
    <table class="disks">
      <thead>
        <tr>
          <th>Target</th>
          <th>Bus</th>
          <th>Device</th>
          <th>Format</th>
          <th>Size</th>
          <th>Source</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each disks as d (d.target)}
          <tr>
            <td><code>{d.target}</code>{#if isBootDisk(d)}<span class="boot-badge">BOOT</span>{/if}</td>
            <td>{d.bus}</td>
            <td>{d.device}</td>
            <td>{d.driver_type ?? "—"}</td>
            <td>{sizeOf(d)}</td>
            <td class="src" title={sourceLabel(d.source)}>{sourceLabel(d.source)}</td>
            <td class="actions">
              <button class="btn-small" onclick={() => openEdit(d)} disabled={busy}>Edit</button>
              {#if d.device === "cdrom"}
                {#if d.source?.kind === "none"}
                  <button class="btn-small" onclick={() => insertMedia(d)} disabled={busy}>Insert</button>
                {:else}
                  <button class="btn-small" onclick={() => ejectMedia(d)} disabled={busy}>Eject</button>
                {/if}
              {/if}
              <button class="btn-small danger" onclick={() => detach(d)} disabled={busy}>Detach</button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

{#if showAdd}
  <div class="modal-backdrop" onclick={() => showAdd = false}>
    <div class="modal" onclick={(e) => e.stopPropagation()}>
      <h3>Add Disk</h3>
      <div class="form">
        <label>Device
          <select bind:value={addDevice}>
            <option value="disk">Disk</option>
            <option value="cdrom">CD-ROM</option>
          </select>
        </label>
        <label>Bus
          <select bind:value={addBus} onchange={() => addTarget = nextTargetFor(addBus)}>
            {#each BUS_CHOICES as b}<option value={b}>{b}</option>{/each}
          </select>
        </label>
        <label>Target
          <input bind:value={addTarget} class:warn={busMismatch(addBus, addTarget)} />
        </label>
        <label>Cache
          <select bind:value={addCache}>
            {#each CACHE_CHOICES as c}<option value={c}>{c}</option>{/each}
          </select>
        </label>

        <div class="mode-switch">
          <label><input type="radio" bind:group={addMode} value="existing" /> Existing volume / path</label>
          <label><input type="radio" bind:group={addMode} value="new" /> Create new volume</label>
        </div>

        {#if addMode === "existing"}
          <label>Pool
            <select bind:value={addPool}>
              {#each pools as p}<option value={p.name}>{p.name}</option>{/each}
            </select>
          </label>
          <label>Volume
            <select bind:value={addVolume} disabled={!volumesByPool[addPool]}>
              <option value="">(none)</option>
              {#each (volumesByPool[addPool] || []) as v}
                <option value={v.name}>{v.name}</option>
              {/each}
            </select>
          </label>
          <label>Or raw path
            <input bind:value={addPath} placeholder="/var/lib/libvirt/images/foo.qcow2" />
          </label>
        {:else}
          <label>Pool
            <select bind:value={addPool}>
              {#each pools as p}<option value={p.name}>{p.name}</option>{/each}
            </select>
          </label>
          <label>New volume name
            <input bind:value={addNewName} />
          </label>
          <label>Size (GB)
            <input type="number" bind:value={addNewSizeGB} min="1" />
          </label>
          <label>Format
            <select bind:value={addNewFormat}>
              <option value="qcow2">qcow2</option>
              <option value="raw">raw</option>
            </select>
          </label>
        {/if}
      </div>
      <div class="modal-actions">
        <button class="btn-small" onclick={() => showAdd = false} disabled={busy}>Cancel</button>
        <button class="btn-primary" onclick={submitAdd} disabled={busy}>
          {busy ? "Adding…" : "Add"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if editing}
  <div class="modal-backdrop" onclick={() => editing = null}>
    <div class="modal" onclick={(e) => e.stopPropagation()}>
      <h3>Edit Disk {editing.target}</h3>
      <div class="form">
        <label>Bus
          <select bind:value={editing.bus}>{#each BUS_CHOICES as b}<option value={b}>{b}</option>{/each}</select>
        </label>
        <label>Cache
          <select bind:value={editing.cache}>{#each CACHE_CHOICES as c}<option value={c}>{c}</option>{/each}</select>
        </label>
        <label>IO
          <select bind:value={editing.io}>{#each IO_CHOICES as c}<option value={c}>{c}</option>{/each}</select>
        </label>
        <label>Discard
          <select bind:value={editing.discard}>{#each DISCARD_CHOICES as c}<option value={c}>{c}</option>{/each}</select>
        </label>
        <label>Detect Zeroes
          <select bind:value={editing.detect_zeroes}>{#each DETECT_ZEROES_CHOICES as c}<option value={c}>{c}</option>{/each}</select>
        </label>
        <label>Serial
          <input bind:value={editing.serial} placeholder="(optional)" />
        </label>
        <label><input type="checkbox" bind:checked={editing.readonly} /> Readonly</label>
        <label><input type="checkbox" bind:checked={editing.shareable} /> Shareable</label>
        {#if editing.bus === "scsi"}
          <label>Rotation rate (1 = SSD)
            <input type="number" bind:value={editing.rotation_rate} />
          </label>
        {/if}
        <label>Boot order
          <input type="number" bind:value={editing.boot_order} min="1" />
        </label>
      </div>
      <div class="modal-actions">
        <button class="btn-small" onclick={() => editing = null} disabled={busy}>Cancel</button>
        <button class="btn-primary" onclick={submitEdit} disabled={busy}>
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .panel { display: flex; flex-direction: column; gap: 12px; }
  .panel-header { display: flex; justify-content: space-between; align-items: center; }
  h3 { margin: 0; font-size: 14px; font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  .err { padding: 10px; background: #7f1d1d; color: #fca5a5; border-radius: 6px; font-size: 13px; }
  .boot-badge { display: inline-block; margin-left: 6px; padding: 1px 5px; border-radius: 3px; font-size: 10px; background: rgba(34,197,94,0.15); color: #4ade80; border: 1px solid rgba(34,197,94,0.3); font-family: inherit; letter-spacing: 0.03em; vertical-align: middle; }
  .muted { color: var(--text-muted); font-size: 13px; }
  table.disks { width: 100%; border-collapse: collapse; font-size: 13px; }
  table.disks th, table.disks td { padding: 8px 10px; text-align: left; border-bottom: 1px solid var(--border); }
  table.disks th { color: var(--text-muted); font-weight: 500; text-transform: uppercase; font-size: 11px; letter-spacing: 0.05em; }
  td.src { max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: monospace; font-size: 12px; }
  td.actions { display: flex; gap: 4px; }
  .btn-primary, .btn-small { padding: 6px 12px; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-button); color: var(--text); font-size: 12px; cursor: pointer; font-family: inherit; }
  .btn-primary { background: #1e3a5f; color: #93c5fd; border-color: #1e3a5f; }
  .btn-primary:hover:not(:disabled) { background: #1e40af; }
  .btn-small:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-small.danger { background: #7f1d1d; color: #fca5a5; border-color: #7f1d1d; }
  .btn-small.danger:hover:not(:disabled) { background: #991b1b; }
  .btn-small:disabled, .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }

  .modal-backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.6); display: flex; align-items: center; justify-content: center; z-index: 100; }
  .modal { background: var(--bg-surface); border: 1px solid var(--border); border-radius: 10px; padding: 20px; min-width: 460px; max-width: 600px; max-height: 85vh; overflow-y: auto; }
  .form { display: grid; grid-template-columns: 1fr 1fr; gap: 12px 16px; margin: 12px 0; }
  .form label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--text-muted); }
  .form input[type="text"], .form input[type="number"], .form input:not([type]), .form select {
    padding: 6px 8px; border-radius: 4px; border: 1px solid var(--border);
    background: var(--bg-sidebar); color: var(--text); font-size: 13px; font-family: inherit;
  }
  .form input.warn { border-color: #f59e0b; }
  .mode-switch { grid-column: 1 / -1; display: flex; gap: 16px; padding: 8px; background: var(--bg-sidebar); border-radius: 6px; }
  .mode-switch label { flex-direction: row; align-items: center; gap: 6px; font-size: 13px; color: var(--text); }
  .modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
</style>
