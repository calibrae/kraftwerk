<script>
  /*
   * Display / user-I/O editor (Round D).
   *
   * One form, four sections: Graphics, Video, Sound, Input.
   * All changes are persistent (apply to the saved config). Live
   * hotplug of display devices is unreliable, so we require a
   * restart for the new config to take effect — we surface a
   * warning when the VM is running.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getState } from "$lib/stores/app.svelte.js";

  let { vmName } = $props();
  const appState = getState();

  let cfg = $state(null);     // on-disk DisplayConfig
  let edit = $state(null);    // mutable copy
  let caps = $state(null);    // DomainCaps for pickers
  let loading = $state(true);
  let err = $state(null);
  let busy = $state(false);
  let lastSavedAt = $state(null);

  // Fallbacks when caps are missing.
  const FALLBACK_GRAPHICS = ["vnc", "spice", "rdp", "sdl", "dbus", "egl-headless", "none"];
  const FALLBACK_VIDEO = ["vga", "cirrus", "qxl", "virtio", "bochs", "ramfb", "vmvga", "none"];
  const SOUND_MODELS = ["ich9", "ich7", "ich6", "ac97", "hda", "es1370", "sb16", "usb"];
  const INPUT_TYPES = ["mouse", "keyboard", "tablet", "passthrough", "evdev"];
  const INPUT_BUSES = ["usb", "virtio", "ps2", "xen"];
  const CODEC_TYPES = ["duplex", "micro", "output"];
  const DEFAULT_MODES = ["any", "secure", "insecure"];

  async function reload() {
    loading = true; err = null;
    try {
      const [dc, caps_] = await Promise.all([
        invoke("get_display_config", { name: vmName }),
        invoke("get_domain_capabilities", {}).catch(() => null),
      ]);
      cfg = dc;
      edit = normalize(JSON.parse(JSON.stringify(dc)));
      caps = caps_;
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  // Ensure at least one graphics/video/sound entry in edit state so the
  // form has something to bind to.
  function normalize(d) {
    if (!d.graphics || d.graphics.length === 0) {
      d.graphics = [{ type: "vnc", autoport: true, listen: "127.0.0.1" }];
    }
    if (!d.video || d.video.length === 0) {
      d.video = [{ model: "virtio", heads: 1, primary: true }];
    }
    if (!d.sound || d.sound.length === 0) {
      d.sound = [{ model: "ich9", codecs: [] }];
    }
    if (!d.input) d.input = [];
    return d;
  }

  $effect(() => { if (vmName) reload(); });

  let dirty = $derived(() => {
    if (!cfg || !edit) return false;
    return JSON.stringify(cfg) !== JSON.stringify(edit);
  });

  // QXL needs SPICE graphics.
  let videoWarning = $derived(() => {
    if (!edit || !edit.video || !edit.video[0]) return null;
    if (edit.video[0].model === "qxl" && edit.graphics[0]?.type !== "spice") {
      return "QXL video is SPICE-only. Pick SPICE graphics or a different video model.";
    }
    if (edit.graphics[0]?.gl_accel && !edit.graphics[0]?.rendernode) {
      return "GL acceleration typically requires a rendernode path (e.g. /dev/dri/renderD128).";
    }
    return null;
  });

  let hasTablet = $derived(
    edit?.input?.some((i) => i.type === "tablet") ?? false
  );

  async function save() {
    if (!edit) return;
    busy = true; err = null;
    // Compute per-subsection patch: only send the subsection if it differs.
    const diff = (a, b) => JSON.stringify(a) !== JSON.stringify(b);
    const patch = {
      graphics: diff(cfg.graphics?.[0], edit.graphics?.[0]) ? edit.graphics[0] : null,
      video: diff(cfg.video?.[0], edit.video?.[0]) ? edit.video[0] : null,
      sound: diff(cfg.sound?.[0], edit.sound?.[0]) ? edit.sound[0] : null,
      inputs: diff(cfg.input, edit.input) ? edit.input : null,
    };
    try {
      await invoke("apply_display_patch", { name: vmName, patch });
      lastSavedAt = Date.now();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  function discard() {
    if (cfg) edit = normalize(JSON.parse(JSON.stringify(cfg)));
    err = null;
  }

  // Input list ops.
  function addInput() {
    edit.input = [...edit.input, { type: "tablet", bus: "usb" }];
  }
  function removeInput(i) {
    edit.input = edit.input.filter((_, idx) => idx !== i);
  }

  // Sound codec list ops.
  function addCodec() {
    const s = edit.sound[0];
    s.codecs = [...(s.codecs ?? []), { type: "duplex" }];
  }
  function removeCodec(i) {
    const s = edit.sound[0];
    s.codecs = s.codecs.filter((_, idx) => idx !== i);
  }

  let isRunning = $derived(appState.selectedVm?.state === "running");

  let graphicsTypes = $derived(
    caps?.devices?.graphics_types?.length ? caps.devices.graphics_types : FALLBACK_GRAPHICS
  );
  let videoModels = $derived(
    caps?.devices?.video_models?.length ? caps.devices.video_models : FALLBACK_VIDEO
  );
</script>

<div class="display">
  {#if loading}
    <p class="muted">Loading...</p>
  {:else if edit}
    {#if err}<div class="error">{err}</div>{/if}
    {#if isRunning && dirty()}
      <div class="notice">VM is running — changes take effect on next boot.</div>
    {/if}
    {#if videoWarning()}
      <div class="notice">{videoWarning()}</div>
    {/if}

    <!-- Graphics -->
    <section>
      <h3>Graphics</h3>
      <div class="grid">
        <label>
          <span>Type</span>
          <select bind:value={edit.graphics[0].type} disabled={busy}>
            {#each graphicsTypes as g}<option value={g}>{g}</option>{/each}
          </select>
        </label>
        <label>
          <span>Listen</span>
          <input bind:value={edit.graphics[0].listen} disabled={busy} placeholder="127.0.0.1" />
        </label>
        <label>
          <span>Port</span>
          <input type="number" bind:value={edit.graphics[0].port} disabled={busy || edit.graphics[0].autoport} placeholder="-1" />
        </label>
        <label class="toggle">
          <input type="checkbox" bind:checked={edit.graphics[0].autoport} disabled={busy} />
          <span>Autoport</span>
        </label>
        <label>
          <span>Keymap</span>
          <input bind:value={edit.graphics[0].keymap} disabled={busy} placeholder="en-us" />
        </label>
        {#if edit.graphics[0].type === "spice"}
          <label>
            <span>Default mode</span>
            <select bind:value={edit.graphics[0].default_mode} disabled={busy}>
              <option value={null}>(default)</option>
              {#each DEFAULT_MODES as m}<option value={m}>{m}</option>{/each}
            </select>
          </label>
        {/if}
        <label class="toggle">
          <input type="checkbox" bind:checked={edit.graphics[0].gl_accel} disabled={busy} />
          <span>GL acceleration</span>
        </label>
        {#if edit.graphics[0].gl_accel}
          <label>
            <span>Rendernode</span>
            <input bind:value={edit.graphics[0].rendernode} disabled={busy}
                   placeholder="/dev/dri/renderD128" />
          </label>
        {/if}
      </div>
    </section>

    <!-- Video -->
    <section>
      <h3>Video</h3>
      <div class="grid">
        <label>
          <span>Model</span>
          <select bind:value={edit.video[0].model} disabled={busy}>
            {#each videoModels as m}<option value={m}>{m}</option>{/each}
          </select>
        </label>
        <label>
          <span>VRAM (KiB)</span>
          <input type="number" bind:value={edit.video[0].vram} disabled={busy} />
        </label>
        <label>
          <span>Heads</span>
          <input type="number" min="1" bind:value={edit.video[0].heads} disabled={busy} />
        </label>
        <label class="toggle">
          <input type="checkbox" bind:checked={edit.video[0].primary} disabled={busy} />
          <span>Primary</span>
        </label>
        <label class="toggle">
          <input type="checkbox" bind:checked={edit.video[0].accel3d} disabled={busy} />
          <span>3D acceleration</span>
        </label>
      </div>
    </section>

    <!-- Sound -->
    <section>
      <h3>Sound</h3>
      <div class="grid">
        <label>
          <span>Model</span>
          <select bind:value={edit.sound[0].model} disabled={busy}>
            {#each SOUND_MODELS as m}<option value={m}>{m}</option>{/each}
          </select>
        </label>
      </div>
      <div class="list">
        <div class="list-header">
          <span class="list-title">Codecs</span>
          <button class="btn-tiny" onclick={addCodec} disabled={busy}>+ codec</button>
        </div>
        {#each edit.sound[0].codecs ?? [] as c, i (i)}
          <div class="row">
            <select bind:value={c.type} disabled={busy}>
              {#each CODEC_TYPES as t}<option value={t}>{t}</option>{/each}
            </select>
            <button class="btn-tiny danger" onclick={() => removeCodec(i)} disabled={busy}>×</button>
          </div>
        {:else}
          <div class="muted small">No codecs defined. Libvirt picks a default.</div>
        {/each}
      </div>
    </section>

    <!-- Input -->
    <section>
      <h3>Input</h3>
      <div class="list">
        <div class="list-header">
          <span class="list-title">Devices</span>
          <button class="btn-tiny" onclick={addInput} disabled={busy}>+ input</button>
        </div>
        {#each edit.input as inp, i (i)}
          <div class="row">
            <select bind:value={inp.type} disabled={busy}>
              {#each INPUT_TYPES as t}<option value={t}>{t}</option>{/each}
            </select>
            <select bind:value={inp.bus} disabled={busy}>
              <option value={null}>(default)</option>
              {#each INPUT_BUSES as b}<option value={b}>{b}</option>{/each}
            </select>
            <button class="btn-tiny danger" onclick={() => removeInput(i)} disabled={busy}>×</button>
          </div>
        {:else}
          <div class="muted small">No inputs defined. Libvirt adds a default PS/2 pair.</div>
        {/each}
        {#if !hasTablet}
          <div class="muted small">
            Tip: add a USB tablet for absolute positioning (required for SPICE
            client-side mouse mode).
          </div>
        {/if}
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
  .display { display: flex; flex-direction: column; gap: 16px; }
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

  .list { display: flex; flex-direction: column; gap: 6px; margin-top: 8px; }
  .list-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px; }
  .list-title { font-size: 11px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  .row { display: flex; align-items: center; gap: 8px; }

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
</style>
