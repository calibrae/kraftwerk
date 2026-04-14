<script>
  /*
   * Round F: Character-device editor.
   *
   * Sections: Serial / Console / Channel / Parallel.
   * Quick actions for the two common channels (qemu-ga, vdagent).
   */
  import { invoke } from "@tauri-apps/api/core";

  let { vmName } = $props();

  let snap = $state(null);
  let loading = $state(true);
  let err = $state(null);
  let busy = $state(false);

  // Add-channel dialog state.
  let showAddChannel = $state(false);
  let newChannel = $state({
    sourceKind: "unix",
    path: "",
    targetName: "",
    targetType: "virtio",
  });

  async function reload() {
    loading = true; err = null;
    try {
      snap = await invoke("get_char_devices", { name: vmName });
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { if (vmName) reload(); });

  function hasChannel(name) {
    return snap?.channels?.some((c) => c.target_name === name) ?? false;
  }

  async function addGuestAgent() {
    busy = true; err = null;
    try {
      await invoke("add_guest_agent_channel", { name: vmName, live: false, config: true });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally { busy = false; }
  }

  async function addVdagent() {
    busy = true; err = null;
    try {
      await invoke("add_spice_vdagent_channel", { name: vmName, live: false, config: true });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally { busy = false; }
  }

  async function removeChannel(targetName) {
    if (!confirm(`Remove channel ${targetName}?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_channel", {
        name: vmName,
        targetName,
        live: false,
        config: true,
      });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally { busy = false; }
  }

  async function removeSerial(port) {
    if (port == null) return;
    if (!confirm(`Remove serial port ${port}?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_serial", {
        name: vmName,
        port,
        live: false,
        config: true,
      });
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally { busy = false; }
  }

  async function submitCustomChannel() {
    busy = true; err = null;
    const k = newChannel.sourceKind;
    let source;
    if (k === "unix") {
      source = { kind: "unix", path: newChannel.path, mode: "bind" };
    } else if (k === "spicevmc") {
      source = { kind: "spicevmc" };
    } else if (k === "spiceport") {
      source = { kind: "spiceport", channel: newChannel.path };
    } else if (k === "dbus") {
      source = { kind: "dbus", channel: newChannel.path };
    } else if (k === "pty") {
      source = { kind: "pty" };
    } else {
      err = "Unsupported source kind";
      busy = false;
      return;
    }
    const channel = {
      source,
      target_type: newChannel.targetType,
      target_name: newChannel.targetName || null,
    };
    try {
      await invoke("add_channel", {
        name: vmName,
        channel,
        live: false,
        config: true,
      });
      showAddChannel = false;
      newChannel = { sourceKind: "unix", path: "", targetName: "", targetType: "virtio" };
      await reload();
    } catch (e) {
      err = e?.message || JSON.stringify(e);
    } finally { busy = false; }
  }

  function formatSource(src) {
    if (!src) return "";
    switch (src.kind) {
      case "pty": return "pty";
      case "dev": return `dev ${src.path}`;
      case "file": return `file ${src.path}${src.append ? " (append)" : ""}`;
      case "pipe": return `pipe ${src.path}`;
      case "tcp": return `tcp ${src.mode} ${src.host}:${src.port} (${src.protocol})`;
      case "udp": return `udp ${src.host}:${src.port}`;
      case "unix": return `unix ${src.mode}${src.path ? " " + src.path : ""}`;
      case "nmdm": return `nmdm ${src.master}/${src.slave}`;
      case "spicevmc": return "spicevmc";
      case "spiceport": return `spiceport ${src.channel}`;
      case "dbus": return `dbus ${src.channel}`;
      default: return JSON.stringify(src);
    }
  }
</script>

<div class="panel">
  {#if loading}
    <p>Loading character devices...</p>
  {:else if err}
    <div class="err">Error: {err}</div>
    <button onclick={reload}>Retry</button>
  {:else if snap}
    <div class="notice">Changes are persistent (applied on next boot).</div>

    <section>
      <h3>Channels</h3>
      <div class="presets">
        {#if !hasChannel("org.qemu.guest_agent.0")}
          <button onclick={addGuestAgent} disabled={busy}>+ Add qemu-guest-agent channel</button>
        {:else}
          <span class="chip ok">qemu-guest-agent configured</span>
        {/if}
        {#if !hasChannel("com.redhat.spice.0")}
          <button onclick={addVdagent} disabled={busy}>+ Add SPICE vdagent channel</button>
        {:else}
          <span class="chip ok">SPICE vdagent configured</span>
        {/if}
        <button onclick={() => showAddChannel = true} disabled={busy}>+ Custom channel...</button>
      </div>

      {#if snap.channels.length === 0}
        <p class="empty">No channels configured.</p>
      {:else}
        <table>
          <thead><tr><th>Target name</th><th>Type</th><th>Source</th><th></th></tr></thead>
          <tbody>
            {#each snap.channels as ch}
              <tr>
                <td><code>{ch.target_name ?? "(unnamed)"}</code></td>
                <td>{ch.target_type}</td>
                <td>{formatSource(ch.source)}</td>
                <td>
                  {#if ch.target_name}
                    <button class="danger" onclick={() => removeChannel(ch.target_name)} disabled={busy}>Remove</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section>
      <h3>Serial ports</h3>
      {#if snap.serials.length === 0}
        <p class="empty">No serial ports.</p>
      {:else}
        <table>
          <thead><tr><th>Port</th><th>Target type</th><th>Source</th><th></th></tr></thead>
          <tbody>
            {#each snap.serials as s}
              <tr>
                <td>{s.target_port ?? "-"}</td>
                <td>{s.target_type}</td>
                <td>{formatSource(s.source)}</td>
                <td>
                  {#if s.target_port != null}
                    <button class="danger" onclick={() => removeSerial(s.target_port)} disabled={busy}>Remove</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section>
      <h3>Consoles</h3>
      {#if snap.consoles.length === 0}
        <p class="empty">No consoles.</p>
      {:else}
        <table>
          <thead><tr><th>Port</th><th>Target type</th><th>Source</th></tr></thead>
          <tbody>
            {#each snap.consoles as c}
              <tr>
                <td>{c.target_port ?? "-"}</td>
                <td>{c.target_type}</td>
                <td>{formatSource(c.source)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <section>
      <h3>Parallel ports</h3>
      {#if snap.parallels.length === 0}
        <p class="empty">No parallel ports.</p>
      {:else}
        <table>
          <thead><tr><th>Port</th><th>Source</th></tr></thead>
          <tbody>
            {#each snap.parallels as p}
              <tr><td>{p.target_port ?? "-"}</td><td>{formatSource(p.source)}</td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  {/if}

  {#if showAddChannel}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="modal-backdrop" onclick={() => showAddChannel = false}>
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="modal" onclick={(e) => e.stopPropagation()}>
        <h3>Add custom channel</h3>
        <label>
          Source type:
          <select bind:value={newChannel.sourceKind}>
            <option value="unix">unix</option>
            <option value="spicevmc">spicevmc</option>
            <option value="spiceport">spiceport</option>
            <option value="dbus">dbus</option>
            <option value="pty">pty</option>
          </select>
        </label>
        {#if ["unix", "spiceport", "dbus"].includes(newChannel.sourceKind)}
          <label>
            {newChannel.sourceKind === "unix" ? "Path (blank = libvirt chooses)" : "Channel"}:
            <input type="text" bind:value={newChannel.path} />
          </label>
        {/if}
        <label>
          Target type:
          <select bind:value={newChannel.targetType}>
            <option value="virtio">virtio</option>
            <option value="guestfwd">guestfwd</option>
            <option value="xen">xen</option>
          </select>
        </label>
        <label>
          Target name:
          <input type="text" bind:value={newChannel.targetName} placeholder="com.example.channel.0" />
        </label>
        <div class="modal-actions">
          <button onclick={() => showAddChannel = false} disabled={busy}>Cancel</button>
          <button onclick={submitCustomChannel} disabled={busy}>Add</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .panel { padding: 12px; display: flex; flex-direction: column; gap: 16px; }
  .notice { font-size: 12px; color: #666; padding: 6px 10px; background: #f5f5f0; border-left: 3px solid #caa; border-radius: 2px; }
  .err { color: #a22; padding: 8px; background: #fdd; border-radius: 4px; margin-bottom: 8px; }
  section { border: 1px solid #ddd; border-radius: 4px; padding: 10px; background: #fafafa; }
  section h3 { margin: 0 0 8px 0; font-size: 14px; }
  .presets { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 8px; }
  .chip { padding: 4px 8px; border-radius: 10px; font-size: 12px; }
  .chip.ok { background: #dfd; color: #171; }
  .empty { color: #999; font-size: 12px; margin: 4px 0; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 4px 8px; border-bottom: 1px solid #eee; }
  th { background: #f0f0f0; }
  code { font-family: monospace; font-size: 12px; }
  button.danger { background: #fee; color: #a22; border: 1px solid #fbb; padding: 2px 8px; border-radius: 3px; cursor: pointer; }
  .modal-backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.4); display: flex; align-items: center; justify-content: center; z-index: 1000; }
  .modal { background: white; padding: 20px; border-radius: 6px; min-width: 320px; display: flex; flex-direction: column; gap: 10px; }
  .modal label { display: flex; flex-direction: column; font-size: 12px; gap: 3px; }
  .modal input, .modal select { padding: 4px; font-size: 13px; }
  .modal-actions { display: flex; justify-content: flex-end; gap: 6px; margin-top: 6px; }
</style>
