<script>
  /*
   * Round I: Advanced CPU + memory tuning + iothreads editor.
   *
   * Most users will never touch this tab — it exists for parity with
   * virt-manager. Everything is persistent (next boot); vCPU count and
   * iothread count have dedicated live-apply buttons where supported.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState } from "$lib/stores/app.svelte.js";

  let { vmName } = $props();
  const appState = getState();

  let cfg = $state(null);     // saved snapshot
  let edit = $state(null);    // mutable copy
  let caps = $state(null);    // DomainCaps (for custom model picker)
  let nested = $state(null);  // NestedVirtState
  let nestedBusy = $state(false);
  let loading = $state(true);
  let busy = $state(false);
  let err = $state(null);
  let saved = $state(null);

  const CPU_MODES = ["host-passthrough", "host-model", "custom", "maximum"];
  const CHECK = ["none", "partial", "full"];
  const POLICIES = ["force", "require", "optional", "disable", "forbid"];
  const CACHE_MODES = ["passthrough", "emulate", "disable"];
  const MEM_ACCESS = ["", "shared", "private"];

  async function reload() {
    loading = true; err = null;
    try {
      const [s, dc, nv] = await Promise.all([
        invoke("get_cpu_tune", { name: vmName }),
        invoke("get_domain_capabilities", {}).catch(() => null),
        invoke("get_nested_virt_state", { name: vmName }).catch(() => null),
      ]);
      cfg = s;
      edit = deepClone(s);
      caps = dc;
      nested = nv;
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { if (vmName) reload(); });

  async function toggleNested() {
    if (!nested || nestedBusy) return;
    if (nested.cpu_mode === "host-passthrough") return;
    nestedBusy = true;
    err = null;
    try {
      await invoke("set_nested_virt", { name: vmName, enable: !nested.enabled_in_domain });
      // Refresh just the nested slice — full reload would clobber edits.
      nested = await invoke("get_nested_virt_state", { name: vmName });
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      nestedBusy = false;
    }
  }

  function deepClone(x) { return JSON.parse(JSON.stringify(x)); }

  let dirty = $derived(() =>
    cfg && edit && JSON.stringify(cfg) !== JSON.stringify(edit)
  );

  let isRunning = $derived(appState.selectedVm?.state === "running");

  // Flatten a patch: only fields that differ from cfg.
  function buildPatch() {
    const patch = {};
    if (JSON.stringify(edit.cpu) !== JSON.stringify(cfg.cpu)) patch.cpu = edit.cpu;
    if (JSON.stringify(edit.vcpus) !== JSON.stringify(cfg.vcpus)) patch.vcpus = edit.vcpus;
    if (JSON.stringify(edit.cputune) !== JSON.stringify(cfg.cputune)) patch.cputune = edit.cputune;
    if (JSON.stringify(edit.memtune) !== JSON.stringify(cfg.memtune)) patch.memtune = edit.memtune;
    if (JSON.stringify(edit.numa) !== JSON.stringify(cfg.numa)) patch.numa = edit.numa;
    if (JSON.stringify(edit.hugepages) !== JSON.stringify(cfg.hugepages)) patch.hugepages = edit.hugepages;
    if (JSON.stringify(edit.iothreads) !== JSON.stringify(cfg.iothreads)) patch.iothreads = edit.iothreads;
    return patch;
  }

  async function save() {
    busy = true; err = null;
    try {
      await invoke("apply_cpu_tune", { name: vmName, patch: buildPatch() });
      saved = Date.now();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  function discard() {
    if (cfg) edit = deepClone(cfg);
    err = null;
  }

  // Live vCPU hotplug.
  async function liveVcpus() {
    busy = true; err = null;
    try {
      await invoke("set_vcpu_count", {
        name: vmName, current: edit.vcpus.current, live: true, config: true,
      });
      saved = Date.now();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function applyIoThreads() {
    busy = true; err = null;
    try {
      await invoke("set_iothread_count", { name: vmName, count: edit.iothreads.count });
      saved = Date.now();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  // ── feature add/remove ──
  function addFeature() {
    edit.cpu.features = [...edit.cpu.features, { name: "", policy: "require" }];
  }
  function removeFeature(i) {
    edit.cpu.features = edit.cpu.features.filter((_, j) => j !== i);
  }

  // ── topology ──
  function ensureTopology() {
    if (!edit.cpu.topology) {
      edit.cpu.topology = { sockets: 1, dies: 1, cores: 1, threads: 1 };
    }
  }
  function clearTopology() { edit.cpu.topology = null; }

  // ── cache ──
  function ensureCache() {
    if (!edit.cpu.cache) edit.cpu.cache = { mode: "passthrough", level: null };
  }
  function clearCache() { edit.cpu.cache = null; }

  // ── cputune pins ──
  function addVcpupin() {
    edit.cputune.vcpupin = [...edit.cputune.vcpupin, { vcpu: 0, cpuset: "" }];
  }
  function removeVcpupin(i) {
    edit.cputune.vcpupin = edit.cputune.vcpupin.filter((_, j) => j !== i);
  }
  function addIoThreadPin() {
    edit.cputune.iothreadpin = [...edit.cputune.iothreadpin, { iothread: 1, cpuset: "" }];
  }
  function removeIoThreadPin(i) {
    edit.cputune.iothreadpin = edit.cputune.iothreadpin.filter((_, j) => j !== i);
  }

  // ── NUMA cells ──
  function addCell() {
    const nextId = edit.numa.cells.length
      ? Math.max(...edit.numa.cells.map((c) => c.id)) + 1
      : 0;
    edit.numa.cells = [
      ...edit.numa.cells,
      { id: nextId, cpus: "", memory_kib: 1048576, memory_unit: "KiB",
        distances: [], memory_access: null },
    ];
  }
  function removeCell(i) {
    edit.numa.cells = edit.numa.cells.filter((_, j) => j !== i);
  }
  function addDistance(ci) {
    edit.numa.cells[ci].distances = [
      ...edit.numa.cells[ci].distances,
      { cell_id: 0, value: 10 },
    ];
  }
  function removeDistance(ci, di) {
    edit.numa.cells[ci].distances = edit.numa.cells[ci].distances
      .filter((_, j) => j !== di);
  }

  // ── hugepages ──
  function addPage() {
    edit.hugepages.pages = [...edit.hugepages.pages, { size_kib: 2048, nodeset: null }];
  }
  function removePage(i) {
    edit.hugepages.pages = edit.hugepages.pages.filter((_, j) => j !== i);
  }

  let customModels = $derived(caps?.cpu?.custom_models ?? []);
</script>

<div class="tune">
  {#if loading}
    <p class="muted">Loading...</p>
  {:else if edit}
    {#if err}<div class="error">{err}</div>{/if}
    {#if isRunning}
      <div class="notice">
        Most tuning takes effect on next boot. vCPU count and iothreads
        have dedicated live-apply buttons.
      </div>
    {/if}

    <!-- ── Nested virtualization (phase 5.4) ── -->
    {#if nested}
      <section class="nested">
        <h3>Nested virtualization</h3>
        <div class="nested-row">
          <span class="vendor-pill" class:intel={nested.vendor === "intel"} class:amd={nested.vendor === "amd"}>
            {nested.vendor === "unknown" ? "?" : nested.vendor}
          </span>
          <span class="muted small">domain mode <code>{nested.cpu_mode || "(none)"}</code></span>
          <span class="grow"></span>
          {#if nested.cpu_mode === "host-passthrough"}
            <span class="muted small">inherits from host</span>
          {:else if nested.vendor === "unknown"}
            <span class="muted small">unknown vendor — can't toggle</span>
          {:else}
            <button class="btn-small" class:on={nested.enabled_in_domain}
              onclick={toggleNested} disabled={nestedBusy || busy}>
              {nestedBusy ? "…" : nested.enabled_in_domain ? "Enabled — click to disable" : "Disabled — click to enable"}
            </button>
          {/if}
        </div>
        <p class="muted small">
          {#if nested.enabled_in_host === true}
            Host kernel module reports <code>nested=Y</code> — VMs that opt in will see {nested.vendor === "intel" ? "vmx" : "svm"}.
          {:else if nested.enabled_in_host === false}
            <strong>Host kernel module reports <code>nested=N</code></strong> — no VM can boot a hypervisor inside until this is fixed on the host (modprobe options or sysfs).
          {:else}
            Host kernel module status unknown (needs SSH access to read /sys/module/kvm_*/parameters/nested).
          {/if}
        </p>
      </section>
    {/if}

    <!-- ── CPU section ── -->
    <section>
      <h3>CPU <span class="badge">restart required</span></h3>
      <div class="grid">
        <label>
          <span>Mode</span>
          <select bind:value={edit.cpu.mode} disabled={busy}>
            {#each CPU_MODES as m}<option value={m}>{m}</option>{/each}
          </select>
        </label>
        {#if edit.cpu.mode === "custom"}
          <label>
            <span>Model</span>
            <select bind:value={edit.cpu.model} disabled={busy}>
              <option value={null}>(none)</option>
              {#each customModels as m}<option value={m}>{m}</option>{/each}
            </select>
          </label>
        {/if}
        <label>
          <span>Check</span>
          <select bind:value={edit.cpu.check} disabled={busy}>
            <option value={null}>(default)</option>
            {#each CHECK as c}<option value={c}>{c}</option>{/each}
          </select>
        </label>
        {#if edit.cpu.mode === "host-passthrough"}
          <label class="toggle">
            <input type="checkbox"
              checked={edit.cpu.migratable === true}
              onchange={(e) => edit.cpu.migratable = e.target.checked ? true : false}
              disabled={busy} />
            <span>Migratable</span>
          </label>
        {/if}
      </div>

      <!-- Topology -->
      <div class="sub">
        <div class="sub-head">
          <strong>Topology</strong>
          {#if edit.cpu.topology}
            <button class="btn-tiny" onclick={clearTopology} disabled={busy}>remove</button>
          {:else}
            <button class="btn-tiny" onclick={ensureTopology} disabled={busy}>+ add</button>
          {/if}
        </div>
        {#if edit.cpu.topology}
          <div class="grid-4">
            <label><span>Sockets</span>
              <input type="number" min="1" bind:value={edit.cpu.topology.sockets} disabled={busy} /></label>
            <label><span>Dies</span>
              <input type="number" min="1" bind:value={edit.cpu.topology.dies} disabled={busy} /></label>
            <label><span>Cores</span>
              <input type="number" min="1" bind:value={edit.cpu.topology.cores} disabled={busy} /></label>
            <label><span>Threads</span>
              <input type="number" min="1" bind:value={edit.cpu.topology.threads} disabled={busy} /></label>
          </div>
        {/if}
      </div>

      <!-- Cache -->
      <div class="sub">
        <div class="sub-head">
          <strong>Cache</strong>
          {#if edit.cpu.cache}
            <button class="btn-tiny" onclick={clearCache} disabled={busy}>remove</button>
          {:else}
            <button class="btn-tiny" onclick={ensureCache} disabled={busy}>+ add</button>
          {/if}
        </div>
        {#if edit.cpu.cache}
          <div class="grid">
            <label><span>Mode</span>
              <select bind:value={edit.cpu.cache.mode} disabled={busy}>
                {#each CACHE_MODES as m}<option value={m}>{m}</option>{/each}
              </select></label>
            <label><span>Level</span>
              <input type="number" min="1" max="3" bind:value={edit.cpu.cache.level} disabled={busy} /></label>
          </div>
        {/if}
      </div>

      <!-- Features -->
      <div class="sub">
        <div class="sub-head">
          <strong>Features</strong>
          <button class="btn-tiny" onclick={addFeature} disabled={busy}>+ add</button>
        </div>
        {#each edit.cpu.features as f, i}
          <div class="row">
            <input placeholder="feature name" bind:value={f.name} disabled={busy} />
            <select bind:value={f.policy} disabled={busy}>
              {#each POLICIES as p}<option value={p}>{p}</option>{/each}
            </select>
            <button class="btn-tiny danger" onclick={() => removeFeature(i)} disabled={busy}>×</button>
          </div>
        {/each}
        {#if !edit.cpu.features.length}<p class="muted">No features.</p>{/if}
      </div>
    </section>

    <!-- ── vCPU section ── -->
    <section>
      <h3>vCPUs <span class="badge">restart for max; live for current</span></h3>
      <div class="grid">
        <label><span>Max (hotplug ceiling)</span>
          <input type="number" min="1" bind:value={edit.vcpus.max} disabled={busy} /></label>
        <label><span>Current (online)</span>
          <input type="number" min="1" max={edit.vcpus.max}
            bind:value={edit.vcpus.current} disabled={busy} /></label>
        <label><span>Placement</span>
          <select bind:value={edit.vcpus.placement} disabled={busy}>
            <option value={null}>(default)</option>
            <option value="static">static</option>
            <option value="auto">auto</option>
          </select></label>
        <label><span>cpuset</span>
          <input placeholder="0-3,8-11" bind:value={edit.vcpus.cpuset} disabled={busy} /></label>
      </div>
      {#if isRunning && edit.vcpus.current !== cfg.vcpus.current}
        <button class="btn btn-small" onclick={liveVcpus} disabled={busy}>
          Apply current vCPU count live
        </button>
      {/if}
    </section>

    <!-- ── cputune ── -->
    <section>
      <h3>CPU Tuning <span class="badge">applies to running VM</span></h3>
      <div class="sub">
        <div class="sub-head">
          <strong>vcpu pin</strong>
          <button class="btn-tiny" onclick={addVcpupin} disabled={busy}>+ add</button>
        </div>
        {#each edit.cputune.vcpupin as p, i}
          <div class="row">
            <input type="number" style="width:60px" min="0" bind:value={p.vcpu} disabled={busy} />
            <input placeholder="cpuset" bind:value={p.cpuset} disabled={busy} />
            <button class="btn-tiny danger" onclick={() => removeVcpupin(i)} disabled={busy}>×</button>
          </div>
        {/each}
      </div>
      <div class="sub">
        <label><span>Emulator pin (cpuset)</span>
          <input bind:value={edit.cputune.emulatorpin} disabled={busy} /></label>
      </div>
      <div class="sub">
        <div class="sub-head">
          <strong>iothread pin</strong>
          <button class="btn-tiny" onclick={addIoThreadPin} disabled={busy}>+ add</button>
        </div>
        {#each edit.cputune.iothreadpin as p, i}
          <div class="row">
            <input type="number" style="width:60px" min="1" bind:value={p.iothread} disabled={busy} />
            <input placeholder="cpuset" bind:value={p.cpuset} disabled={busy} />
            <button class="btn-tiny danger" onclick={() => removeIoThreadPin(i)} disabled={busy}>×</button>
          </div>
        {/each}
      </div>
      <div class="grid">
        <label><span>shares</span><input type="number" bind:value={edit.cputune.shares} disabled={busy} /></label>
        <label><span>period (µs)</span><input type="number" bind:value={edit.cputune.period_us} disabled={busy} /></label>
        <label><span>quota (µs, -1=unlim)</span><input type="number" bind:value={edit.cputune.quota_us} disabled={busy} /></label>
        <label><span>emulator period</span><input type="number" bind:value={edit.cputune.emulator_period_us} disabled={busy} /></label>
        <label><span>emulator quota</span><input type="number" bind:value={edit.cputune.emulator_quota_us} disabled={busy} /></label>
      </div>
    </section>

    <!-- ── memtune ── -->
    <section>
      <h3>Memory Limits (KiB) <span class="badge">restart required</span></h3>
      <div class="grid">
        <label><span>hard_limit</span><input type="number" bind:value={edit.memtune.hard_limit_kib} disabled={busy} /></label>
        <label><span>soft_limit</span><input type="number" bind:value={edit.memtune.soft_limit_kib} disabled={busy} /></label>
        <label><span>swap_hard_limit</span><input type="number" bind:value={edit.memtune.swap_hard_limit_kib} disabled={busy} /></label>
        <label><span>min_guarantee</span><input type="number" bind:value={edit.memtune.min_guarantee_kib} disabled={busy} /></label>
      </div>
    </section>

    <!-- ── NUMA ── -->
    <section>
      <h3>NUMA <span class="badge">restart required</span></h3>
      <button class="btn-tiny" onclick={addCell} disabled={busy}>+ add cell</button>
      {#each edit.numa.cells as cell, ci}
        <div class="sub">
          <div class="sub-head">
            <strong>Cell {cell.id}</strong>
            <button class="btn-tiny danger" onclick={() => removeCell(ci)} disabled={busy}>remove</button>
          </div>
          <div class="grid">
            <label><span>id</span><input type="number" min="0" bind:value={cell.id} disabled={busy} /></label>
            <label><span>cpus</span><input placeholder="0-3" bind:value={cell.cpus} disabled={busy} /></label>
            <label><span>memory (KiB)</span><input type="number" bind:value={cell.memory_kib} disabled={busy} /></label>
            <label><span>memAccess</span>
              <select bind:value={cell.memory_access} disabled={busy}>
                <option value={null}>(none)</option>
                <option value="shared">shared</option>
                <option value="private">private</option>
              </select></label>
          </div>
          <div class="sub">
            <div class="sub-head">
              <strong>Distances</strong>
              <button class="btn-tiny" onclick={() => addDistance(ci)} disabled={busy}>+ add</button>
            </div>
            {#each cell.distances as d, di}
              <div class="row">
                <input type="number" style="width:60px" bind:value={d.cell_id} disabled={busy} />
                <input type="number" style="width:80px" bind:value={d.value} disabled={busy} />
                <button class="btn-tiny danger" onclick={() => removeDistance(ci, di)} disabled={busy}>×</button>
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </section>

    <!-- ── Hugepages ── -->
    <section>
      <h3>Hugepages <span class="badge">restart required</span> <span class="badge">requires host reservation</span></h3>
      <button class="btn-tiny" onclick={addPage} disabled={busy}>+ add page</button>
      {#each edit.hugepages.pages as p, i}
        <div class="row">
          <label><span>size (KiB)</span><input type="number" bind:value={p.size_kib} disabled={busy} /></label>
          <label><span>nodeset</span><input placeholder="0" bind:value={p.nodeset} disabled={busy} /></label>
          <button class="btn-tiny danger" onclick={() => removePage(i)} disabled={busy}>×</button>
        </div>
      {/each}
    </section>

    <!-- ── iothreads ── -->
    <section>
      <h3>IOThreads</h3>
      <div class="grid">
        <label><span>count</span><input type="number" min="0" bind:value={edit.iothreads.count} disabled={busy} /></label>
      </div>
      {#if edit.iothreads.count !== cfg.iothreads.count}
        <button class="btn btn-small" onclick={applyIoThreads} disabled={busy}>
          Apply iothread count
        </button>
      {/if}
    </section>

    <div class="actions">
      <button class="btn" onclick={discard} disabled={busy || !dirty()}>Discard</button>
      <button class="btn btn-primary" onclick={save} disabled={busy || !dirty()}>
        {busy ? "Saving..." : "Save"}
      </button>
      {#if saved && !dirty()}<span class="saved-note">Saved.</span>{/if}
    </div>
  {/if}
</div>

<style>
  .tune { display: flex; flex-direction: column; gap: 16px; }
  .muted { color: var(--text-muted); font-size: 13px; }
  .error { padding: 8px 12px; background: rgba(239,68,68,0.1);
    border: 1px solid rgba(239,68,68,0.3); border-radius: 6px;
    color: #ef4444; font-size: 12px; }
  .notice { padding: 8px 12px; background: rgba(251,191,36,0.1);
    border: 1px solid rgba(251,191,36,0.3); border-radius: 6px;
    color: #fbbf24; font-size: 12px; }

  section { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 8px; padding: 14px; display: flex; flex-direction: column; gap: 10px; }
  h3 { margin: 0 0 8px; font-size: 11px; font-weight: 600; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em;
    display: flex; align-items: center; gap: 8px; }
  .badge { font-size: 10px; font-weight: 500; padding: 2px 6px; border-radius: 4px;
    background: rgba(251,191,36,0.15); color: #fbbf24; text-transform: none; letter-spacing: 0; }

  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 10px; }
  .grid-4 { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; }
  label { display: flex; flex-direction: column; gap: 3px; font-size: 12px; }
  label > span { font-size: 10px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  input[type="text"], input:not([type]), input[type="number"], select {
    padding: 5px 8px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 12px; font-family: inherit;
    outline: none; min-width: 60px;
  }
  input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  .toggle { flex-direction: row; align-items: center; gap: 6px; font-size: 12px; cursor: pointer; }
  .toggle span { text-transform: none; letter-spacing: normal; color: var(--text); }

  .sub { border-left: 2px solid var(--border); padding-left: 10px; display: flex; flex-direction: column; gap: 8px; }
  .sub-head { display: flex; align-items: center; gap: 8px; font-size: 12px; }
  .row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }

  .btn-tiny { padding: 2px 8px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--bg-button); color: var(--text); font-size: 11px; cursor: pointer; font-family: inherit; }
  .btn-tiny:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-tiny:disabled { opacity: 0.35; cursor: not-allowed; }
  .btn-tiny.danger:hover { background: #7f1d1d; border-color: #7f1d1d; color: #fca5a5; }

  .btn { padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; cursor: pointer; font-family: inherit; }
  .btn-small { padding: 4px 10px; font-size: 12px; align-self: flex-start; }
  .btn:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .actions { display: flex; gap: 8px; align-items: center; padding-top: 4px; }
  .saved-note { color: #34d399; font-size: 12px; margin-left: 8px; }

  section.nested { padding: 12px; background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; }
  section.nested h3 { margin: 0 0 8px; font-size: 13px; font-weight: 600; }
  .nested-row { display: flex; align-items: center; gap: 10px; }
  .grow { flex: 1; }
  .vendor-pill {
    font-size: 10px; padding: 2px 8px; border-radius: 4px;
    background: var(--bg-input); color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em;
  }
  .vendor-pill.intel { background: rgba(96, 165, 250, 0.15); color: #60a5fa; }
  .vendor-pill.amd   { background: rgba(248, 113, 113, 0.18); color: #fca5a5; }
  .btn-small.on { background: rgba(52, 211, 153, 0.15); color: #34d399; border-color: rgba(52, 211, 153, 0.4); }
</style>
