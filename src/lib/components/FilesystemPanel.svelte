<script>
  /*
   * Filesystem passthrough + shared memory editor.
   *
   * Two sections:
   *  1. Filesystem Passthrough - table + "Add share" dialog (virtiofs /
   *     9p). virtiofs entries require <memoryBacking mode='shared'/> on
   *     the domain; when missing we surface a warning and an enable
   *     button.
   *  2. Shared Memory - table + "Add shmem" dialog (ivshmem-plain /
   *     ivshmem-doorbell).
   *
   * All mutations are persistent (config=true, live=false). virtiofs
   * cannot be live-hot-plugged without shared memory backing, which
   * itself is persistent-only, so a restart is generally required.
   */
  import { invoke } from "@tauri-apps/api/core";

  let { vmName } = $props();

  let filesystems = $state([]);
  let shmems = $state([]);
  let domainXml = $state("");
  let hasSharedMemBacking = $state(false);
  let loading = $state(true);
  let err = $state(null);
  let busy = $state(false);

  // Add-filesystem dialog state
  let showAddFs = $state(false);
  let fsForm = $state(newFsForm());
  // Add-shmem dialog state
  let showAddShmem = $state(false);
  let shmemForm = $state(newShmemForm());

  function newFsForm() {
    return {
      driver_type: "virtiofs",
      source_dir: "",
      target_dir: "",
      accessmode: null,
      readonly: false,
      queue_size: null,
      xattr: false,
      posix_lock: false,
      flock: false,
    };
  }

  function newShmemForm() {
    return {
      name: "",
      size_mib: 64,
      model: "ivshmem-plain",
      role: "peer",
      server: "",
    };
  }

  async function reload() {
    loading = true; err = null;
    try {
      const [fs, sh, xml] = await Promise.all([
        invoke("list_filesystems", { name: vmName }),
        invoke("list_shmems", { name: vmName }),
        invoke("get_domain_xml", { name: vmName, inactive: true }),
      ]);
      filesystems = fs;
      shmems = sh;
      domainXml = xml;
      hasSharedMemBacking =
        /<memoryBacking>[\s\S]*?<access\s+mode=['"]shared['"]/i.test(xml);
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { if (vmName) reload(); });

  async function enableSharedMemoryBacking() {
    busy = true; err = null;
    try {
      await invoke("enable_shared_memory_backing", { name: vmName });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function submitAddFilesystem() {
    if (!fsForm.source_dir || !fsForm.target_dir) {
      err = "source_dir and target_dir are required";
      return;
    }
    busy = true; err = null;
    try {
      const fs = buildFsPayload(fsForm);
      const forceMb = fs.driver_type === "virtiofs" && !hasSharedMemBacking;
      await invoke("add_filesystem", {
        name: vmName,
        fs,
        forceMemoryBacking: forceMb,
        live: false,
        config: true,
      });
      showAddFs = false;
      fsForm = newFsForm();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  function buildFsPayload(f) {
    const virtiofs = f.driver_type === "virtiofs";
    return {
      driver_type: f.driver_type,
      source_dir: f.source_dir,
      target_dir: f.target_dir,
      accessmode: virtiofs ? null : f.accessmode,
      readonly: !!f.readonly,
      multidevs: null,
      queue_size: virtiofs && f.queue_size ? Number(f.queue_size) : null,
      xattr: virtiofs && !!f.xattr,
      posix_lock: virtiofs && !!f.posix_lock,
      flock: virtiofs && !!f.flock,
      binary_path: null,
    };
  }

  async function removeFilesystem(target_dir) {
    if (!confirm(`Remove filesystem share "${target_dir}"?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_filesystem", {
        name: vmName, targetDir: target_dir, live: false, config: true,
      });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function submitAddShmem() {
    if (!shmemForm.name) {
      err = "shmem name is required";
      return;
    }
    busy = true; err = null;
    try {
      const shmem = {
        name: shmemForm.name,
        size_bytes: Math.max(1, Number(shmemForm.size_mib)) * 1024 * 1024,
        model: shmemForm.model,
        role: shmemForm.role,
        server: shmemForm.model === "ivshmem-doorbell" && shmemForm.server
          ? shmemForm.server
          : null,
      };
      await invoke("add_shmem", {
        name: vmName, shmem, live: false, config: true,
      });
      showAddShmem = false;
      shmemForm = newShmemForm();
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  async function removeShmem(shName) {
    if (!confirm(`Remove shared memory "${shName}"?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_shmem", {
        name: vmName, shmemName: shName, live: false, config: true,
      });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      busy = false;
    }
  }

  function formatSize(bytes) {
    if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GiB`;
    if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(0)} MiB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KiB`;
    return `${bytes} B`;
  }
</script>

<div class="fs-panel">
  {#if loading}
    <p class="muted">Loading...</p>
  {:else}
    {#if err}
      <div class="error">{err}</div>
    {/if}

    <section>
      <div class="section-header">
        <h3>Filesystem Passthrough</h3>
        <button class="btn" onclick={() => { showAddFs = true; fsForm = newFsForm(); }}
                disabled={busy}>+ Add Share</button>
      </div>

      {#if !hasSharedMemBacking}
        <div class="warn">
          <div>
            <strong>virtiofs requires shared memory backing.</strong>
            Enable it on this domain to add virtiofs filesystems. Takes
            effect on next boot.
          </div>
          <button class="btn-small" onclick={enableSharedMemoryBacking} disabled={busy}>
            Enable shared memory backing
          </button>
        </div>
      {/if}

      {#if filesystems.length === 0}
        <p class="muted">No filesystem shares configured.</p>
      {:else}
        <table>
          <thead>
            <tr><th>Driver</th><th>Host path</th><th>Guest tag</th><th>Mode</th><th>Flags</th><th></th></tr>
          </thead>
          <tbody>
            {#each filesystems as f (f.target_dir)}
              <tr>
                <td><code>{f.driver_type}</code></td>
                <td><code>{f.source_dir}</code></td>
                <td><code>{f.target_dir}</code></td>
                <td>{f.accessmode ?? (f.driver_type === "virtiofs" ? "-" : "default")}</td>
                <td>
                  {#if f.readonly}<span class="chip">ro</span>{/if}
                  {#if f.queue_size}<span class="chip">q={f.queue_size}</span>{/if}
                  {#if f.xattr}<span class="chip">xattr</span>{/if}
                  {#if f.posix_lock}<span class="chip">posix_lock</span>{/if}
                  {#if f.flock}<span class="chip">flock</span>{/if}
                </td>
                <td>
                  <button class="btn-danger-small" onclick={() => removeFilesystem(f.target_dir)}
                          disabled={busy}>Remove</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section>
      <div class="section-header">
        <h3>Shared Memory</h3>
        <button class="btn" onclick={() => { showAddShmem = true; shmemForm = newShmemForm(); }}
                disabled={busy}>+ Add Shared Memory</button>
      </div>

      {#if shmems.length === 0}
        <p class="muted">No shared-memory devices configured.</p>
      {:else}
        <table>
          <thead>
            <tr><th>Name</th><th>Model</th><th>Role</th><th>Size</th><th>Server</th><th></th></tr>
          </thead>
          <tbody>
            {#each shmems as s (s.name)}
              <tr>
                <td><code>{s.name}</code></td>
                <td>{s.model}</td>
                <td>{s.role}</td>
                <td>{formatSize(s.size_bytes)}</td>
                <td>{s.server ? s.server : "-"}</td>
                <td>
                  <button class="btn-danger-small" onclick={() => removeShmem(s.name)}
                          disabled={busy}>Remove</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  {/if}
</div>

{#if showAddFs}
  <div class="modal-backdrop" onclick={() => (showAddFs = false)}>
    <div class="modal" onclick={(e) => e.stopPropagation()}>
      <h4>Add Filesystem Share</h4>

      <label>
        Driver
        <select bind:value={fsForm.driver_type}>
          <option value="virtiofs">virtiofs (recommended, Linux guest)</option>
          <option value="path">path (9p, legacy)</option>
          <option value="handle">handle (9p by fd)</option>
        </select>
      </label>

      <label>
        Host directory
        <input type="text" bind:value={fsForm.source_dir} placeholder="/srv/shared" />
      </label>

      <label>
        Guest mount tag
        <input type="text" bind:value={fsForm.target_dir} placeholder="shared" />
      </label>

      {#if fsForm.driver_type !== "virtiofs"}
        <label>
          Access mode
          <select bind:value={fsForm.accessmode}>
            <option value={null}>(default)</option>
            <option value="passthrough">passthrough</option>
            <option value="mapped">mapped</option>
            <option value="squash">squash</option>
          </select>
        </label>
      {/if}

      {#if fsForm.driver_type === "virtiofs"}
        <label>
          Queue size (optional)
          <input type="number" bind:value={fsForm.queue_size} placeholder="1024" min="0" />
        </label>
        <label class="check"><input type="checkbox" bind:checked={fsForm.xattr} /> xattr</label>
        <label class="check"><input type="checkbox" bind:checked={fsForm.posix_lock} /> posix lock</label>
        <label class="check"><input type="checkbox" bind:checked={fsForm.flock} /> flock</label>
      {/if}

      <label class="check"><input type="checkbox" bind:checked={fsForm.readonly} /> Read-only</label>

      {#if fsForm.driver_type === "virtiofs" && !hasSharedMemBacking}
        <div class="warn-inline">
          This domain has no shared memoryBacking. Adding will enable it
          persistently (restart required).
        </div>
      {/if}

      {#if err}<div class="error">{err}</div>{/if}

      <div class="modal-actions">
        <button class="btn" onclick={submitAddFilesystem} disabled={busy}>
          {busy ? "Adding..." : "Add"}
        </button>
        <button class="btn-ghost" onclick={() => (showAddFs = false)}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

{#if showAddShmem}
  <div class="modal-backdrop" onclick={() => (showAddShmem = false)}>
    <div class="modal" onclick={(e) => e.stopPropagation()}>
      <h4>Add Shared Memory</h4>

      <label>
        Name <input type="text" bind:value={shmemForm.name} placeholder="ivshmem" />
      </label>
      <label>
        Size (MiB)
        <input type="number" bind:value={shmemForm.size_mib} min="1" />
      </label>
      <label>
        Model
        <select bind:value={shmemForm.model}>
          <option value="ivshmem-plain">ivshmem-plain</option>
          <option value="ivshmem-doorbell">ivshmem-doorbell</option>
        </select>
      </label>
      <label>
        Role
        <select bind:value={shmemForm.role}>
          <option value="peer">peer</option>
          <option value="master">master</option>
        </select>
      </label>
      {#if shmemForm.model === "ivshmem-doorbell"}
        <label>
          Server socket path
          <input type="text" bind:value={shmemForm.server}
                 placeholder="/var/run/ivshmem.sock" />
        </label>
      {/if}

      {#if err}<div class="error">{err}</div>{/if}

      <div class="modal-actions">
        <button class="btn" onclick={submitAddShmem} disabled={busy}>
          {busy ? "Adding..." : "Add"}
        </button>
        <button class="btn-ghost" onclick={() => (showAddShmem = false)}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .fs-panel { padding: 1rem; display: flex; flex-direction: column; gap: 1.5rem; }
  section { display: flex; flex-direction: column; gap: 0.5rem; }
  .section-header {
    display: flex; justify-content: space-between; align-items: center;
  }
  h3 { margin: 0; font-size: 1rem; font-weight: 600; }
  h4 { margin: 0 0 0.5rem 0; }
  .muted { color: var(--muted, #888); font-style: italic; margin: 0; }
  .warn {
    background: #fff5d6; color: #7a5800;
    border: 1px solid #e5c979; border-radius: 4px;
    padding: 0.75rem; display: flex; justify-content: space-between;
    gap: 1rem; align-items: center;
  }
  .warn-inline {
    background: #fff5d6; color: #7a5800;
    border: 1px solid #e5c979; border-radius: 4px;
    padding: 0.5rem; font-size: 0.9em;
  }
  .error {
    background: #fde2e1; color: #7a1f1b;
    border: 1px solid #f5b5b1; border-radius: 4px; padding: 0.5rem;
  }
  table { width: 100%; border-collapse: collapse; font-size: 0.9em; }
  th, td {
    text-align: left; padding: 0.4rem 0.5rem;
    border-bottom: 1px solid var(--border, #e5e7eb);
  }
  code { font-family: ui-monospace, monospace; font-size: 0.9em; }
  .chip {
    display: inline-block; padding: 1px 6px; margin-right: 4px;
    background: var(--chip-bg, #eef); border-radius: 10px; font-size: 0.75em;
  }
  .btn, .btn-small, .btn-ghost, .btn-danger-small {
    border: 1px solid var(--border, #ccc); background: white;
    padding: 0.3rem 0.7rem; border-radius: 4px; cursor: pointer;
    font-size: 0.85em;
  }
  .btn:hover, .btn-small:hover { background: #f3f4f6; }
  .btn-danger-small { border-color: #f5b5b1; color: #7a1f1b; }
  .btn-danger-small:hover { background: #fde2e1; }
  .btn-ghost { border-color: transparent; }
  .modal-backdrop {
    position: fixed; inset: 0; background: rgba(0,0,0,0.4);
    display: flex; align-items: center; justify-content: center; z-index: 100;
  }
  .modal {
    background: white; border-radius: 8px; padding: 1.5rem;
    min-width: 380px; max-width: 600px; display: flex;
    flex-direction: column; gap: 0.7rem;
  }
  .modal label { display: flex; flex-direction: column; font-size: 0.9em; gap: 0.2rem; }
  .modal label.check { flex-direction: row; align-items: center; gap: 0.5rem; }
  .modal input, .modal select {
    padding: 0.3rem; border: 1px solid var(--border, #ccc); border-radius: 4px;
  }
  .modal-actions { display: flex; gap: 0.5rem; justify-content: flex-end; margin-top: 0.5rem; }
</style>
