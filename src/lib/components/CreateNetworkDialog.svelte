<script>
  import { invoke } from "@tauri-apps/api/core";
  import { refreshNetworks } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false) } = $props();

  // Modes — each has its own field set
  const MODES = [
    { id: "nat",      label: "NAT",      desc: "Outbound only, private subnet behind NAT (most common)" },
    { id: "route",    label: "Routed",   desc: "Forwarded via kernel, no NAT — host must route" },
    { id: "open",     label: "Open",     desc: "Like NAT but no netfilter rules (manual firewall)" },
    { id: "isolated", label: "Isolated", desc: "VM-to-VM only, no host or internet access" },
    { id: "bridge",   label: "Bridge",   desc: "Attach to an existing host bridge (L2 direct)" },
  ];

  // ── form state
  let mode = $state("nat");
  let name = $state("");
  let bridgeName = $state("virbr100");
  let forwardDev = $state("");
  let domainName = $state("");

  let ipv4Enabled = $state(true);
  let ipv4Address = $state("192.168.100.1");
  let ipv4Netmask = $state("255.255.255.0");
  let ipv4Dhcp = $state(true);
  let ipv4DhcpStart = $state("192.168.100.100");
  let ipv4DhcpEnd = $state("192.168.100.200");

  let ipv6Enabled = $state(false);
  let ipv6Address = $state("fd00::1");
  let ipv6Prefix = $state(64);
  let ipv6Dhcp = $state(false);
  let ipv6DhcpStart = $state("fd00::100");
  let ipv6DhcpEnd = $state("fd00::1ff");

  let autostart = $state(false);
  let startNow = $state(true);

  let busy = $state(false);
  let err = $state(null);

  function reset() {
    mode = "nat";
    name = "";
    bridgeName = "virbr100";
    forwardDev = "";
    domainName = "";
    ipv4Enabled = true;
    ipv4Address = "192.168.100.1";
    ipv4Netmask = "255.255.255.0";
    ipv4Dhcp = true;
    ipv4DhcpStart = "192.168.100.100";
    ipv4DhcpEnd = "192.168.100.200";
    ipv6Enabled = false;
    ipv6Address = "fd00::1";
    ipv6Prefix = 64;
    ipv6Dhcp = false;
    ipv6DhcpStart = "fd00::100";
    ipv6DhcpEnd = "fd00::1ff";
    autostart = false;
    startNow = true;
    err = null;
    busy = false;
  }

  function close() { open = false; reset(); }

  // Derived: which field groups are relevant for the chosen mode
  let showBridgePicker = $derived(mode === "bridge");
  let showIpConfig = $derived(mode !== "bridge");
  let showForwardDev = $derived(mode === "route");
  let bridgeLabel = $derived(mode === "bridge" ? "Host Bridge Name" : "Virtual Bridge Name");
  let bridgeHint = $derived(
    mode === "bridge"
      ? "Must be an existing bridge on the host (e.g. br0)."
      : "Libvirt will create this bridge for you (e.g. virbr100)."
  );

  async function submit(e) {
    e.preventDefault();
    if (!name.trim() || !bridgeName.trim()) return;
    busy = true;
    err = null;

    const req = {
      name: name.trim(),
      forward_mode: mode,
      bridge_name: bridgeName.trim(),
      forward_dev: forwardDev.trim() || null,
      domain_name: domainName.trim() || null,
      ipv4: null,
      ipv6: null,
      start: startNow,
      autostart,
    };

    if (showIpConfig && ipv4Enabled) {
      req.ipv4 = {
        address: ipv4Address.trim(),
        netmask: ipv4Netmask.trim(),
        dhcp_start: ipv4Dhcp ? ipv4DhcpStart.trim() : null,
        dhcp_end: ipv4Dhcp ? ipv4DhcpEnd.trim() : null,
      };
    }
    if (showIpConfig && ipv6Enabled) {
      req.ipv6 = {
        address: ipv6Address.trim(),
        prefix: Number(ipv6Prefix) || 64,
        dhcp_start: ipv6Dhcp ? ipv6DhcpStart.trim() : null,
        dhcp_end: ipv6Dhcp ? ipv6DhcpEnd.trim() : null,
      };
    }

    try {
      await invoke("create_network", { req });
      await refreshNetworks();
      close();
    } catch (ex) {
      err = ex?.message || String(ex);
      busy = false;
    }
  }
</script>

