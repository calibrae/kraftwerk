<script>
  /*
   * Boot / firmware / machine editor.
   *
   * All changes are persistent (next boot). We show a "restart required"
   * hint when any field differs from the saved value.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState } from "$lib/stores/app.svelte.js";

  let { vmName } = $props();
  const appState = getState();

  let cfg = $state(null);      // current on-disk BootConfig
  let edit = $state(null);     // mutable copy being edited
  let caps = $state(null);     // DomainCaps for pickers
  let loading = $state(true);
  let err = $state(null);
  let busy = $state(false);
  let lastSavedAt = $state(null);

  const POWEROFF_ACTIONS = ["destroy", "restart"];
  const REBOOT_ACTIONS = ["destroy", "restart"];
  const CRASH_ACTIONS = ["destroy", "restart", "preserve", "coredump-destroy", "coredump-restart"];
  const BOOT_DEV_CHOICES = ["hd", "cdrom", "network", "fd"];

  async function reload() {
    loading = true; err = null;
    try {
      const [bc, dc] = await Promise.all([
        invoke("get_boot_config", { name: vmName }),
        invoke("get_domain_capabilities", {}).catch(() => null),
      ]);
      cfg = bc; edit = JSON.parse(JSON.stringify(bc));
      caps = dc;
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { if (vmName) reload(); });

  let dirty = $derived(() => {
    if (!cfg || !edit) return false;
    return JSON.stringify(cfg) !== JSON.stringify(edit);
  });

  async function save() {
    if (!edit) return;
    if (edit.firmware !== cfg.firmware) {
      const confirmed = confirm(
        "WARNING: switching firmware between BIOS and EFI on an existing VM\n" +
        "almost always breaks the boot — the partition table layout, bootloader\n" +
        "(grub-bios vs grub-efi), and EFI System Partition are different.\n\n" +
        "Use this only on:\n" +
        "  • a fresh VM that has not been installed yet, or\n" +
        "  • a VM whose disk you are about to wipe and reinstall.\n\n" +
        "Continue?"
      );
      if (!confirmed) {
        edit.firmware = cfg.firmware;
        return;
      }
    }
    busy = true; err = null;
    const patch = {
      firmware: edit.firmware !== cfg.firmware ? edit.firmware : null,
      machine: edit.machine !== cfg.machine ? edit.machine : null,
      boot_order: JSON.stringify(edit.boot_order) !== JSON.stringify(cfg.boot_order)
        ? edit.boot_order : null,
      boot_menu_enabled: edit.boot_menu_enabled !== cfg.boot_menu_enabled ? edit.boot_menu_enabled : null,
      boot_menu_timeout_ms: edit.boot_menu_timeout_ms !== cfg.boot_menu_timeout_ms
        ? (edit.boot_menu_timeout_ms == null ? null : edit.boot_menu_timeout_ms)
        : null,
      secure_boot: edit.secure_boot !== cfg.secure_boot ? edit.secure_boot : null,
      on_poweroff: edit.on_poweroff !== cfg.on_poweroff ? edit.on_poweroff : null,
      on_reboot: edit.on_reboot !== cfg.on_reboot ? edit.on_reboot : null,
      on_crash: edit.on_crash !== cfg.on_crash ? edit.on_crash : null,
      features: JSON.stringify(edit.features) !== JSON.stringify(cfg.features)
        ? edit.features : null,
      cpu_mode: edit.cpu_mode !== cfg.cpu_mode ? edit.cpu_mode : null,
    };
    try {
      await invoke("apply_boot_patch", { name: vmName, patch });
      lastSavedAt = Date.now();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  function discard() {
    if (cfg) edit = JSON.parse(JSON.stringify(cfg));
    err = null;
  }

  function moveBootDev(i, dir) {
    if (!edit) return;
    const j = i + dir;
    if (j < 0 || j >= edit.boot_order.length) return;
    const next = [...edit.boot_order];
    [next[i], next[j]] = [next[j], next[i]];
    edit.boot_order = next;
  }
  function removeBootDev(i) {
    if (!edit) return;
    edit.boot_order = edit.boot_order.filter((_, idx) => idx !== i);
  }
  function addBootDev(d) {
    if (!edit || edit.boot_order.includes(d)) return;
    edit.boot_order = [...edit.boot_order, d];
  }

  let isRunning = $derived(appState.selectedVm?.state === "running");

  let machineOptions = $derived(caps?.devices?.disk_buses ? [] : []); // not available in caps yet
  let firmwareOptions = $derived(["bios", "efi"]);
  let cpuModeOptions = $derived(caps?.cpu?.modes_supported ?? ["host-passthrough", "host-model", "custom"]);
</script>

<div class="boot">
  {#if loading}
    <p class="muted">Loading...</p>
  {:else if edit}
    {#if err}<div class="error">{err}</div>{/if}
    {#if isRunning && dirty()}
      <div class="notice">VM is running — changes take effect on next boot.</div>
    {/if}

    <section>
      <h3>Firmware</h3>
      <div class="grid">
        <label>
          <span>Firmware</span>
          <select bind:value={edit.firmware} disabled={busy}>
            {#each firmwareOptions as f}<option value={f}>{f.toUpperCase()}</option>{/each}
          </select>
          {#if cfg && edit.firmware !== cfg.firmware}
            <small class="hint warn">
              Changing firmware breaks an installed VM. Only do this on a fresh
              install or one you are about to wipe.
            </small>
          {/if}
        </label>
        {#if edit.firmware === "efi"}
          <label class="toggle">
            <input type="checkbox" bind:checked={edit.secure_boot} disabled={busy} />
            <span>Secure Boot</span>
          </label>
        {/if}
        <label>
          <span>Machine Type</span>
          <input bind:value={edit.machine} disabled={busy} placeholder="q35" />
        </label>
      </div>
    </section>

    <section>
      <h3>Boot Order</h3>
      <ol class="boot-list">
        {#each edit.boot_order as dev, i (i + dev)}
          <li>
            <span class="boot-dev">{dev}</span>
            <button class="btn-tiny" onclick={() => moveBootDev(i, -1)} disabled={busy || i === 0}>↑</button>
            <button class="btn-tiny" onclick={() => moveBootDev(i, 1)} disabled={busy || i === edit.boot_order.length - 1}>↓</button>
            <button class="btn-tiny danger" onclick={() => removeBootDev(i)} disabled={busy}>×</button>
          </li>
        {:else}
          <li class="muted">No boot devices configured.</li>
        {/each}
      </ol>
      <div class="boot-add">
        {#each BOOT_DEV_CHOICES as d}
          <button
            class="btn-tiny"
            onclick={() => addBootDev(d)}
            disabled={busy || edit.boot_order.includes(d)}
          >+ {d}</button>
        {/each}
      </div>
      <label class="toggle">
        <input type="checkbox" bind:checked={edit.boot_menu_enabled} disabled={busy} />
        <span>Show boot menu on startup</span>
      </label>
      {#if edit.boot_menu_enabled}
        <label>
          <span>Menu timeout (ms)</span>
          <input type="number" min="0" step="500" bind:value={edit.boot_menu_timeout_ms} disabled={busy} />
        </label>
      {/if}
    </section>

    <section>
      <h3>CPU Mode</h3>
      <label>
        <span>Mode</span>
        <select bind:value={edit.cpu_mode} disabled={busy}>
          <option value={null}>(default)</option>
          {#each cpuModeOptions as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>
    </section>

    <section>
      <h3>Features</h3>
      <div class="feats">
        <label class="toggle"><input type="checkbox" bind:checked={edit.features.acpi} disabled={busy} /><span>ACPI</span></label>
        <label class="toggle"><input type="checkbox" bind:checked={edit.features.apic} disabled={busy} /><span>APIC</span></label>
        <label class="toggle"><input type="checkbox" bind:checked={edit.features.pae} disabled={busy} /><span>PAE</span></label>
        <label class="toggle"><input type="checkbox" bind:checked={edit.features.smm} disabled={busy || edit.firmware === "efi"} /><span>SMM {#if edit.firmware === "efi"}(forced on)*{/if}</span></label>
        <label class="toggle"><input type="checkbox" bind:checked={edit.features.hap} disabled={busy} /><span>HAP</span></label>
      </div>
    </section>

    <section>
      <h3>Events</h3>
      <div class="grid">
        <label>
          <span>On Power Off</span>
          <select bind:value={edit.on_poweroff} disabled={busy}>
            <option value={null}>(default)</option>
            {#each POWEROFF_ACTIONS as a}<option value={a}>{a}</option>{/each}
          </select>
        </label>
        <label>
          <span>On Reboot</span>
          <select bind:value={edit.on_reboot} disabled={busy}>
            <option value={null}>(default)</option>
            {#each REBOOT_ACTIONS as a}<option value={a}>{a}</option>{/each}
          </select>
        </label>
        <label>
          <span>On Crash</span>
          <select bind:value={edit.on_crash} disabled={busy}>
            <option value={null}>(default)</option>
            {#each CRASH_ACTIONS as a}<option value={a}>{a}</option>{/each}
          </select>
        </label>
      </div>
    </section>

    <div class="actions">
      <button class="btn" onclick={discard} disabled={busy || !dirty()}>Discard</button>
      <button class="btn btn-primary" onclick={save} disabled={busy || !dirty()}>
        {busy ? "Saving..." : "Save"}
      </button>
      {#if lastSavedAt && !dirty()}
        <span class="saved-note">Saved.</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .boot { display: flex; flex-direction: column; gap: 16px; }
  .muted { color: var(--text-muted); font-size: 13px; }
  .error { padding: 8px 12px; background: rgba(239,68,68,0.1);
    border: 1px solid rgba(239,68,68,0.3); border-radius: 6px;
    color: #ef4444; font-size: 12px; }
  .notice { padding: 8px 12px; background: rgba(251,191,36,0.1);
    border: 1px solid rgba(251,191,36,0.3); border-radius: 6px;
    color: #fbbf24; font-size: 12px; }

  section { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 8px; padding: 14px; }
  h3 { margin: 0 0 10px; font-size: 11px; font-weight: 600; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em; }

  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 12px; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; }
  label > span { font-size: 11px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  input[type="text"], input:not([type]), input[type="number"], select {
    padding: 6px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit;
    outline: none;
  }
  input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  .toggle { flex-direction: row; align-items: center; gap: 8px; font-size: 13px; cursor: pointer; }
  .toggle span { text-transform: none; letter-spacing: normal; color: var(--text); }
  .feats { display: flex; flex-wrap: wrap; gap: 14px; }

  .boot-list { list-style: decimal; padding-left: 24px; margin: 0 0 10px; display: flex; flex-direction: column; gap: 6px; font-size: 13px; }
  .boot-list li { display: flex; align-items: center; gap: 8px; }
  .boot-dev { font-family: 'SF Mono', monospace; background: var(--bg-sidebar);
    padding: 2px 8px; border-radius: 4px; font-size: 12px; }
  .boot-add { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 10px; }

  .btn-tiny { padding: 2px 8px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--bg-button); color: var(--text); font-size: 11px; cursor: pointer; font-family: inherit; }
  .btn-tiny:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-tiny:disabled { opacity: 0.35; cursor: not-allowed; }
  .btn-tiny.danger:hover { background: #7f1d1d; border-color: #7f1d1d; color: #fca5a5; }

  .actions { display: flex; gap: 8px; align-items: center; padding-top: 4px; }
  .btn { padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; cursor: pointer; font-family: inherit; }
  .btn:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .saved-note { color: #34d399; font-size: 12px; margin-left: 8px; }
  .hint.warn { color: #fbbf24; font-size: 11px; }
</style>
