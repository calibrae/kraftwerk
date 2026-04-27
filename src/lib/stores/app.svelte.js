import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** @typedef {{ id: string, display_name: string, uri: string, auth_type: string, last_connected: number|null }} SavedConnection */
/** @typedef {{ name: string, uuid: string, state: string, vcpus: number, memory_mb: number, graphics_type: string|null, has_serial: boolean }} VmInfo */

// Reactive state
let savedConnections = $state([]);
let connectionStates = $state({});
let vms = $state([]);
let networks = $state([]);
let pools = $state([]);
let volumesByPool = $state({});
let selectedConnectionId = $state(null);
let selectedVmName = $state(null);
// Set of names for multi-select. Always kept in sync with selectedVmName
// (selectedVmName === single-selection focus, used by VmDetail). When
// the set is empty or has one item we collapse to single-select; when
// >1 we surface the bulk-action toolbar instead of the VmDetail.
let selectedVmNames = $state(new Set());
let error = $state(null);
let loading = $state(false);
let inFlight = false;

export function getState() {
  return {
    get savedConnections() { return savedConnections; },
    get connectionStates() { return connectionStates; },
    get vms() { return vms; },
    get networks() { return networks; },
    get pools() { return pools; },
    get volumesByPool() { return volumesByPool; },
    get selectedConnectionId() { return selectedConnectionId; },
    get selectedVmName() { return selectedVmName; },
    get selectedVmNames() { return selectedVmNames; },
    get hasMultiSelect() { return selectedVmNames.size > 1; },
    get error() { return error; },
    get loading() { return loading; },
    get selectedVm() {
      if (!selectedVmName) return null;
      return vms.find(v => v.name === selectedVmName) ?? null;
    },
    get selectedConnection() {
      if (!selectedConnectionId) return null;
      return savedConnections.find(c => c.id === selectedConnectionId) ?? null;
    },
    get isConnected() {
      if (!selectedConnectionId) return false;
      return connectionStates[selectedConnectionId]?.status === "connected";
    }
  };
}

export async function loadConnections() {
  try {
    savedConnections = await invoke("list_saved_connections");
  } catch (e) {
    error = e;
  }
}

export async function addConnection(displayName, uri, authType) {
  try {
    error = null;
    const conn = await invoke("add_connection", {
      displayName, uri, authType
    });
    savedConnections = [...savedConnections, conn];
    return conn;
  } catch (e) {
    error = e;
    throw e;
  }
}

export async function updateConnection(id, displayName, uri, authType) {
  const updated = await invoke("update_connection", {
    id,
    displayName,
    uri,
    authType,
  });
  savedConnections = savedConnections.map(c => c.id === id ? updated : c);
  return updated;
}

export async function removeConnection(id) {
  try {
    await invoke("remove_connection", { id });
    savedConnections = savedConnections.filter(c => c.id !== id);
    if (selectedConnectionId === id) {
      selectedConnectionId = null;
      vms = [];
      selectedVmName = null;
    }
  } catch (e) {
    error = e;
  }
}

export async function connect(id) {
  try {
    error = null;
    loading = true;
    connectionStates = { ...connectionStates, [id]: { status: "connecting" } };
    selectedConnectionId = id;
    const domainList = await invoke("connect", { id });
    connectionStates = { ...connectionStates, [id]: { status: "connected" } };
    pollFailures.delete(id);
    vms = domainList;
    selectedVmName = null;
    // Load networks in parallel
    try { networks = await invoke("list_networks"); } catch (_) { networks = []; }
    try { pools = await invoke("list_storage_pools"); } catch (_) { pools = []; }
  } catch (e) {
    connectionStates = { ...connectionStates, [id]: { status: "error", message: e.message || String(e) } };
    error = e;
  } finally {
    loading = false;
  }
}

export async function disconnect(id) {
  try {
    await invoke("disconnect", { id });
    connectionStates = { ...connectionStates, [id]: { status: "disconnected" } };
    pollFailures.delete(id);
    if (selectedConnectionId === id) {
      vms = [];
      networks = [];
      pools = [];
      volumesByPool = {};
      selectedVmName = null;
    }
  } catch (e) {
    error = e;
  }
}

export async function refreshVms() {
  try {
    vms = await invoke("list_domains");
  } catch (e) {
    error = e;
  }
}

export function selectVm(name, ev) {
  // Cmd/Ctrl-click: toggle this VM in the multi-set without changing focus.
  // Shift-click: extend a contiguous range from the focused VM to this one.
  // Plain click: collapse to a single selection on `name`.
  if (ev?.metaKey || ev?.ctrlKey) {
    const next = new Set(selectedVmNames);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    if (selectedVmName && !next.has(selectedVmName)) next.add(selectedVmName);
    selectedVmNames = next;
    selectedVmName = name; // focus the most recently clicked
    return;
  }
  if (ev?.shiftKey && selectedVmName) {
    const order = vms.map(v => v.name);
    const a = order.indexOf(selectedVmName);
    const b = order.indexOf(name);
    if (a >= 0 && b >= 0) {
      const [lo, hi] = a < b ? [a, b] : [b, a];
      const next = new Set(order.slice(lo, hi + 1));
      selectedVmNames = next;
      selectedVmName = name;
      return;
    }
  }
  selectedVmName = name;
  selectedVmNames = new Set([name]);
}

