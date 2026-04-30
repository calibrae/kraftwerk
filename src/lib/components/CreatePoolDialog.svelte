<script>
  import { createPool } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false) } = $props();

  import { invoke } from "@tauri-apps/api/core";
  const TYPES = [
    { id: "dir",     label: "Directory",    desc: "Local filesystem folder (simplest, most common)" },
    { id: "netfs",   label: "NFS Mount",    desc: "Mount a remote NFS export" },
    { id: "logical", label: "LVM Volume Group", desc: "Existing LVM volume group on the host" },
    { id: "iscsi",   label: "iSCSI",        desc: "Remote iSCSI target" },
    { id: "rbd",     label: "Ceph / RBD",   desc: "RADOS Block Device pool on a Ceph cluster" },
    { id: "zfs",     label: "ZFS Dataset",  desc: "Existing ZFS pool/dataset on the hypervisor host" },
  ];

  let poolType = $state("dir");
  let name = $state("");
  let targetPath = $state("/var/lib/libvirt/images/my-pool");
  let sourceHost = $state("");
  let sourceDir = $state("");
  let sourceName = $state("");
  let startNow = $state(true);
  let autostart = $state(false);
  let busy = $state(false);
  let err = $state(null);

  // Auth section — surfaced for iSCSI (CHAP) and RBD (Ceph).
  let authEnabled = $state(false);
  let authUsername = $state("");
  let authPassphrase = $state("");
  let authSecretUuid = $state("");
  let authMode = $state("create"); // "create" | "existing"

  // Which extra fields to show per pool type
  let showSourceHost = $derived(poolType === "netfs" || poolType === "iscsi" || poolType === "rbd");
  let showSourceDir = $derived(poolType === "netfs" || poolType === "iscsi");
  let showSourceName = $derived(poolType === "logical" || poolType === "rbd" || poolType === "zfs");
  let showTargetPath = $derived(poolType !== "rbd" && poolType !== "zfs");
  let showAuth = $derived(poolType === "iscsi" || poolType === "rbd");
  let authType = $derived(poolType === "rbd" ? "ceph" : "chap");
  let secretUsageType = $derived(poolType === "rbd" ? "ceph" : "iscsi");
  let targetHelp = $derived(
    poolType === "dir" ? "Directory will be created if it doesn't exist." :
    poolType === "netfs" ? "Local mount point where the NFS share will be mounted." :
    poolType === "logical" ? "Device path (e.g. /dev/vg-name)." :
    poolType === "iscsi" ? "Local path for iSCSI block device." :
    ""
  );

  function reset() {
    poolType = "dir"; name = "";
    targetPath = "/var/lib/libvirt/images/my-pool";
    sourceHost = ""; sourceDir = ""; sourceName = "";
    authEnabled = false; authUsername = ""; authPassphrase = "";
    authSecretUuid = ""; authMode = "create";
    startNow = true; autostart = false;
    busy = false; err = null;
  }

  function close() { open = false; reset(); }

  async function submit(e) {
    e.preventDefault();
    if (!name.trim()) return;
    busy = true; err = null;
    try {
      let auth = null;
      if (showAuth && authEnabled) {
        if (!authUsername.trim()) {
          throw new Error("Auth username required");
        }
        let uuid = authSecretUuid.trim();
        if (authMode === "create") {
          if (!authPassphrase) throw new Error("Passphrase required to create a new secret");
          uuid = await invoke("define_secret", {
            req: {
              usage: secretUsageType,
              usage_id: secretUsageType === "iscsi"
                ? (sourceDir.trim() || null)
                : (authUsername.trim() || null),
              description: `${authType} auth for pool ${name.trim()}`,
              ephemeral: false,
              private: true,
              value: authPassphrase,
            },
          });
        }
        if (!uuid) throw new Error("Existing secret UUID required");
        auth = {
          auth_type: authType,
          username: authUsername.trim(),
          secret_uuid: uuid,
          secret_usage: secretUsageType === "iscsi"
            ? (sourceDir.trim() || null)
            : null,
        };
      }

      await createPool({
        name: name.trim(),
        pool_type: poolType,
        target_path: showTargetPath ? (targetPath.trim() || null) : null,
        source_host: showSourceHost ? (sourceHost.trim() || null) : null,
        source_dir: showSourceDir ? (sourceDir.trim() || null) : null,
        source_name: showSourceName ? (sourceName.trim() || null) : null,
        auth,
        build: true,
        start: startNow,
        autostart,
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
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="cp-title">
      <h3 id="cp-title">New Storage Pool</h3>

      <form onsubmit={submit}>
        <fieldset class="type-picker">
          <legend>Type</legend>
          <div class="type-grid">
            {#each TYPES as t}
              <label class="type-option" class:active={poolType === t.id}>
                <input type="radio" name="ptype" value={t.id} bind:group={poolType} />
                <div class="type-label">{t.label}</div>
                <div class="type-desc">{t.desc}</div>
              </label>
            {/each}
          </div>
        </fieldset>

        <label>
          <span>Name</span>
          <input bind:value={name} placeholder="my-pool" required />
        </label>

        {#if showTargetPath}
          <label>
            <span>Target Path</span>
            <input bind:value={targetPath} />
            <small class="hint">{targetHelp}</small>
          </label>
        {/if}

        {#if showSourceHost}
          <label>
            <span>Source Host</span>
            <input bind:value={sourceHost} placeholder="nas.local" />
          </label>
        {/if}

        {#if showSourceDir}
          <label>
            <span>{poolType === "iscsi" ? "Target IQN" : "Source Directory"}</span>
            <input bind:value={sourceDir} placeholder={poolType === "iscsi" ? "iqn.2023.example:target" : "/export/virt"} />
          </label>
        {/if}

        {#if showSourceName}
          <label>
            <span>
              {poolType === "rbd" ? "Ceph Pool Name"
                : poolType === "zfs" ? "ZFS Dataset (pool/path)"
                : "Volume Group Name"}
            </span>
            <input bind:value={sourceName} placeholder={
              poolType === "rbd" ? "libvirt-pool"
                : poolType === "zfs" ? "tank/libvirt"
                : "my-vg"
            } />
          </label>
        {/if}

        {#if showAuth}
          <fieldset>
            <legend>Authentication</legend>
            <label class="toggle">
              <input type="checkbox" bind:checked={authEnabled} />
              <span>Enable {poolType === "rbd" ? "Ceph (cephx)" : "CHAP"} authentication</span>
            </label>
            {#if authEnabled}
              <label>
                <span>Username</span>
                <input bind:value={authUsername} placeholder={poolType === "rbd" ? "libvirt" : "iscsi-user"} />
              </label>
              <div class="auth-mode">
                <label class="radio">
                  <input type="radio" name="authmode" value="create" bind:group={authMode} />
                  <span>Create new secret</span>
                </label>
                <label class="radio">
                  <input type="radio" name="authmode" value="existing" bind:group={authMode} />
                  <span>Use existing secret UUID</span>
                </label>
              </div>
              {#if authMode === "create"}
                <label>
                  <span>{poolType === "rbd" ? "CephX key" : "CHAP password"}</span>
                  <input type="password" bind:value={authPassphrase} placeholder="paste here" />
                </label>
                <small class="hint">A new libvirt secret will be created with this value, then referenced by the pool. The secret is private (not readable back via API).</small>
              {:else}
                <label>
                  <span>Secret UUID</span>
                  <input bind:value={authSecretUuid} placeholder="00000000-0000-0000-0000-000000000000" />
                </label>
              {/if}
            {/if}
          </fieldset>
        {/if}

        <div class="flags">
          <label class="toggle">
            <input type="checkbox" bind:checked={startNow} />
            <span>Start now</span>
          </label>
          <label class="toggle">
            <input type="checkbox" bind:checked={autostart} />
            <span>Autostart on boot</span>
          </label>
        </div>

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
    border-radius: 12px; padding: 24px; width: 520px; max-width: 100%;
    max-height: 90vh; overflow-y: auto; box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4); }
  h3 { margin: 0 0 16px; font-size: 16px; font-weight: 600; }
  form { display: flex; flex-direction: column; gap: 14px; }
  label { display: flex; flex-direction: column; gap: 4px; }
  label > span { font-size: 11px; font-weight: 500; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  small.hint { font-size: 11px; color: var(--text-muted); margin-top: 2px; }

  input[type="text"], input:not([type]) {
    padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit; outline: none;
  }
  input:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  fieldset { border: 1px solid var(--border); border-radius: 8px; padding: 12px 14px 14px;
    margin: 0; display: flex; flex-direction: column; gap: 10px; }
  legend { padding: 0 6px; font-size: 12px; color: var(--text-muted); font-weight: 500; }

  .type-picker { gap: 0; padding-bottom: 14px; }
  .type-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-top: 4px; }
  .type-option { border: 1px solid var(--border); border-radius: 8px; padding: 10px 12px;
    cursor: pointer; display: flex; flex-direction: column; gap: 2px; }
  .type-option:hover { background: var(--bg-hover); }
  .type-option.active { border-color: var(--accent); background: var(--accent-dim); }
  .type-option input { position: absolute; opacity: 0; pointer-events: none; }
  .type-label { font-size: 13px; font-weight: 600; color: var(--text); }
  .type-desc { font-size: 11px; color: var(--text-muted); line-height: 1.35; }

  .toggle { flex-direction: row; align-items: center; gap: 8px; cursor: pointer; }
  .toggle input { margin: 0; }
  .toggle span { text-transform: none; letter-spacing: normal; color: var(--text); font-size: 13px; font-weight: 400; }

  .flags { display: flex; gap: 20px; padding-top: 4px; }

  .auth-mode { display: flex; gap: 16px; padding: 4px 0; }
  .radio { flex-direction: row; align-items: center; gap: 6px; cursor: pointer; }
  .radio input { margin: 0; }
  .radio span { text-transform: none; letter-spacing: normal; color: var(--text); font-size: 12px; font-weight: 400; }
  input[type="password"] {
    padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit; outline: none;
  }
  input[type="password"]:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  .error { padding: 8px 12px; background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3); border-radius: 6px; color: #ef4444; font-size: 12px; }

  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .btn-cancel, .btn-primary { padding: 8px 16px; border-radius: 6px; font-size: 13px; font-family: inherit; cursor: pointer; }
  .btn-cancel { border: 1px solid var(--border); background: var(--bg-button); color: var(--text); }
  .btn-cancel:hover { background: var(--bg-hover); }
  .btn-primary { border: 1px solid var(--accent); background: var(--accent); color: white; }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled, .btn-cancel:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