{#if open}
  <div class="backdrop" onclick={close} role="presentation">
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="cn-title">
      <h3 id="cn-title">New Network</h3>

      <form onsubmit={submit}>
        <fieldset class="mode-picker">
          <legend>Type</legend>
          <div class="mode-grid">
            {#each MODES as m}
              <label class="mode-option" class:active={mode === m.id}>
                <input type="radio" name="mode" value={m.id} bind:group={mode} />
                <div class="mode-label">{m.label}</div>
                <div class="mode-desc">{m.desc}</div>
              </label>
            {/each}
          </div>
        </fieldset>

        <label>
          <span>Name</span>
          <input bind:value={name} placeholder="my-network" required />
        </label>

        <label>
          <span>{bridgeLabel}</span>
          <input bind:value={bridgeName} placeholder={mode === "bridge" ? "br0" : "virbr100"} required />
          <small class="hint">{bridgeHint}</small>
        </label>

        {#if showForwardDev}
          <label>
            <span>Forward Device (optional)</span>
            <input bind:value={forwardDev} placeholder="eth0" />
            <small class="hint">Pin routing to a specific host interface, or leave blank for any.</small>
          </label>
        {/if}

        {#if showIpConfig}
          <label>
            <span>DNS Domain (optional)</span>
            <input bind:value={domainName} placeholder="lab.local" />
          </label>

          <fieldset class="ip-group">
            <legend>
              <label class="toggle">
                <input type="checkbox" bind:checked={ipv4Enabled} />
                <span>IPv4</span>
              </label>
            </legend>
            {#if ipv4Enabled}
              <div class="row">
                <label><span>Address</span><input bind:value={ipv4Address} /></label>
                <label><span>Netmask</span><input bind:value={ipv4Netmask} /></label>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={ipv4Dhcp} />
                <span>Enable DHCP</span>
              </label>
              {#if ipv4Dhcp}
                <div class="row">
                  <label><span>DHCP Start</span><input bind:value={ipv4DhcpStart} /></label>
                  <label><span>DHCP End</span><input bind:value={ipv4DhcpEnd} /></label>
                </div>
              {/if}
            {/if}
          </fieldset>

          <fieldset class="ip-group">
            <legend>
              <label class="toggle">
                <input type="checkbox" bind:checked={ipv6Enabled} />
                <span>IPv6</span>
              </label>
            </legend>
            {#if ipv6Enabled}
              <div class="row">
                <label><span>Address</span><input bind:value={ipv6Address} /></label>
                <label><span>Prefix</span><input type="number" min="1" max="128" bind:value={ipv6Prefix} /></label>
              </div>
              <label class="toggle">
                <input type="checkbox" bind:checked={ipv6Dhcp} />
                <span>Enable DHCPv6</span>
              </label>
              {#if ipv6Dhcp}
                <div class="row">
                  <label><span>DHCP Start</span><input bind:value={ipv6DhcpStart} /></label>
                  <label><span>DHCP End</span><input bind:value={ipv6DhcpEnd} /></label>
                </div>
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

        {#if err}
          <div class="error">{err}</div>
        {/if}

        <div class="actions">
          <button type="button" class="btn-cancel" onclick={close} disabled={busy}>Cancel</button>
          <button type="submit" class="btn-primary" disabled={busy || !name.trim() || !bridgeName.trim()}>
            {busy ? "Creating..." : "Create"}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: 0; background: rgba(0, 0, 0, 0.55);
    display: flex; align-items: center; justify-content: center; z-index: 100;
    padding: 20px;
  }
  .dialog {
    background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 12px; padding: 24px; width: 560px; max-width: 100%;
    max-height: 90vh; overflow-y: auto;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
  }
  h3 { margin: 0 0 16px; font-size: 16px; font-weight: 600; }

  form { display: flex; flex-direction: column; gap: 14px; }
  label { display: flex; flex-direction: column; gap: 4px; }
  label > span { font-size: 11px; font-weight: 500; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  small.hint { font-size: 11px; color: var(--text-muted); margin-top: 2px; }

  .row { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }

  input[type="text"], input:not([type]), input[type="number"] {
    padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit; outline: none;
  }
  input:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  fieldset {
    border: 1px solid var(--border); border-radius: 8px; padding: 12px 14px 14px;
    margin: 0; display: flex; flex-direction: column; gap: 10px;
  }
  legend { padding: 0 6px; font-size: 12px; color: var(--text-muted); font-weight: 500; }

  .mode-picker { gap: 0; padding-bottom: 14px; }
  .mode-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-top: 4px; }
  .mode-option {
    border: 1px solid var(--border); border-radius: 8px; padding: 10px 12px;
    cursor: pointer; transition: background 0.1s, border-color 0.1s;
    display: flex; flex-direction: column; gap: 2px;
  }
  .mode-option:hover { background: var(--bg-hover); }
  .mode-option.active { border-color: var(--accent); background: var(--accent-dim); }
  .mode-option input { position: absolute; opacity: 0; pointer-events: none; }
  .mode-label { font-size: 13px; font-weight: 600; color: var(--text); text-transform: none; letter-spacing: 0; }
  .mode-desc { font-size: 11px; color: var(--text-muted); line-height: 1.35; }

  .ip-group { gap: 10px; }

  .toggle { flex-direction: row; align-items: center; gap: 8px; cursor: pointer; }
  .toggle input { margin: 0; }
  .toggle span { text-transform: none; letter-spacing: normal; color: var(--text); font-size: 13px; font-weight: 400; }

  .flags { display: flex; gap: 20px; padding-top: 4px; }

  .error {
    padding: 8px 12px; background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3); border-radius: 6px;
    color: #ef4444; font-size: 12px;
  }

  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .btn-cancel, .btn-primary { padding: 8px 16px; border-radius: 6px; font-size: 13px; font-family: inherit; cursor: pointer; }
  .btn-cancel { border: 1px solid var(--border); background: var(--bg-button); color: var(--text); }
  .btn-cancel:hover { background: var(--bg-hover); }
  .btn-primary { border: 1px solid var(--accent); background: var(--accent); color: white; }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled, .btn-cancel:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
