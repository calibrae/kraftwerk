<script>
  import { createNatNetwork } from "$lib/stores/app.svelte.js";

  let { open = $bindable(false) } = $props();

  let name = $state("");
  let bridge = $state("virbr100");
  let ipv4Address = $state("192.168.100.1");
  let ipv4Netmask = $state("255.255.255.0");
  let enableDhcp = $state(true);
  let dhcpStart = $state("192.168.100.100");
  let dhcpEnd = $state("192.168.100.200");
  let busy = $state(false);
  let err = $state(null);

  function reset() {
    name = ""; bridge = "virbr100";
    ipv4Address = "192.168.100.1"; ipv4Netmask = "255.255.255.0";
    enableDhcp = true;
    dhcpStart = "192.168.100.100"; dhcpEnd = "192.168.100.200";
    err = null; busy = false;
  }

  function close() { open = false; reset(); }

  async function submit(e) {
    e.preventDefault();
    if (!name.trim() || !bridge.trim() || !ipv4Address.trim()) return;
    busy = true; err = null;
    try {
      await createNatNetwork({
        name: name.trim(),
        bridge: bridge.trim(),
        ipv4Address: ipv4Address.trim(),
        ipv4Netmask: ipv4Netmask.trim(),
        dhcpStart: enableDhcp ? dhcpStart.trim() : null,
        dhcpEnd: enableDhcp ? dhcpEnd.trim() : null,
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
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="nd-title">
      <h3 id="nd-title">New NAT Network</h3>

      <form onsubmit={submit}>
        <label>
          <span>Name</span>
          <input bind:value={name} placeholder="my-network" required />
        </label>

        <label>
          <span>Bridge Name</span>
          <input bind:value={bridge} placeholder="virbr100" required />
        </label>

        <div class="row">
          <label>
            <span>IPv4 Address</span>
            <input bind:value={ipv4Address} placeholder="192.168.100.1" required />
          </label>
          <label>
            <span>Netmask</span>
            <input bind:value={ipv4Netmask} placeholder="255.255.255.0" required />
          </label>
        </div>

        <label class="checkbox">
          <input type="checkbox" bind:checked={enableDhcp} />
          <span>Enable DHCP</span>
        </label>

        {#if enableDhcp}
          <div class="row">
            <label>
              <span>DHCP Start</span>
              <input bind:value={dhcpStart} />
            </label>
            <label>
              <span>DHCP End</span>
              <input bind:value={dhcpEnd} />
            </label>
          </div>
        {/if}

        {#if err}
          <div class="error">{err}</div>
        {/if}

        <div class="actions">
          <button type="button" class="btn-cancel" onclick={close} disabled={busy}>Cancel</button>
          <button type="submit" class="btn-primary" disabled={busy || !name.trim()}>
            {busy ? "Creating..." : "Create & Start"}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: 0; background: rgba(0, 0, 0, 0.5);
    display: flex; align-items: center; justify-content: center; z-index: 100;
  }
  .dialog {
    background: var(--bg-surface); border: 1px solid var(--border);
    border-radius: 12px; padding: 24px; width: 480px; max-width: 90vw;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
  }
  h3 { margin: 0 0 20px; font-size: 16px; font-weight: 600; }
  form { display: flex; flex-direction: column; gap: 14px; }
  label { display: flex; flex-direction: column; gap: 4px; }
  label span { font-size: 11px; font-weight: 500; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  label.checkbox { flex-direction: row; align-items: center; gap: 8px; }
  label.checkbox span { text-transform: none; letter-spacing: normal; color: var(--text); }
  .row { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }

  input[type="text"], input:not([type]), input[type="number"] {
    padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit; outline: none;
  }
  input:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim); }

  .error {
    padding: 8px 12px; background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3); border-radius: 6px;
    color: #ef4444; font-size: 12px;
  }

  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }

  .btn-cancel {
    padding: 8px 16px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); font-size: 13px; cursor: pointer;
  }
  .btn-cancel:hover { background: var(--bg-hover); }

  .btn-primary {
    padding: 8px 16px; border: 1px solid var(--accent); border-radius: 6px;
    background: var(--accent); color: white; font-size: 13px; cursor: pointer;
  }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled, .btn-cancel:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