export function clearVmSelection() {
  selectedVmName = null;
  selectedVmNames = new Set();
}

// Apply a per-VM action across the current multi-selection.
// `action` is a Tauri command name like "start_domain", "shutdown_domain",
// etc. Each call is awaited sequentially so failures are surfaced in
// order; the loop continues past errors and reports the count.
export async function bulkAction(action) {
  if (selectedVmNames.size === 0) return { ok: 0, failed: 0, errors: [] };
  loading = true;
  let ok = 0, failed = 0;
  const errors = [];
  for (const name of selectedVmNames) {
    try {
      await invoke(action, { name });
      ok++;
    } catch (e) {
      failed++;
      errors.push({ name, error: e?.message || String(e) });
    }
  }
  loading = false;
  await refreshVms();
  if (failed > 0) {
    error = new Error(`${failed} of ${ok + failed} ${action} calls failed:\n` +
      errors.map(e => `  ${e.name}: ${e.error}`).join("\n"));
  }
  return { ok, failed, errors };
}

export function clearError() {
  error = null;
}

let domainEventUnlisten = null;

// Subscribe to libvirt lifecycle events emitted by the Rust backend.
// Idempotent — calling twice keeps the first subscription.
export async function subscribeDomainEvents() {
  if (domainEventUnlisten) return;
  domainEventUnlisten = await listen("domain_event", async (msg) => {
    if (!selectedConnectionId) return;
    if (connectionStates[selectedConnectionId]?.status !== "connected") return;
    // Coalesce bursts: if a previous refresh is already in-flight just let
    // it ride. The fast-poll cadence will catch any straggler.
    if (inFlight) return;
    inFlight = true;
    try {
      vms = await invoke("list_domains");
    } catch (_) {
      // ignore — next poll will retry
    } finally {
      inFlight = false;
    }
  });
}

export async function unsubscribeDomainEvents() {
  if (domainEventUnlisten) {
    domainEventUnlisten();
    domainEventUnlisten = null;
  }
}

// Domain actions
export async function startDomain(name) {
  try {
    error = null;
    await invoke("start_domain", { name });
    await refreshVms();
  } catch (e) { error = e; }
}

export async function shutdownDomain(name) {
  try {
    error = null;
    await invoke("shutdown_domain", { name });
    await refreshVms();
  } catch (e) { error = e; }
}

export async function destroyDomain(name) {
  try {
    error = null;
    await invoke("destroy_domain", { name });
    await refreshVms();
  } catch (e) { error = e; }
}

export async function managedSaveDomain(name) {
  try {
    error = null;
    await invoke("managed_save_domain", { name });
    await refreshVms();
  } catch (e) { error = e; }
}

export async function managedSaveRemove(name) {
  try {
    error = null;
    await invoke("managed_save_remove", { name });
    await refreshVms();
  } catch (e) { error = e; }
}

export async function hasManagedSave(name) {
  try {
    return await invoke("has_managed_save", { name });
  } catch (_) { return false; }
}

export async function coreDumpDomain(name, path, crashAfter, live) {
  try {
    error = null;
    await invoke("core_dump_domain", { name, path, crashAfter, live });
    await refreshVms();
  } catch (e) { error = e; throw e; }
}

export async function screenshotDomain(name, screen = 0) {
  try {
    error = null;
    return await invoke("screenshot_domain", { name, screen });
  } catch (e) { error = e; throw e; }
}

export async function suspendDomain(name) {
  try {
    error = null;
    await invoke("suspend_domain", { name });
    await refreshVms();
  } catch (e) { error = e; }
}

export async function resumeDomain(name) {
  try {
    error = null;
    await invoke("resume_domain", { name });
    await refreshVms();
  } catch (e) { error = e; }
}

export async function rebootDomain(name) {
  try {
    error = null;
    await invoke("reboot_domain", { name });
    await refreshVms();
  } catch (e) { error = e; }
}

export async function getDomainXml(name, inactive = false) {
  try {
    return await invoke("get_domain_xml", { name, inactive });
  } catch (e) {
    error = e;
    return null;
  }
}


export async function refreshNetworks() {
  try { networks = await invoke("list_networks"); } catch (e) { error = e; }
}

export async function startNetwork(name) {
  try { error = null; await invoke("start_network", { name }); await refreshNetworks(); }
  catch (e) { error = e; }
}

export async function stopNetwork(name) {
  try { error = null; await invoke("stop_network", { name }); await refreshNetworks(); }
  catch (e) { error = e; }
}

