import { invoke } from "@tauri-apps/api/core";

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
let error = $state(null);
let loading = $state(false);

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

export function selectVm(name) {
  selectedVmName = name;
}

export function clearError() {
  error = null;
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
// crashes, libvirt-side changes that virtmanager-rs didn't cause.
// 30s interval for networks + pools (state changes rare).

let vmPollTimer = null;
let slowPollTimer = null;
let inFlight = false;

export function startAutoPolls() {
  stopAutoPolls();
  vmPollTimer = setInterval(async () => {
    if (inFlight || !selectedConnectionId) return;
    if (connectionStates[selectedConnectionId]?.status !== "connected") return;
    inFlight = true;
    try {
      const fresh = await invoke("list_domains");
      // Only assign if something actually changed, so we don't trigger
      // unnecessary reactive updates.
      if (JSON.stringify(fresh) !== JSON.stringify(vms)) {
        vms = fresh;
      }
    } catch (_) {
      // ignore transient errors; next tick will retry
    } finally {
      inFlight = false;
    }
  }, 3000);

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
