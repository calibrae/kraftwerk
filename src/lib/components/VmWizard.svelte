<script>
  /*
   * VM creation wizard — three entry paths:
   *
   *   1. Install from ISO    — full install flow, attaches CD with ISO,
   *                            boot from CD then disk.
   *   2. Import existing disk — wraps an existing qcow2/raw as the boot
   *                            disk; no install media, boot from hd.
   *   3. Empty (manual)      — minimal VM, drops you into the config tabs
   *                            to set everything up before first boot.
   *
   * After Create, the wizard closes and the new VM appears in the sidebar.
   * No mid-wizard "customize before install" detour — edit the VM in the
   * regular config tabs once it exists.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState, refreshVms, clearError } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false) } = $props();
  const appState = getState();

  let osVariants = $state([]);
  let busy = $state(false);
  let err = $state(null);
  let step = $state(0);              // 0 = pick mode; 1+ = path-specific
  let mode = $state(null);           // "iso" | "import" | "empty"

  // Common
  let name = $state("");
  let osVariantId = $state("fedora");
  let vcpus = $state(2);
  let memoryMb = $state(2048);
  let networkMode = $state("network"); // "network" | "bridge" | "none"
  let networkName = $state("");
  let bridgeName = $state("");
  let graphics = $state("vnc");
  let startNow = $state(false);      // default OFF — user usually wants to edit first

  // ISO-install only
  let isoPath = $state("");

  // New-disk fields (ISO + Empty)
  let diskMode = $state("new");      // "new" | "existing"
  let diskPoolName = $state("");
  let diskSizeGb = $state(20);
  let diskFormat = $state("qcow2");
  let existingVolumePath = $state("");
  let volumesForPool = $state([]);

  // Import-disk only
  let importPoolName = $state("");
  let importVolumePath = $state("");
  let importVolumes = $state([]);

  async function init() {
    if (osVariants.length === 0) osVariants = await invoke("list_os_variants");
    const pools = appState.pools.filter(p => p.is_active);
    if (pools.length > 0) {
      if (!diskPoolName) diskPoolName = pools[0].name;
      if (!importPoolName) importPoolName = pools[0].name;
    }
    const nets = appState.networks.filter(n => n.is_active);
    if (nets.length > 0 && !networkName) networkName = nets[0].name;
  }
  $effect(() => { if (open) init(); });

  let selectedVariant = $derived(osVariants.find(v => v.id === osVariantId) ?? null);

  $effect(() => {
    if (diskPoolName) reloadVolumes(diskPoolName).then(v => volumesForPool = v);
  });
  $effect(() => {
    if (importPoolName) reloadVolumes(importPoolName).then(v => importVolumes = v);
  });
  async function reloadVolumes(pool) {
    try { return await invoke("list_volumes", { poolName: pool }); }
    catch { return []; }
  }

  function reset() {
    step = 0; mode = null; busy = false; err = null;
    name = ""; osVariantId = "fedora";
    vcpus = 2; memoryMb = 2048;
    networkMode = "network"; networkName = ""; bridgeName = "";
    graphics = "vnc"; startNow = false;
    isoPath = ""; diskMode = "new"; diskSizeGb = 20; diskFormat = "qcow2";
    existingVolumePath = ""; importVolumePath = "";
  }
  function close() { open = false; reset(); }

  // Step labels and counts depend on mode.
  let totalSteps = $derived(
    mode === "iso"    ? 5 :
    mode === "import" ? 4 :
    mode === "empty"  ? 3 :
    1
  );

  function canAdvance() {
    if (step === 0) return mode != null;
    if (mode === "iso") {
      switch (step) {
        case 1: return name.trim() && osVariantId;
        case 2: return vcpus >= 1 && memoryMb >= 128;
        case 3: {
          if (diskMode === "new") return diskPoolName && diskSizeGb > 0;
          return !!existingVolumePath;
        }
        case 4: return isoPath.trim().length > 0;
        default: return true;
      }
    }
    if (mode === "import") {
      switch (step) {
        case 1: return name.trim() && importVolumePath;
        case 2: return osVariantId;
        case 3: return vcpus >= 1 && memoryMb >= 128;
        default: return true;
      }
    }
    if (mode === "empty") {
      switch (step) {
        case 1: return name.trim();
        case 2: return vcpus >= 1 && memoryMb >= 128;
        default: return true;
      }
    }
    return true;
  }

  async function submit() {
    if (!selectedVariant) {
      err = "Pick an OS variant first.";
      return;
    }
    busy = true; err = null;
    const d = selectedVariant.defaults;

    let disk_source;
    if (mode === "iso") {
      disk_source = diskMode === "new"
        ? { kind: "new_volume", pool_name: diskPoolName, name: `${name.trim()}.${diskFormat}`, capacity_bytes: Math.floor(diskSizeGb * 1024 * 1024 * 1024), format: diskFormat }
        : { kind: "existing_path", path: existingVolumePath, format: existingVolumePath.endsWith(".iso") ? "raw" : "qcow2" };
    } else if (mode === "import") {
      disk_source = { kind: "existing_path", path: importVolumePath, format: importVolumePath.endsWith(".raw") || importVolumePath.endsWith(".img") ? "raw" : "qcow2" };
    } else {
      // empty
      disk_source = diskMode === "new"
        ? { kind: "new_volume", pool_name: diskPoolName, name: `${name.trim()}.${diskFormat}`, capacity_bytes: Math.floor(diskSizeGb * 1024 * 1024 * 1024), format: diskFormat }
        : { kind: "existing_path", path: existingVolumePath || "", format: "qcow2" };
    }

    let network;
    if (mode === "empty" || networkMode === "network") network = { kind: "network", name: networkName || "default" };
    else if (networkMode === "bridge") network = { kind: "bridge", name: bridgeName };
    else network = { kind: "none" };

    const params = {
      name: name.trim(),
      memory_mb: Number(memoryMb),
      vcpus: Number(vcpus),
      os_type: selectedVariant.os_type,
      machine_type: d.machine_type,
      arch: "x86_64",
      firmware: d.firmware,
      disk_bus: d.disk_bus,
      nic_model: d.nic_model,
      video_model: d.video_model,
      disk_source,
      network,
      install_media: mode === "iso" && isoPath.trim() ? { iso_path: isoPath.trim() } : {},
      graphics,
    };

    try {
      await invoke("create_vm", { params, start: startNow });
      await refreshVms();
      close();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
      busy = false;
    }
  }

  let linuxV = $derived(osVariants.filter(v => v.os_type === "linux"));
  let windowsV = $derived(osVariants.filter(v => v.os_type === "windows"));
  let bsdV = $derived(osVariants.filter(v => v.os_type === "bsd"));
</script>

{#if open}
  <div class="backdrop" onclick={close} role="presentation">
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="wz-title">
      <div class="header">
        <h3 id="wz-title">Create Virtual Machine</h3>
        {#if mode}
          <div class="stepper">
            {#each Array(totalSteps) as _, i}
              <div class="dot" class:active={i === step} class:done={i < step}></div>
            {/each}
          </div>
        {/if}
      </div>

      <div class="body">
        {#if step === 0}
          <h4>How do you want to create this VM?</h4>
          <div class="modes">
            <button class="mode-card" class:active={mode === "iso"} onclick={() => mode = "iso"}>
              <div class="mode-title">Install from ISO</div>
              <div class="mode-desc">Boot from a CD/DVD image and run the OS installer. New disk for the installation.</div>
            </button>
            <button class="mode-card" class:active={mode === "import"} onclick={() => mode = "import"}>
              <div class="mode-title">Import existing disk</div>
              <div class="mode-desc">Wrap an existing qcow2 or raw image (e.g. cloud image, prior VM) as the boot disk.</div>
            </button>
            <button class="mode-card" class:active={mode === "empty"} onclick={() => mode = "empty"}>
              <div class="mode-title">Empty VM</div>
              <div class="mode-desc">Bare definition with a fresh disk. No install media. Edit the config tabs before first boot.</div>
            </button>
          </div>

        {:else if mode === "iso"}
          {#if step === 1}
            <h4>Name & Operating System</h4>
            <label><span>VM Name</span><input bind:value={name} placeholder="my-vm" required /></label>
            <label>
              <span>Operating System</span>
              <select bind:value={osVariantId}>
                <optgroup label="Linux">{#each linuxV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
                <optgroup label="Windows">{#each windowsV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
                <optgroup label="BSD">{#each bsdV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
              </select>
            </label>
          {:else if step === 2}
            <h4>CPU & Memory</h4>
            <label><span>vCPUs</span><input type="number" min="1" max="64" bind:value={vcpus} /></label>
            <label><span>Memory (MiB)</span><input type="number" min="128" step="128" bind:value={memoryMb} /></label>
          {:else if step === 3}
            <h4>Storage</h4>
            <div class="choice">
              <label class="radio-card" class:active={diskMode === "new"}>
                <input type="radio" value="new" bind:group={diskMode} />
                <div><div class="rc-title">New disk</div><div class="rc-desc">Create a new volume in a pool</div></div>
              </label>
              <label class="radio-card" class:active={diskMode === "existing"}>
                <input type="radio" value="existing" bind:group={diskMode} />
                <div><div class="rc-title">Existing</div><div class="rc-desc">Attach a volume already in a pool</div></div>
              </label>
            </div>
            {#if diskMode === "new"}
              <label><span>Pool</span>
                <select bind:value={diskPoolName}>{#each appState.pools.filter(p => p.is_active) as p}<option value={p.name}>{p.name}</option>{/each}</select>
              </label>
              <div class="row">
                <label><span>Size (GB)</span><input type="number" min="1" bind:value={diskSizeGb} /></label>
                <label><span>Format</span><select bind:value={diskFormat}><option value="qcow2">qcow2</option><option value="raw">raw</option></select></label>
              </div>
            {:else}
              <label><span>Pool</span>
                <select bind:value={diskPoolName}>{#each appState.pools.filter(p => p.is_active) as p}<option value={p.name}>{p.name}</option>{/each}</select>
              </label>
              <label><span>Volume</span>
                <select bind:value={existingVolumePath}>
                  <option value="">— select —</option>
                  {#each volumesForPool as v}<option value={v.path}>{v.name}</option>{/each}
                </select>
              </label>
            {/if}
          {:else if step === 4}
            <h4>Install Media + Network</h4>
            <label>
              <span>Install ISO Path *</span>
              <input bind:value={isoPath} placeholder="/var/lib/libvirt/boot/installer.iso" />
              <small class="hint">Absolute path on the hypervisor. VM will boot from CD first.</small>
            </label>
            <div class="net-section">
              <div class="choice">
                <label class="radio-card" class:active={networkMode === "network"}>
                  <input type="radio" value="network" bind:group={networkMode} />
                  <div><div class="rc-title">Virtual Network</div><div class="rc-desc">Libvirt-managed</div></div>
                </label>
                <label class="radio-card" class:active={networkMode === "bridge"}>
                  <input type="radio" value="bridge" bind:group={networkMode} />
                  <div><div class="rc-title">Host Bridge</div><div class="rc-desc">Existing host bridge</div></div>
                </label>
                <label class="radio-card" class:active={networkMode === "none"}>
                  <input type="radio" value="none" bind:group={networkMode} />
                  <div><div class="rc-title">None</div><div class="rc-desc">No NIC</div></div>
                </label>
              </div>
              {#if networkMode === "network"}
                <label><span>Network</span>
                  <select bind:value={networkName}>
                    {#each appState.networks as n}<option value={n.name}>{n.name} ({n.forward_mode})</option>{/each}
                  </select>
                </label>
              {:else if networkMode === "bridge"}
                <label><span>Bridge Name</span><input bind:value={bridgeName} placeholder="br0" /></label>
              {/if}
            </div>
          {/if}

        {:else if mode === "import"}
          {#if step === 1}
            <h4>Disk to Import</h4>
            <label><span>VM Name</span><input bind:value={name} placeholder="my-imported-vm" required /></label>
            <label><span>Pool</span>
              <select bind:value={importPoolName}>{#each appState.pools.filter(p => p.is_active) as p}<option value={p.name}>{p.name}</option>{/each}</select>
            </label>
            <label><span>Disk image</span>
              <select bind:value={importVolumePath}>
                <option value="">— select —</option>
                {#each importVolumes as v}<option value={v.path}>{v.name} ({v.format})</option>{/each}
              </select>
              <small class="hint">Pick an existing qcow2 or raw image to use as the boot disk.</small>
            </label>
          {:else if step === 2}
            <h4>Operating System (for sane defaults)</h4>
            <label>
              <span>Operating System</span>
              <select bind:value={osVariantId}>
                <optgroup label="Linux">{#each linuxV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
                <optgroup label="Windows">{#each windowsV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
                <optgroup label="BSD">{#each bsdV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
              </select>
            </label>
          {:else if step === 3}
            <h4>CPU, Memory & Network</h4>
            <div class="row">
              <label><span>vCPUs</span><input type="number" min="1" max="64" bind:value={vcpus} /></label>
              <label><span>Memory (MiB)</span><input type="number" min="128" step="128" bind:value={memoryMb} /></label>
            </div>
            <div class="net-section">
              <div class="choice">
                <label class="radio-card" class:active={networkMode === "network"}>
                  <input type="radio" value="network" bind:group={networkMode} />
                  <div><div class="rc-title">Virtual Network</div><div class="rc-desc">Libvirt-managed</div></div>
                </label>
                <label class="radio-card" class:active={networkMode === "bridge"}>
                  <input type="radio" value="bridge" bind:group={networkMode} />
                  <div><div class="rc-title">Host Bridge</div><div class="rc-desc">Existing host bridge</div></div>
                </label>
                <label class="radio-card" class:active={networkMode === "none"}>
                  <input type="radio" value="none" bind:group={networkMode} />
                  <div><div class="rc-title">None</div><div class="rc-desc">No NIC</div></div>
                </label>
              </div>
              {#if networkMode === "network"}
                <label><span>Network</span>
                  <select bind:value={networkName}>
                    {#each appState.networks as n}<option value={n.name}>{n.name} ({n.forward_mode})</option>{/each}
                  </select>
                </label>
              {:else if networkMode === "bridge"}
                <label><span>Bridge Name</span><input bind:value={bridgeName} placeholder="br0" /></label>
              {/if}
            </div>
          {/if}

        {:else if mode === "empty"}
          {#if step === 1}
            <h4>Name & OS Defaults</h4>
            <label><span>VM Name</span><input bind:value={name} placeholder="my-vm" required /></label>
            <label>
              <span>OS Defaults (you can change anything later)</span>
              <select bind:value={osVariantId}>
                <optgroup label="Linux">{#each linuxV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
                <optgroup label="Windows">{#each windowsV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
                <optgroup label="BSD">{#each bsdV as v}<option value={v.id}>{v.label}</option>{/each}</optgroup>
              </select>
            </label>
          {:else if step === 2}
            <h4>Minimum CPU & Memory</h4>
            <label><span>vCPUs</span><input type="number" min="1" max="64" bind:value={vcpus} /></label>
            <label><span>Memory (MiB)</span><input type="number" min="128" step="128" bind:value={memoryMb} /></label>
            <label><span>Disk size (GB) — quick default</span><input type="number" min="1" bind:value={diskSizeGb} /></label>
            <small class="hint">A new {diskFormat} volume will be created in <code>{diskPoolName}</code>. You can swap it later.</small>
          {/if}
        {/if}

        {#if step === totalSteps - 1 && mode}
          <hr/>
          <label class="toggle">
            <input type="checkbox" bind:checked={startNow} />
            <span>Start VM after creation</span>
          </label>
        {/if}

        {#if err}<div class="error">{err}</div>{/if}
      </div>

      <div class="footer">
        <button class="btn" onclick={close} disabled={busy}>Cancel</button>
        <div style="flex:1"></div>
        {#if step > 0}
          <button class="btn" onclick={() => step -= 1} disabled={busy}>Back</button>
        {/if}
        {#if step < totalSteps - 1}
          <button class="btn btn-primary" onclick={() => step += 1} disabled={busy || !canAdvance()}>Next</button>
        {:else if mode}
          <button class="btn btn-primary" onclick={submit} disabled={busy || !canAdvance()}>
            {busy ? "Creating..." : "Create"}
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.55);
    display: flex; align-items: center; justify-content: center; z-index: 100; padding: 20px; }
  .dialog { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 12px; width: 640px; max-width: 100%; max-height: 92vh;
    display: flex; flex-direction: column; box-shadow: 0 12px 40px rgba(0,0,0,0.4); overflow: hidden; }

  .header { padding: 20px 24px 16px; border-bottom: 1px solid var(--border); }
  .header h3 { margin: 0 0 12px; font-size: 16px; font-weight: 600; }
  .stepper { display: flex; gap: 6px; }
  .dot { width: 32px; height: 4px; border-radius: 2px; background: var(--bg-button); }
  .dot.done { background: var(--accent); }
  .dot.active { background: var(--accent); opacity: 0.7; }

  .body { padding: 20px 24px; overflow-y: auto; display: flex; flex-direction: column; gap: 14px; flex: 1; }
  h4 { margin: 0; font-size: 14px; font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }

  .modes { display: flex; flex-direction: column; gap: 8px; }
  .mode-card {
    text-align: left; background: var(--bg-button); border: 1px solid var(--border);
    color: var(--text); border-radius: 8px; padding: 14px 16px; cursor: pointer;
    font-family: inherit; display: flex; flex-direction: column; gap: 4px;
  }
  .mode-card:hover { background: var(--bg-hover); }
  .mode-card.active { border-color: var(--accent); background: var(--accent-dim); }
  .mode-title { font-size: 14px; font-weight: 600; }
  .mode-desc { font-size: 12px; color: var(--text-muted); }

  label { display: flex; flex-direction: column; gap: 4px; }
  label > span { font-size: 11px; font-weight: 500; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  small.hint { font-size: 11px; color: var(--text-muted); margin-top: 2px; }
  .row { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }

  input[type="text"], input:not([type]), input[type="number"], select {
    padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit; outline: none;
  }
  input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  .choice { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .radio-card {
    border: 1px solid var(--border); border-radius: 8px; padding: 10px 12px;
    cursor: pointer; display: flex; gap: 6px; align-items: flex-start;
  }
  .radio-card.active { border-color: var(--accent); background: var(--accent-dim); }
  .radio-card input { margin-top: 3px; flex-shrink: 0; }
  .rc-title { font-size: 13px; font-weight: 600; }
  .rc-desc { font-size: 11px; color: var(--text-muted); line-height: 1.35; }

  .toggle { flex-direction: row; align-items: center; gap: 8px; }
  .toggle input { margin: 0; }
  .toggle span { text-transform: none; letter-spacing: normal; color: var(--text); font-size: 13px; font-weight: 400; }

  .error { padding: 8px 12px; background: rgba(239,68,68,0.1);
    border: 1px solid rgba(239,68,68,0.3); border-radius: 6px; color: #ef4444; font-size: 12px; }
  hr { border: none; border-top: 1px solid var(--border); margin: 4px 0; }

  .footer {
    padding: 14px 24px; border-top: 1px solid var(--border);
    display: flex; gap: 8px; align-items: center;
  }
  .btn { padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; font-family: inherit; cursor: pointer; }
  .btn:hover { background: var(--bg-hover); }
  .btn-primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
