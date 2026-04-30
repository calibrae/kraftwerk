<script>
  /*
   * Clone-from-template dialog. Builds on the regular clone path
   * (full-copy of disks via virStorageVolCreateXMLFrom) and adds an
   * optional cloud-init NoCloud seed ISO that's generated on the
   * hypervisor host and attached as a CD-ROM on the new VM.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { refreshVms } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false), source = null } = $props();

  let targetName = $state("");
  let startAfter = $state(true);

  // Cloud-init form
  let useCloudInit = $state(true);
  let hostname = $state("");
  let username = $state("cali");
  let sshKeys = $state("");   // newline-separated public keys
  let passwordHash = $state("");
  let runcmd = $state("");    // newline-separated commands
  let packages = $state("");  // space- or newline-separated package names

  let busy = $state(false);
  let err = $state(null);

  $effect(() => {
    if (open && source) {
      targetName = `${source.name}-instance`;
      hostname = `${source.name}-instance`;
      err = null;
      busy = false;
    }
  });

  function close() { open = false; err = null; }

  async function doClone(e) {
    e.preventDefault();
    if (!targetName.trim() || busy) return;
    busy = true;
    err = null;

    const cloudInit = useCloudInit
      ? {
          hostname: hostname.trim() || null,
          username: username.trim() || null,
          ssh_authorized_keys: sshKeys.split(/\r?\n/).map(s => s.trim()).filter(Boolean),
          password_hash: passwordHash.trim() || null,
          runcmd: runcmd.split(/\r?\n/).map(s => s.trim()).filter(Boolean),
          packages: packages.split(/[\s\n]+/).map(s => s.trim()).filter(Boolean),
          network_config: null,
        }
      : null;

    try {
      await invoke("clone_from_template", {
        templateName: source.name,
        options: {
          target_name: targetName.trim(),
          randomize_macs: true,
          start_after: startAfter,
        },
        cloudInit,
      });
      await refreshVms();
      close();
    } catch (ex) {
      err = ex?.message || String(ex);
      busy = false;
    }
  }
</script>

{#if open && source}
<div class="backdrop" onclick={close} role="presentation">
  <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
    <header>
      <h3>Instantiate from template · {source.name}</h3>
      <button class="x" onclick={close} disabled={busy}>×</button>
    </header>
    <form onsubmit={doClone}>
      <label>
        <span>New VM name</span>
        <input type="text" bind:value={targetName} required autofocus />
      </label>

      <fieldset>
        <legend>cloud-init seed</legend>
        <label class="cb">
          <input type="checkbox" bind:checked={useCloudInit} />
          <span>Generate a NoCloud seed ISO and attach it</span>
        </label>
        {#if useCloudInit}
          <label>
            <span>Hostname (also instance fqdn)</span>
            <input type="text" bind:value={hostname} />
          </label>
          <label>
            <span>Default user</span>
            <input type="text" bind:value={username} placeholder="cali" />
          </label>
          <label>
            <span>SSH authorized_keys (one per line)</span>
            <textarea rows="3" bind:value={sshKeys} placeholder="ssh-ed25519 AAAA… you@host"></textarea>
          </label>
          <label>
            <span>Password hash (optional, e.g. <code>mkpasswd -m sha-512</code>)</span>
            <input type="text" bind:value={passwordHash} placeholder="$6$..." />
          </label>
          <label>
            <span>Packages to install (whitespace-separated)</span>
            <input type="text" bind:value={packages} placeholder="htop tmux vim" />
          </label>
          <label>
            <span>runcmd (one shell command per line)</span>
            <textarea rows="2" bind:value={runcmd} placeholder="systemctl enable sshd"></textarea>
          </label>
        {/if}
      </fieldset>

      <label class="cb">
        <input type="checkbox" bind:checked={startAfter} />
        <span>Start the new VM after creation</span>
      </label>

      <p class="hint">
        Source must be shut off; each writable disk is full-copy
        duplicated in its own pool. The cloud-init ISO is built on
        the hypervisor host (needs <code>genisoimage</code>,
        <code>xorrisofs</code>, or <code>mkisofs</code>) and attached
        as a CD-ROM. The guest image must already have cloud-init
        installed for the seed to take effect.
      </p>

      {#if err}<pre class="error">{err}</pre>{/if}

      <div class="actions">
        <button type="button" class="btn" onclick={close} disabled={busy}>Cancel</button>
        <button type="submit" class="btn primary" disabled={busy || !targetName.trim()}>
          {busy ? "Instantiating…" : "Instantiate"}
        </button>
      </div>
    </form>
  </div>
</div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.6);
    display: flex; align-items: center; justify-content: center; z-index: 200; padding: 20px; }
  .dialog { background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 12px; width: 560px; max-width: 100%; max-height: 90vh;
    display: flex; flex-direction: column; box-shadow: 0 12px 40px rgba(0,0,0,0.5); overflow: hidden; }
  header { padding: 14px 18px; border-bottom: 1px solid var(--border);
    display: flex; align-items: center; justify-content: space-between; }
  header h3 { margin: 0; font-size: 14px; }
  .x { background: none; border: none; color: var(--text-muted); font-size: 22px; cursor: pointer; }
  form { padding: 16px 18px; display: flex; flex-direction: column; gap: 12px; overflow-y: auto; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; }
  label > span { color: var(--text-muted); font-size: 11px;
    text-transform: uppercase; letter-spacing: 0.05em; }
  label.cb { flex-direction: row; align-items: center; gap: 8px; }
  label.cb span { text-transform: none; letter-spacing: 0; color: var(--text); font-size: 13px; }
  input[type=text], textarea {
    background: var(--bg-button); color: var(--text); border: 1px solid var(--border);
    border-radius: 4px; padding: 6px 8px; font-family: inherit; font-size: 13px;
  }
  textarea { font-family: 'SF Mono', monospace; font-size: 12px; resize: vertical; }
  fieldset { border: 1px solid var(--border); border-radius: 6px; padding: 10px 12px;
    display: flex; flex-direction: column; gap: 8px; }
  fieldset legend { padding: 0 6px; font-size: 11px; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em; }
  .hint { font-size: 11px; color: var(--text-muted); margin: 0; line-height: 1.5; }
  .hint code { background: rgba(0,0,0,0.3); padding: 1px 4px; border-radius: 3px; font-size: 10px; }
  .error { margin: 0; padding: 8px 10px; background: rgba(239,68,68,0.1);
    border: 1px solid rgba(239,68,68,0.3); color: #ef4444; font-size: 12px;
    border-radius: 4px; white-space: pre-wrap; max-height: 160px; overflow: auto; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; padding-top: 4px; }
  .btn { padding: 7px 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; cursor: pointer; font-family: inherit; }
  .btn.primary { background: var(--accent); border-color: var(--accent); color: white; }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
