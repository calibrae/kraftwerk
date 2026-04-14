<script>
  /*
   * Virtio-adjacent device editor (Round E).
   *
   * One panel with collapsible sections per device:
   *  - TPM          — persistent, restart required
   *  - RNG          — live hotplug supported; multiple allowed
   *  - Watchdog     — persistent
   *  - Panic        — persistent
   *  - Balloon      — model persistent, stats period live
   *  - Vsock        — live + persistent
   *  - IOMMU        — persistent
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState } from "$lib/stores/app.svelte.js";

  let { vmName } = $props();
  const appState = getState();

  const TPM_MODELS = ["tpm-tis", "tpm-crb", "tpm-spapr"];
  const TPM_BACKENDS = ["passthrough", "emulator", "external"];
  const TPM_VERSIONS = ["1.2", "2.0"];
  const RNG_MODELS = ["virtio", "virtio-transitional", "virtio-non-transitional"];
  const RNG_BACKENDS = ["random", "builtin", "egd"];
  const WATCHDOG_MODELS = ["i6300esb", "ib700", "itco", "diag288"];
  const WATCHDOG_ACTIONS = ["reset", "shutdown", "poweroff", "pause", "dump", "inject-nmi", "none"];
  const PANIC_MODELS = ["isa", "pseries", "hyperv", "s390", "pvpanic"];
  const BALLOON_MODELS = ["virtio", "virtio-transitional", "virtio-non-transitional", "none"];
  const IOMMU_MODELS = ["intel", "smmuv3", "virtio"];

  let snap = $state(null);
  let loading = $state(true);
  let err = $state(null);
  let busy = $state(false);
  let openSection = $state({
    tpm: false, rng: true, watchdog: false, panic: false,
    balloon: false, vsock: false, iommu: false,
  });

  // Local edit buffers.
  let tpmEnabled = $state(false);
  let tpm = $state(null);
  let watchdogEnabled = $state(false);
  let watchdog = $state(null);
  let panicEnabled = $state(false);
  let panicCfg = $state(null);
  let balloonEnabled = $state(false);
  let balloon = $state(null);
  let vsockEnabled = $state(false);
  let vsock = $state(null);
  let iommuEnabled = $state(false);
  let iommu = $state(null);
  let newRng = $state({
    model: "virtio", backend_model: "random",
    source_path: "/dev/urandom", rate_period_ms: null, rate_bytes: null,
  });

  let isRunning = $derived(appState.selectedVm?.state === "running");

  async function reload() {
    loading = true; err = null;
    try {
      snap = await invoke("get_virtio_devices", { name: vmName });
      tpmEnabled = !!snap.tpm;
      tpm = snap.tpm ?? { model: "tpm-crb", backend_model: "emulator", backend_version: "2.0", source_path: null };
      watchdogEnabled = !!snap.watchdog;
      watchdog = snap.watchdog ?? { model: "i6300esb", action: "reset" };
      panicEnabled = !!snap.panic;
      panicCfg = snap.panic ?? { model: "pvpanic" };
      balloonEnabled = !!snap.balloon;
      balloon = snap.balloon ?? {
        model: "virtio", autodeflate: false, freepage_reporting: false, stats_period_secs: null,
      };
      vsockEnabled = !!snap.vsock;
      vsock = snap.vsock ?? { cid: 3, model: "virtio", auto_cid: true };
      iommuEnabled = !!snap.iommu;
      iommu = snap.iommu ?? {
        model: "intel", driver_intremap: false, driver_caching_mode: false,
        driver_eim: false, driver_iotlb: false,
      };
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { if (vmName) reload(); });

  async function run(fn, label) {
    busy = true; err = null;
    try { await fn(); await reload(); }
    catch (e) { err = `${label}: ${e?.message || JSON.stringify(e)}`; }
    finally { busy = false; }
  }

  const saveTpm = () => run(
    () => invoke("set_tpm", { name: vmName, cfg: tpmEnabled ? tpm : null }),
    "TPM",
  );
  const saveWatchdog = () => run(
    () => invoke("set_watchdog", { name: vmName, cfg: watchdogEnabled ? watchdog : null }),
    "Watchdog",
  );
  const savePanic = () => run(
    () => invoke("set_panic", { name: vmName, cfg: panicEnabled ? panicCfg : null }),
    "Panic",
  );
  const saveBalloon = () => run(
    () => invoke("set_balloon", {
      name: vmName, cfg: balloonEnabled ? balloon : null,
      live: isRunning, config: true,
    }),
    "Balloon",
  );
  const saveVsock = () => run(
    () => invoke("set_vsock", {
      name: vmName, cfg: vsockEnabled ? vsock : null,
      live: isRunning, config: true,
    }),
    "Vsock",
  );
  const saveIommu = () => run(
    () => invoke("set_iommu", { name: vmName, cfg: iommuEnabled ? iommu : null }),
    "IOMMU",
  );

  const addRng = () => run(
    () => invoke("add_rng", {
      name: vmName, cfg: newRng, live: isRunning, config: true,
    }),
    "Add RNG",
  );
  const removeRng = (r) => run(
    () => invoke("remove_rng", {
      name: vmName, cfg: r, live: isRunning, config: true,
    }),
    "Remove RNG",
  );

  function toggleSection(k) {
    openSection = { ...openSection, [k]: !openSection[k] };
  }
</script>

<div class="virtio">
  {#if loading}
    <p class="muted">Loading...</p>
  {:else if snap}
    {#if err}<div class="error">{err}</div>{/if}
    {#if isRunning}
      <div class="notice">VM is running — persistent-only changes take effect on next boot.</div>
    {/if}

    <!-- TPM -->
    <section>
      <button class="sect-head" onclick={() => toggleSection("tpm")}>
        <span class="chev">{openSection.tpm ? "▾" : "▸"}</span>
        <span class="sect-title">TPM</span>
        <span class="badge persistent">Persistent: restart required</span>
        {#if tpmEnabled}<span class="on-dot"></span>{/if}
      </button>
      {#if openSection.tpm}
        <div class="sect-body">
          <label class="toggle">
            <input type="checkbox" bind:checked={tpmEnabled} disabled={busy} />
            <span>Enable TPM</span>
          </label>
          {#if tpmEnabled && tpm}
            <div class="grid">
              <label><span>Model</span>
                <select bind:value={tpm.model} disabled={busy}>
                  {#each TPM_MODELS as m}<option value={m}>{m}</option>{/each}
                </select>
              </label>
              <label><span>Backend</span>
                <select bind:value={tpm.backend_model} disabled={busy}>
                  {#each TPM_BACKENDS as b}<option value={b}>{b}</option>{/each}
                </select>
              </label>
              {#if tpm.backend_model === "emulator"}
                <label><span>Version</span>
                  <select bind:value={tpm.backend_version} disabled={busy}>
                    {#each TPM_VERSIONS as v}<option value={v}>{v}</option>{/each}
                  </select>
                </label>
              {:else if tpm.backend_model === "passthrough"}
                <label><span>Device path</span>
                  <input bind:value={tpm.source_path} disabled={busy} placeholder="/dev/tpm0" />
                </label>
              {:else if tpm.backend_model === "external"}
                <label><span>Socket path</span>
                  <input bind:value={tpm.source_path} disabled={busy} placeholder="/var/run/tpm.sock" />
                </label>
              {/if}
            </div>
          {/if}
          <div class="actions"><button class="btn btn-primary" onclick={saveTpm} disabled={busy}>Save TPM</button></div>
        </div>
      {/if}
    </section>

    <!-- RNG -->
    <section>
      <button class="sect-head" onclick={() => toggleSection("rng")}>
        <span class="chev">{openSection.rng ? "▾" : "▸"}</span>
        <span class="sect-title">RNG</span>
        <span class="badge live">Live: hotplug</span>
        {#if snap.rngs.length > 0}<span class="count">{snap.rngs.length}</span>{/if}
      </button>
      {#if openSection.rng}
        <div class="sect-body">
          {#if snap.rngs.length === 0}
            <p class="muted">No RNG devices configured.</p>
          {:else}
            <ul class="dev-list">
              {#each snap.rngs as r, i (i)}
                <li>
                  <code>
                    {r.model} / {r.backend_model}
                    {#if r.source_path}({r.source_path}){/if}
                    {#if r.rate_period_ms}rate={r.rate_bytes}B/{r.rate_period_ms}ms{/if}
                  </code>
                  <button class="btn-tiny danger" onclick={() => removeRng(r)} disabled={busy}>Remove</button>
                </li>
              {/each}
            </ul>
          {/if}
          <div class="divider"></div>
          <div class="subhead">Add new RNG</div>
          <div class="grid">
            <label><span>Model</span>
              <select bind:value={newRng.model} disabled={busy}>
                {#each RNG_MODELS as m}<option value={m}>{m}</option>{/each}
              </select>
            </label>
            <label><span>Backend</span>
              <select bind:value={newRng.backend_model} disabled={busy}>
                {#each RNG_BACKENDS as b}<option value={b}>{b}</option>{/each}
              </select>
            </label>
            {#if newRng.backend_model !== "builtin"}
              <label><span>Source path</span>
                <input bind:value={newRng.source_path} disabled={busy} placeholder="/dev/urandom" />
              </label>
            {/if}
            <label><span>Rate bytes</span>
              <input type="number" bind:value={newRng.rate_bytes} disabled={busy} placeholder="(unlimited)" />
            </label>
            <label><span>Rate period ms</span>
              <input type="number" bind:value={newRng.rate_period_ms} disabled={busy} placeholder="(unlimited)" />
            </label>
          </div>
          <div class="actions"><button class="btn btn-primary" onclick={addRng} disabled={busy}>Add RNG</button></div>
        </div>
      {/if}
    </section>

    <!-- Watchdog -->
    <section>
      <button class="sect-head" onclick={() => toggleSection("watchdog")}>
        <span class="chev">{openSection.watchdog ? "▾" : "▸"}</span>
        <span class="sect-title">Watchdog</span>
        <span class="badge persistent">Persistent: restart required</span>
        {#if watchdogEnabled}<span class="on-dot"></span>{/if}
      </button>
      {#if openSection.watchdog}
        <div class="sect-body">
          <label class="toggle">
            <input type="checkbox" bind:checked={watchdogEnabled} disabled={busy} />
            <span>Enable watchdog</span>
          </label>
          {#if watchdogEnabled && watchdog}
            <div class="grid">
              <label><span>Model</span>
                <select bind:value={watchdog.model} disabled={busy}>
                  {#each WATCHDOG_MODELS as m}<option value={m}>{m}</option>{/each}
                </select>
              </label>
              <label><span>Action</span>
                <select bind:value={watchdog.action} disabled={busy}>
                  {#each WATCHDOG_ACTIONS as a}<option value={a}>{a}</option>{/each}
                </select>
              </label>
            </div>
          {/if}
          <div class="actions"><button class="btn btn-primary" onclick={saveWatchdog} disabled={busy}>Save Watchdog</button></div>
        </div>
      {/if}
    </section>

    <!-- Panic -->
    <section>
      <button class="sect-head" onclick={() => toggleSection("panic")}>
        <span class="chev">{openSection.panic ? "▾" : "▸"}</span>
        <span class="sect-title">Panic Notifier</span>
        <span class="badge persistent">Persistent: restart required</span>
        {#if panicEnabled}<span class="on-dot"></span>{/if}
      </button>
      {#if openSection.panic}
        <div class="sect-body">
          <label class="toggle">
            <input type="checkbox" bind:checked={panicEnabled} disabled={busy} />
            <span>Enable panic notifier</span>
          </label>
          {#if panicEnabled && panicCfg}
            <label><span>Model</span>
              <select bind:value={panicCfg.model} disabled={busy}>
                {#each PANIC_MODELS as m}<option value={m}>{m}</option>{/each}
              </select>
            </label>
          {/if}
          <div class="actions"><button class="btn btn-primary" onclick={savePanic} disabled={busy}>Save Panic</button></div>
        </div>
      {/if}
    </section>

    <!-- Balloon -->
    <section>
      <button class="sect-head" onclick={() => toggleSection("balloon")}>
        <span class="chev">{openSection.balloon ? "▾" : "▸"}</span>
        <span class="sect-title">Memory Balloon</span>
        <span class="badge mixed">Live: stats period / Persistent: model</span>
        {#if balloonEnabled}<span class="on-dot"></span>{/if}
      </button>
      {#if openSection.balloon}
        <div class="sect-body">
          <label class="toggle">
            <input type="checkbox" bind:checked={balloonEnabled} disabled={busy} />
            <span>Enable memballoon</span>
          </label>
          {#if balloonEnabled && balloon}
            <div class="grid">
              <label><span>Model</span>
                <select bind:value={balloon.model} disabled={busy}>
                  {#each BALLOON_MODELS as m}<option value={m}>{m}</option>{/each}
                </select>
              </label>
              <label><span>Stats period (seconds)</span>
                <input type="number" bind:value={balloon.stats_period_secs} disabled={busy} placeholder="(off)" />
              </label>
            </div>
            <div class="toggles-row">
              <label class="toggle">
                <input type="checkbox" bind:checked={balloon.autodeflate} disabled={busy || !balloon.model.startsWith("virtio")} />
                <span>autodeflate (requires virtio)</span>
              </label>
              <label class="toggle">
                <input type="checkbox" bind:checked={balloon.freepage_reporting} disabled={busy} />
                <span>free-page reporting</span>
              </label>
            </div>
          {/if}
          <div class="actions"><button class="btn btn-primary" onclick={saveBalloon} disabled={busy}>Save Balloon</button></div>
        </div>
      {/if}
    </section>

    <!-- Vsock -->
    <section>
      <button class="sect-head" onclick={() => toggleSection("vsock")}>
        <span class="chev">{openSection.vsock ? "▾" : "▸"}</span>
        <span class="sect-title">Vsock</span>
        <span class="badge live">Live: hotplug</span>
        {#if vsockEnabled}<span class="on-dot"></span>{/if}
      </button>
      {#if openSection.vsock}
        <div class="sect-body">
          <label class="toggle">
            <input type="checkbox" bind:checked={vsockEnabled} disabled={busy} />
            <span>Enable vsock</span>
          </label>
          {#if vsockEnabled && vsock}
            <div class="grid">
              <label><span>Model</span>
                <select bind:value={vsock.model} disabled={busy}>
                  {#each RNG_MODELS as m}<option value={m}>{m}</option>{/each}
                </select>
              </label>
              <label class="toggle">
                <input type="checkbox" bind:checked={vsock.auto_cid} disabled={busy} />
                <span>Auto-assign CID</span>
              </label>
              {#if !vsock.auto_cid}
                <label><span>CID (must be ≥ 3)</span>
                  <input type="number" min="3" bind:value={vsock.cid} disabled={busy} />
                </label>
              {/if}
            </div>
          {/if}
          <div class="actions"><button class="btn btn-primary" onclick={saveVsock} disabled={busy}>Save Vsock</button></div>
        </div>
      {/if}
    </section>

    <!-- IOMMU -->
    <section>
      <button class="sect-head" onclick={() => toggleSection("iommu")}>
        <span class="chev">{openSection.iommu ? "▾" : "▸"}</span>
        <span class="sect-title">IOMMU</span>
        <span class="badge persistent">Persistent: restart required</span>
        {#if iommuEnabled}<span class="on-dot"></span>{/if}
      </button>
      {#if openSection.iommu}
        <div class="sect-body">
          <label class="toggle">
            <input type="checkbox" bind:checked={iommuEnabled} disabled={busy} />
            <span>Enable IOMMU</span>
          </label>
          {#if iommuEnabled && iommu}
            <label><span>Model</span>
              <select bind:value={iommu.model} disabled={busy}>
                {#each IOMMU_MODELS as m}<option value={m}>{m}</option>{/each}
              </select>
            </label>
            <div class="toggles-row">
              <label class="toggle"><input type="checkbox" bind:checked={iommu.driver_intremap} disabled={busy} /><span>intremap</span></label>
              <label class="toggle"><input type="checkbox" bind:checked={iommu.driver_caching_mode} disabled={busy} /><span>caching_mode</span></label>
              <label class="toggle"><input type="checkbox" bind:checked={iommu.driver_eim} disabled={busy} /><span>eim</span></label>
              <label class="toggle"><input type="checkbox" bind:checked={iommu.driver_iotlb} disabled={busy} /><span>iotlb</span></label>
            </div>
          {/if}
          <div class="actions"><button class="btn btn-primary" onclick={saveIommu} disabled={busy}>Save IOMMU</button></div>
        </div>
      {/if}
    </section>
  {/if}
</div>

<style>
  .virtio { display: flex; flex-direction: column; gap: 12px; }
  .muted { color: var(--text-muted); font-size: 13px; }
  .error { padding: 8px 12px; background: rgba(239,68,68,0.1);
    border: 1px solid rgba(239,68,68,0.3); border-radius: 6px;
    color: #ef4444; font-size: 12px; }
  .notice { padding: 8px 12px; background: rgba(251,191,36,0.1);
    border: 1px solid rgba(251,191,36,0.3); border-radius: 6px;
    color: #fbbf24; font-size: 12px; }

  section { background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; }
  .sect-head { width: 100%; background: transparent; border: none; color: var(--text);
    padding: 12px 14px; display: flex; align-items: center; gap: 10px;
    cursor: pointer; font-family: inherit; text-align: left; }
  .sect-head:hover { background: var(--bg-hover); }
  .chev { color: var(--text-muted); font-size: 12px; width: 12px; }
  .sect-title { font-size: 14px; font-weight: 600; flex: 0 0 auto; min-width: 140px; }
  .badge { font-size: 10px; text-transform: uppercase; letter-spacing: 0.05em; padding: 2px 8px;
    border-radius: 10px; border: 1px solid var(--border); color: var(--text-muted); }
  .badge.live { border-color: #065f46; color: #34d399; }
  .badge.persistent { border-color: #78350f; color: #fbbf24; }
  .badge.mixed { border-color: #1e3a5f; color: #93c5fd; }
  .on-dot { width: 8px; height: 8px; border-radius: 50%; background: #34d399; margin-left: auto; }
  .count { margin-left: auto; font-size: 11px; padding: 2px 8px; border-radius: 10px;
    background: var(--bg-sidebar); color: var(--text-muted); }

  .sect-body { padding: 12px 14px 14px; border-top: 1px solid var(--border);
    display: flex; flex-direction: column; gap: 12px; }
  .subhead { font-size: 11px; font-weight: 600; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em; }
  .divider { height: 1px; background: var(--border); margin: 4px 0; }

  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 10px; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; }
  label > span { font-size: 11px; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em; }
  input:not([type="checkbox"]), select {
    padding: 6px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit;
    outline: none;
  }
  input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }
  .toggle { flex-direction: row; align-items: center; gap: 8px; font-size: 13px; cursor: pointer; }
  .toggle span { text-transform: none; letter-spacing: normal; color: var(--text); }
  .toggles-row { display: flex; flex-wrap: wrap; gap: 14px; }

  .dev-list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 6px; }
  .dev-list li { display: flex; align-items: center; justify-content: space-between;
    gap: 10px; padding: 6px 10px; background: var(--bg-sidebar); border-radius: 6px; font-size: 12px; }
  .dev-list code { font-family: 'SF Mono', monospace; }

  .btn-tiny { padding: 2px 8px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--bg-button); color: var(--text); font-size: 11px; cursor: pointer; font-family: inherit; }
  .btn-tiny.danger:hover { background: #7f1d1d; border-color: #7f1d1d; color: #fca5a5; }

  .actions { display: flex; gap: 8px; align-items: center; padding-top: 4px; }
  .btn { padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; cursor: pointer; font-family: inherit; }
  .btn-primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
