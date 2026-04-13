<script>
  import { invoke } from "@tauri-apps/api/core";
  import { getState, refreshVms, clearError } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false) } = $props();
  const appState = getState();

  let step = $state(1);
  let osVariants = $state([]);
  let busy = $state(false);
  let err = $state(null);

  // ── Step 1: Name & OS
  let name = $state("");
  let osVariantId = $state("fedora");

  // ── Step 2: CPU & Memory
  let vcpus = $state(2);
  let memoryMb = $state(2048);

  // ── Step 3: Storage
  let diskMode = $state("new"); // "new" | "existing"
  let diskPoolName = $state("");
  let diskSizeGb = $state(20);
  let diskFormat = $state("qcow2");
  let existingVolumePath = $state("");
  let volumesForPool = $state([]);

  // ── Step 4: Network + Install
  let networkMode = $state("network"); // "network" | "bridge" | "none"
  let networkName = $state("");
  let bridgeName = $state("");
  let isoPath = $state("");
  let startNow = $state(true);

  let graphics = $state("vnc"); // vnc | spice | none

  async function init() {
    if (osVariants.length === 0) {
      osVariants = await invoke("list_os_variants");
    }
    const pools = appState.pools.filter(p => p.is_active);
    if (pools.length > 0 && !diskPoolName) diskPoolName = pools[0].name;
    const nets = appState.networks.filter(n => n.is_active);
    if (nets.length > 0 && !networkName) networkName = nets[0].name;
  }

  $effect(() => {
    if (open) init();
  });

  let selectedVariant = $derived(osVariants.find(v => v.id === osVariantId) ?? null);
  let linuxVariants = $derived(osVariants.filter(v => v.os_type === "linux"));
  let windowsVariants = $derived(osVariants.filter(v => v.os_type === "windows"));
  let bsdVariants = $derived(osVariants.filter(v => v.os_type === "bsd"));

  async function reloadVolumesForPool() {
    if (!diskPoolName) { volumesForPool = []; return; }
    try {
      volumesForPool = await invoke("list_volumes", { poolName: diskPoolName });
    } catch (e) {
      volumesForPool = [];
    }
  }

  $effect(() => {
    if (diskMode === "existing" && diskPoolName) reloadVolumesForPool();
  });

  function reset() {
    step = 1; busy = false; err = null;
    name = ""; osVariantId = "fedora";
    vcpus = 2; memoryMb = 2048;
    diskMode = "new"; diskSizeGb = 20; diskFormat = "qcow2"; existingVolumePath = "";
    networkMode = "network"; networkName = ""; bridgeName = "";
    isoPath = ""; startNow = true; graphics = "vnc";
  }
  function close() { open = false; reset(); }

  function canAdvance() {
    switch (step) {
      case 1: return name.trim() && osVariantId;
      case 2: return vcpus >= 1 && memoryMb >= 128;
      case 3: {
        if (diskMode === "new") return diskPoolName && diskSizeGb > 0;
        return !!existingVolumePath;
      }
      case 4: {
        if (networkMode === "network") return !!networkName;
        if (networkMode === "bridge") return !!bridgeName;
        return true; // "none" is fine
      }
      default: return true;
    }
  }

  async function submit() {
    if (!selectedVariant) return;
    busy = true; err = null;
    const d = selectedVariant.defaults;

    const disk_source = diskMode === "new"
      ? {
          kind: "new_volume",
          pool_name: diskPoolName,
          name: `${name.trim()}.${diskFormat}`,
          capacity_bytes: Math.floor(diskSizeGb * 1024 * 1024 * 1024),
          format: diskFormat,
        }
      : {
          kind: "existing_path",
          path: existingVolumePath,
          format: existingVolumePath.endsWith(".iso") ? "raw" : "qcow2",
        };

    let network;
    if (networkMode === "network") network = { kind: "network", name: networkName };
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
      install_media: isoPath.trim() ? { iso_path: isoPath.trim() } : {},
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
</script>

{#if open}
  <div class="backdrop" onclick={close} role="presentation">
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="wz-title">
      <div class="header">
        <h3 id="wz-title">Create Virtual Machine</h3>
        <div class="stepper">
          {#each [1, 2, 3, 4, 5] as i}
            <div class="dot" class:active={i === step} class:done={i < step}></div>
          {/each}
        </div>
      </div>

      <div class="body">
        {#if step === 1}
          <h4>Name & Operating System</h4>
          <label>
            <span>VM Name</span>
            <input bind:value={name} placeholder="my-vm" required />
          </label>

          <label>
            <span>Operating System</span>
            <select bind:value={osVariantId}>
              {#if linuxVariants.length > 0}
                <optgroup label="Linux">
                  {#each linuxVariants as v}<option value={v.id}>{v.label}</option>{/each}
                </optgroup>
              {/if}
              {#if windowsVariants.length > 0}
                <optgroup label="Windows">
                  {#each windowsVariants as v}<option value={v.id}>{v.label}</option>{/each}
                </optgroup>
              {/if}
              {#if bsdVariants.length > 0}
                <optgroup label="BSD">
                  {#each bsdVariants as v}<option value={v.id}>{v.label}</option>{/each}
                </optgroup>
              {/if}
            </select>
          </label>

          {#if selectedVariant}
            <div class="os-summary">
              <div><span class="k">Disk Bus</span><span class="v">{selectedVariant.defaults.disk_bus}</span></div>
              <div><span class="k">NIC Model</span><span class="v">{selectedVariant.defaults.nic_model}</span></div>
              <div><span class="k">Video</span><span class="v">{selectedVariant.defaults.video_model}</span></div>
              <div><span class="k">Firmware</span><span class="v">{selectedVariant.defaults.firmware}</span></div>
            </div>
          {/if}

        {:else if step === 2}
          <h4>CPU & Memory</h4>
          <label>
            <span>vCPUs</span>
            <input type="number" min="1" max="64" bind:value={vcpus} />
          </label>

          <label>
            <span>Memory (MiB)</span>
            <input type="number" min="128" step="128" bind:value={memoryMb} />
          </label>

        {:else if step === 3}
          <h4>Storage</h4>

          <div class="choice">
            <label class="radio-card" class:active={diskMode === "new"}>
              <input type="radio" name="disk-mode" value="new" bind:group={diskMode} />
              <div>
                <div class="rc-title">Create new disk</div>
                <div class="rc-desc">New qcow2/raw volume in an existing pool</div>
              </div>
            </label>
            <label class="radio-card" class:active={diskMode === "existing"}>
              <input type="radio" name="disk-mode" value="existing" bind:group={diskMode} />
              <div>
                <div class="rc-title">Use existing</div>
                <div class="rc-desc">Attach a volume already in a pool</div>
              </div>
            </label>
          </div>

          {#if diskMode === "new"}
            <label>
              <span>Pool</span>
              <select bind:value={diskPoolName}>
                {#each appState.pools.filter(p => p.is_active) as p}
                  <option value={p.name}>{p.name} ({p.pool_type})</option>
                {/each}
              </select>
            </label>
            <div class="row">
              <label>
                <span>Size (GB)</span>
                <input type="number" min="1" step="1" bind:value={diskSizeGb} />
              </label>
              <label>
                <span>Format</span>
                <select bind:value={diskFormat}>
                  <option value="qcow2">qcow2</option>
                  <option value="raw">raw</option>
                </select>
              </label>
            </div>
          {:else}
            <label>
              <span>Pool</span>
              <select bind:value={diskPoolName}>
                {#each appState.pools.filter(p => p.is_active) as p}
                  <option value={p.name}>{p.name}</option>
                {/each}
              </select>
            </label>
            <label>
              <span>Volume</span>
              <select bind:value={existingVolumePath}>
                <option value="">— select a volume —</option>
                {#each volumesForPool as v}
                  <option value={v.path}>{v.name} ({v.format})</option>
                {/each}
              </select>
            </label>
          {/if}

        {:else if step === 4}
          <h4>Network & Install Media</h4>

          <div class="choice">
            <label class="radio-card" class:active={networkMode === "network"}>
              <input type="radio" value="network" bind:group={networkMode} />
              <div>
                <div class="rc-title">Virtual Network</div>
                <div class="rc-desc">Libvirt-managed network</div>
              </div>
            </label>
            <label class="radio-card" class:active={networkMode === "bridge"}>
              <input type="radio" value="bridge" bind:group={networkMode} />
              <div>
                <div class="rc-title">Host Bridge</div>
                <div class="rc-desc">Existing bridge on the host</div>
              </div>
            </label>
            <label class="radio-card" class:active={networkMode === "none"}>
              <input type="radio" value="none" bind:group={networkMode} />
              <div>
                <div class="rc-title">None</div>
                <div class="rc-desc">No network interface</div>
              </div>
            </label>
          </div>

          {#if networkMode === "network"}
            <label>
              <span>Network</span>
              <select bind:value={networkName}>
                {#each appState.networks as n}
                  <option value={n.name}>{n.name} ({n.forward_mode})</option>
                {/each}
              </select>
            </label>
          {:else if networkMode === "bridge"}
            <label>
              <span>Bridge Name</span>
              <input bind:value={bridgeName} placeholder="br0" />
            </label>
          {/if}

          <label>
            <span>Install ISO (optional)</span>
            <input bind:value={isoPath} placeholder="/var/lib/libvirt/boot/installer.iso" />
            <small class="hint">Absolute path on the hypervisor. If set, VM boots from CD first.</small>
          </label>

          <label>
            <span>Graphics</span>
            <select bind:value={graphics}>
              <option value="vnc">VNC</option>
              <option value="spice">SPICE</option>
              <option value="none">None (serial only)</option>
            </select>
          </label>

        {:else if step === 5}
          <h4>Review</h4>
          <dl class="review">
            <dt>Name</dt><dd>{name}</dd>
            <dt>OS</dt><dd>{selectedVariant?.label ?? osVariantId}</dd>
            <dt>vCPUs</dt><dd>{vcpus}</dd>
            <dt>Memory</dt><dd>{memoryMb} MiB</dd>
            <dt>Disk</dt>
            <dd>
              {#if diskMode === "new"}
                New {diskFormat} in {diskPoolName} — {diskSizeGb} GB
              {:else}
                {existingVolumePath || "(none)"}
              {/if}
            </dd>
            <dt>Network</dt>
            <dd>
              {#if networkMode === "network"}Virtual: {networkName}
              {:else if networkMode === "bridge"}Bridge: {bridgeName}
              {:else}None{/if}
            </dd>
            {#if isoPath}<dt>Install ISO</dt><dd class="mono">{isoPath}</dd>{/if}
            <dt>Graphics</dt><dd>{graphics.toUpperCase()}</dd>
            <dt>Firmware</dt><dd>{selectedVariant?.defaults.firmware}</dd>
          </dl>

          <label class="toggle">
            <input type="checkbox" bind:checked={startNow} />
            <span>Start VM after creation</span>
          </label>
        {/if}

        {#if err}<div class="error">{err}</div>{/if}
      </div>

      <div class="footer">
        <button type="button" class="btn" onclick={close} disabled={busy}>Cancel</button>
        <div style="flex:1"></div>
        {#if step > 1}
          <button type="button" class="btn" onclick={() => step -= 1} disabled={busy}>Back</button>
        {/if}
        {#if step < 5}
          <button type="button" class="btn btn-primary" onclick={() => step += 1} disabled={busy || !canAdvance()}>Next</button>
        {:else}
          <button type="button" class="btn btn-primary" onclick={submit} disabled={busy}>
            {busy ? "Creating..." : "Create VM"}
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
    display: flex; flex-direction: column;
    box-shadow: 0 12px 40px rgba(0,0,0,0.4); overflow: hidden; }

  .header { padding: 20px 24px 16px; border-bottom: 1px solid var(--border); }
  .header h3 { margin: 0 0 12px; font-size: 16px; font-weight: 600; }

  .stepper { display: flex; gap: 6px; }
  .dot { width: 32px; height: 4px; border-radius: 2px; background: var(--bg-button); }
  .dot.done { background: var(--accent); }
  .dot.active { background: var(--accent); opacity: 0.7; }

  .body { padding: 20px 24px; overflow-y: auto; display: flex; flex-direction: column; gap: 14px; flex: 1; }
  h4 { margin: 0; font-size: 14px; font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }

  label { display: flex; flex-direction: column; gap: 4px; }
  label > span { font-size: 11px; font-weight: 500; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  small.hint { font-size: 11px; color: var(--text-muted); margin-top: 2px; }

  input[type="text"], input:not([type]), input[type="number"], select {
    padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit; outline: none;
  }
  input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  .row { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }

  .os-summary {
    display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px;
    padding: 10px; background: var(--bg-sidebar); border-radius: 6px;
  }
  .os-summary > div { display: flex; flex-direction: column; gap: 2px; }
  .k { font-size: 10px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  .v { font-size: 12px; font-family: 'SF Mono', monospace; }

  .choice { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; }
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

  .review { display: grid; grid-template-columns: 120px 1fr; gap: 6px 12px; margin: 0; font-size: 13px; }
  dt { color: var(--text-muted); }
  dd { margin: 0; word-break: break-all; }
  .mono { font-family: 'SF Mono', monospace; font-size: 12px; }

  .error { padding: 8px 12px; background: rgba(239,68,68,0.1);
    border: 1px solid rgba(239,68,68,0.3); border-radius: 6px; color: #ef4444; font-size: 12px; }

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
