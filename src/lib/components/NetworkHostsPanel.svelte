<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  let { networkName } = $props();

  let cfg = $state(null);
  let loading = $state(false);
  let busy = $state(false);
  let err = $state(null);

  // DHCP form
  let dhcpMac = $state("");
  let dhcpName = $state("");
  let dhcpIp = $state("");

  // DNS form
  let dnsIp = $state("");
  let dnsHostnames = $state("");

  async function load() {
    if (!networkName) return;
    loading = true;
    err = null;
    try {
      cfg = await invoke("get_network_config", { name: networkName });
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);
  let lastLoadedFor = null;
  $effect(() => {
    if (networkName && networkName !== lastLoadedFor) {
      lastLoadedFor = networkName;
      load();
    }
  });

  async function addDhcp() {
    if (!dhcpIp.trim()) { err = "IP required"; return; }
    busy = true; err = null;
    try {
      await invoke("add_dhcp_host", {
        network: networkName,
        mac: dhcpMac.trim() || null,
        name: dhcpName.trim() || null,
        ip: dhcpIp.trim(),
      });
      dhcpMac = ""; dhcpName = ""; dhcpIp = "";
      await load();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  async function removeDhcp(host) {
    if (!confirm(`Remove DHCP host ${host.ip}${host.mac ? " ("+host.mac+")" : ""}?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_dhcp_host", {
        network: networkName,
        mac: host.mac ?? null,
        name: host.name ?? null,
        ip: host.ip,
      });
      await load();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  async function addDns() {
    if (!dnsIp.trim() || !dnsHostnames.trim()) {
      err = "IP and at least one hostname required"; return;
    }
    busy = true; err = null;
    try {
      const names = dnsHostnames
        .split(/[\s,]+/)
        .map(h => h.trim())
        .filter(Boolean);
      await invoke("add_dns_host", {
        network: networkName,
        ip: dnsIp.trim(),
        hostnames: names,
      });
      dnsIp = ""; dnsHostnames = "";
      await load();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  async function removeDns(entry) {
    if (!confirm(`Remove DNS host ${entry.ip} (${entry.hostnames.join(", ")})?`)) return;
    busy = true; err = null;
    try {
      await invoke("remove_dns_host", {
        network: networkName,
        ip: entry.ip,
        hostnames: entry.hostnames,
      });
      await load();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  let dhcpHosts = $derived.by(() => {
    const v4 = cfg?.ipv4?.dhcp_hosts ?? [];
    const v6 = cfg?.ipv6?.dhcp_hosts ?? [];
    return [...v4, ...v6];
  });
</script>

<section class="panel">
  <header><h3>DHCP &amp; DNS</h3>
    {#if loading}<span class="muted small">Loading…</span>{/if}
  </header>
  {#if err}<pre class="err">{err}</pre>{/if}

  <h4>Static DHCP Reservations <span class="count">{dhcpHosts.length}</span></h4>
  {#if dhcpHosts.length === 0}
    <p class="muted small">No reservations.</p>
  {:else}
    <ul class="rows">
      {#each dhcpHosts as h (h.mac ?? h.ip)}
        <li>
          <code class="ip">{h.ip}</code>
          {#if h.mac}<code class="mac">{h.mac}</code>{/if}
          {#if h.name}<span class="hostname">{h.name}</span>{/if}
          <span class="grow"></span>
          <button class="btn-tiny danger" onclick={() => removeDhcp(h)} disabled={busy}>Remove</button>
        </li>
      {/each}
    </ul>
  {/if}

  <form class="add" onsubmit={(e) => { e.preventDefault(); addDhcp(); }}>
    <input type="text" bind:value={dhcpMac} placeholder="MAC (optional, 52:54:00:…)" disabled={busy} />
    <input type="text" bind:value={dhcpName} placeholder="hostname (optional)" disabled={busy} />
    <input type="text" bind:value={dhcpIp} placeholder="IP address (required)" disabled={busy} />
    <button type="submit" class="btn-tiny primary" disabled={busy}>Add</button>
  </form>
  <p class="muted small">
    Pin a MAC to an IP so libvirt's dnsmasq always hands the same lease.
    Affects live + persistent config; takes effect immediately for the
    next DHCP request.
  </p>

  <h4>DNS Hostname Overrides <span class="count">{cfg?.dns_hosts?.length ?? 0}</span></h4>
  {#if (cfg?.dns_hosts ?? []).length === 0}
    <p class="muted small">No overrides.</p>
  {:else}
    <ul class="rows">
      {#each cfg.dns_hosts as h (h.ip)}
        <li>
          <code class="ip">{h.ip}</code>
          {#each h.hostnames as n}<span class="hostname">{n}</span>{/each}
          <span class="grow"></span>
          <button class="btn-tiny danger" onclick={() => removeDns(h)} disabled={busy}>Remove</button>
        </li>
      {/each}
    </ul>
  {/if}

  <form class="add" onsubmit={(e) => { e.preventDefault(); addDns(); }}>
    <input type="text" bind:value={dnsIp} placeholder="IP address" disabled={busy} />
    <input type="text" bind:value={dnsHostnames} placeholder="hostnames (space or comma separated)" disabled={busy} />
    <button type="submit" class="btn-tiny primary" disabled={busy}>Add</button>
  </form>
  <p class="muted small">
    Resolved by the network's local dnsmasq. Useful for naming guests
    that don't register themselves in DNS.
  </p>
</section>

<style>
  .panel { padding: 12px 0; display: flex; flex-direction: column; gap: 10px; }
  header { display: flex; align-items: baseline; gap: 8px; }
  header h3 { margin: 0; font-size: 13px; font-weight: 600; }
  h4 { margin: 8px 0 4px; font-size: 12px; font-weight: 500; color: var(--text-muted); display: flex; align-items: center; gap: 6px; }
  .count {
    font-size: 10px;
    background: var(--bg-input);
    padding: 1px 6px;
    border-radius: 999px;
    color: var(--text);
  }
  .muted { color: var(--text-muted); }
  .small { font-size: 11px; }
  .grow { flex: 1; }

  ul.rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  ul.rows li {
    display: flex; align-items: center; gap: 10px;
    padding: 6px 10px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12px;
  }
  .ip { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--text); min-width: 110px; }
  .mac {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    color: var(--text-muted);
    font-size: 11px;
  }
  .hostname {
    font-size: 11px;
    background: rgba(96, 165, 250, 0.12);
    color: #60a5fa;
    padding: 1px 6px;
    border-radius: 3px;
  }

  form.add { display: flex; gap: 6px; flex-wrap: wrap; }
  form.add input {
    flex: 1; min-width: 160px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text);
    font-size: 12px;
    font-family: inherit;
  }

  .btn-tiny {
    padding: 4px 10px; font-size: 11px;
    border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text);
    cursor: pointer; font-family: inherit;
  }
  .btn-tiny:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-tiny:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-tiny.danger { color: #ef4444; }
  .btn-tiny.primary { background: var(--accent); color: white; border-color: var(--accent); }
  .btn-tiny.primary:hover:not(:disabled) { filter: brightness(1.1); }

  pre.err {
    margin: 0; padding: 6px 10px;
    background: rgba(239, 68, 68, 0.10);
    border: 1px solid rgba(239, 68, 68, 0.30);
    border-radius: 4px;
    color: #ef4444; font-size: 11px;
    white-space: pre-wrap;
  }
</style>