export async function deleteNetwork(name) {
  try { error = null; await invoke("delete_network", { name }); await refreshNetworks(); }
  catch (e) { error = e; }
}

export async function setNetworkAutostart(name, autostart) {
  try { error = null; await invoke("set_network_autostart", { name, autostart }); await refreshNetworks(); }
  catch (e) { error = e; }
}

export async function createNatNetwork(params) {
  try {
    error = null;
    await invoke("create_nat_network", params);
    await refreshNetworks();
  } catch (e) {
    error = e;
    throw e;
  }
}

export async function getNetworkConfig(name) {
  try { return await invoke("get_network_config", { name }); }
  catch (e) { error = e; return null; }
}

export async function getNetworkXml(name) {
  try { return await invoke("get_network_xml", { name }); }
  catch (e) { error = e; return null; }
}


// ── Storage actions ──

export async function refreshPools() {
  try { pools = await invoke("list_storage_pools"); } catch (e) { error = e; }
}

export async function refreshVolumes(poolName) {
  try {
    const vols = await invoke("list_volumes", { poolName });
    volumesByPool = { ...volumesByPool, [poolName]: vols };
    return vols;
  } catch (e) { error = e; return []; }
}

export async function startPool(name) {
  try { error = null; await invoke("start_pool", { name }); await refreshPools(); }
  catch (e) { error = e; }
}

export async function stopPool(name) {
  try { error = null; await invoke("stop_pool", { name }); await refreshPools(); }
  catch (e) { error = e; }
}

export async function refreshPoolVolumes(name) {
  try { error = null; await invoke("refresh_pool", { name }); await refreshVolumes(name); await refreshPools(); }
  catch (e) { error = e; }
}

export async function deletePool(name) {
  try { error = null; await invoke("delete_pool", { name }); await refreshPools(); }
  catch (e) { error = e; }
}

export async function setPoolAutostart(name, autostart) {
  try { error = null; await invoke("set_pool_autostart", { name, autostart }); await refreshPools(); }
  catch (e) { error = e; }
}

export async function createPool(req) {
  try { error = null; await invoke("create_pool", { req }); await refreshPools(); }
  catch (e) { error = e; throw e; }
}

export async function createVolume(req) {
  try {
    error = null;
    const path = await invoke("create_volume", { req });
    await refreshVolumes(req.pool_name);
    await refreshPools();
    return path;
  } catch (e) { error = e; throw e; }
}

export async function deleteVolume(poolName, path) {
  try { error = null; await invoke("delete_volume", { path }); await refreshVolumes(poolName); await refreshPools(); }
  catch (e) { error = e; }
}

export async function resizeVolume(poolName, path, capacityBytes) {
  try { error = null; await invoke("resize_volume", { path, capacityBytes }); await refreshVolumes(poolName); }
  catch (e) { error = e; throw e; }
}


// ── Auto-refresh ──
//
// 3s VM-state poll while connected. Detects guest-initiated shutdowns,
// crashes, libvirt-side changes that kraftwerk didn't cause.
// 30s interval for networks + pools (state changes rare).

let vmPollTimer = null;
let slowPollTimer = null;
// Consecutive failed polls per connection id. Flip to "error" at threshold.
const pollFailures = new Map();
const POLL_FAILURE_THRESHOLD = 3;

export function startAutoPolls() {
  stopAutoPolls();
  vmPollTimer = setInterval(async () => {
    if (inFlight || !selectedConnectionId) return;
    if (connectionStates[selectedConnectionId]?.status !== "connected") return;
    inFlight = true;
    const id = selectedConnectionId;
    try {
      const fresh = await invoke("list_domains");
      pollFailures.set(id, 0);
      if (JSON.stringify(fresh) !== JSON.stringify(vms)) {
        vms = fresh;
      }
    } catch (e) {
      const n = (pollFailures.get(id) ?? 0) + 1;
      pollFailures.set(id, n);
      if (n >= POLL_FAILURE_THRESHOLD) {
        connectionStates = {
          ...connectionStates,
          [id]: { status: "error", message: `Connection lost: ${e?.message || String(e)}` },
        };
        pollFailures.delete(id);
      }
    } finally {
      inFlight = false;
    }
  }, 15000);

  slowPollTimer = setInterval(async () => {
    if (!selectedConnectionId) return;
    if (connectionStates[selectedConnectionId]?.status !== "connected") return;
    try {
      const [n, p] = await Promise.all([
        invoke("list_networks").catch(() => networks),
        invoke("list_storage_pools").catch(() => pools),
      ]);
      if (JSON.stringify(n) !== JSON.stringify(networks)) networks = n;
      if (JSON.stringify(p) !== JSON.stringify(pools)) pools = p;
    } catch (_) {}
  }, 30000);
}

export function stopAutoPolls() {
  if (vmPollTimer) { clearInterval(vmPollTimer); vmPollTimer = null; }
  if (slowPollTimer) { clearInterval(slowPollTimer); slowPollTimer = null; }
}
