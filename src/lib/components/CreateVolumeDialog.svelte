<script>
  import { invoke } from "@tauri-apps/api/core";
  import { createVolume, getState } from "$lib/stores/app.svelte.js";

  const appState = getState();

  let { open = $bindable(false), poolName = "" } = $props();

  let name = $state("disk.qcow2");
  let capacityGb = $state(20);
  let format = $state("qcow2");
  let thinProvision = $state(true);
  let busy = $state(false);
  let err = $state(null);

  // LUKS state — only available for qcow2 (raw can be LUKS too at the
  // device layer, but we keep the v1 surface simple).
  let encryptLuks = $state(false);
  let passphrase = $state("");
  let showPass = $state(false);

  function reset() {
    name = "disk.qcow2"; capacityGb = 20;
    format = "qcow2"; thinProvision = true;
    encryptLuks = false; passphrase = ""; showPass = false;
    busy = false; err = null;
  }
  function close() { open = false; reset(); }

  // Auto-adjust extension when format changes
  $effect(() => {
    const base = name.replace(/\.(qcow2|raw|iso|img)$/i, "");
    if (format === "qcow2" && !name.endsWith(".qcow2")) name = `${base}.qcow2`;
    else if (format === "raw" && !name.endsWith(".raw") && !name.endsWith(".img")) name = `${base}.raw`;
    else if (format === "iso" && !name.endsWith(".iso")) name = `${base}.iso`;
  });

  async function submit(e) {
    e.preventDefault();
    if (!name.trim() || !poolName || capacityGb <= 0) return;
    if (encryptLuks && !passphrase) {
      err = "LUKS passphrase required";
      return;
    }
    busy = true; err = null;
    try {
      const bytes = Math.floor(capacityGb * 1024 * 1024 * 1024);
      let luksUuid = null;
      if (encryptLuks) {
        // Predict the path the new volume will take so the secret's
        // usage_id matches what libvirt computes.
        const pool = (appState.pools ?? []).find(p => p.name === poolName);
        const targetPath = pool?.target_path
          ? `${pool.target_path.replace(/\/$/, "")}/${name.trim()}`
          : null;
        luksUuid = await invoke("define_secret", {
          req: {
            usage: targetPath ? "volume" : "none",
            usage_id: targetPath,
            description: `LUKS for ${name.trim()}`,
            ephemeral: false,
            private: true,
            value: passphrase,
          },
        });
      }
      await createVolume({
        pool_name: poolName,
        name: name.trim(),
        capacity_bytes: bytes,
        format,
        allocation_bytes: thinProvision ? null : bytes,
        luks_secret_uuid: luksUuid,
      });
      close();
    } catch (ex) {
      err = ex?.message || String(ex);
      busy = false;
    }
  }
</script>

{#if open}
  <div class="backdrop" onclick={close} role="presentation">
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="cv-title">
      <h3 id="cv-title">New Volume in "{poolName}"</h3>

      <form onsubmit={submit}>
        <label>
          <span>Name</span>
          <input bind:value={name} required />
        </label>

        <label>
          <span>Format</span>
          <select bind:value={format}>
            <option value="qcow2">qcow2 (recommended — thin + snapshots)</option>
            <option value="raw">raw (maximum performance)</option>
            <option value="iso">iso (CD/DVD image)</option>
          </select>
        </label>

        <label>
          <span>Capacity (GB)</span>
          <input type="number" min="0.1" step="0.1" bind:value={capacityGb} required />
        </label>

        {#if format !== "iso"}
          <label class="toggle">
            <input type="checkbox" bind:checked={thinProvision} />
            <span>Thin-provisioned (allocate on demand)</span>
          </label>
        {/if}

        {#if format === "qcow2"}
          <label class="toggle">
            <input type="checkbox" bind:checked={encryptLuks} />
            <span>Encrypt with LUKS</span>
          </label>
          {#if encryptLuks}
            <label>
              <span>Passphrase</span>
              <div class="row">
                <input type={showPass ? "text" : "password"} bind:value={passphrase} autocomplete="new-password" />
                <button type="button" class="btn-link" onclick={() => showPass = !showPass}>
                  {showPass ? "hide" : "show"}
                </button>
              </div>
            </label>
            <p class="hint">
              The passphrase is stored as a libvirt secret marked
              <code>private</code> — it can't be read back via API. If you
              lose it the volume's data is unrecoverable. Use a password
              manager.
            </p>
          {/if}
        {/if}

        {#if err}<div class="error">{err}</div>{/if}

        <div class="actions">
          <button type="button" class="btn-cancel" onclick={close} disabled={busy}>Cancel</button>
          <button type="submit" class="btn-primary" disabled={busy || !name.trim()}>
            {busy ? "Creating..." : "Create"}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.55);
    display: flex; align-items: center; justify-content: center; z-index: 100; padding: 20px; }
  .dialog { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 12px; padding: 24px; width: 440px; max-width: 100%;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4); }
  h3 { margin: 0 0 16px; font-size: 16px; font-weight: 600; }
  form { display: flex; flex-direction: column; gap: 14px; }
  label { display: flex; flex-direction: column; gap: 4px; }
  label > span { font-size: 11px; font-weight: 500; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }

  input[type="text"], input:not([type]), input[type="number"], select {
    padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit; outline: none;
  }
  input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  .toggle { flex-direction: row; align-items: center; gap: 8px; cursor: pointer; }
  .toggle input { margin: 0; }
  .toggle span { text-transform: none; letter-spacing: normal; color: var(--text); font-size: 13px; font-weight: 400; }

  .error { padding: 8px 12px; background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3); border-radius: 6px; color: #ef4444; font-size: 12px; }

  .row { display: flex; gap: 6px; align-items: stretch; }
  .row input { flex: 1; }
  .btn-link {
    border: 1px solid var(--border);
    background: var(--bg-button); color: var(--text-muted);
    padding: 0 10px; border-radius: 6px; font-size: 11px;
    font-family: inherit; cursor: pointer;
  }
  .btn-link:hover { color: var(--text); background: var(--bg-hover); }

  .hint { font-size: 11px; color: var(--text-muted); margin: -6px 0 0; }
  .hint code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; background: var(--bg-input); padding: 1px 4px; border-radius: 3px; }

  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .btn-cancel, .btn-primary { padding: 8px 16px; border-radius: 6px; font-size: 13px; font-family: inherit; cursor: pointer; }
  .btn-cancel { border: 1px solid var(--border); background: var(--bg-button); color: var(--text); }
  .btn-cancel:hover { background: var(--bg-hover); }
  .btn-primary { border: 1px solid var(--accent); background: var(--accent); color: white; }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled, .btn-cancel:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
