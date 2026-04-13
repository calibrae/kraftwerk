import { invoke } from "@tauri-apps/api/core";

/** @typedef {{ id: string, display_name: string, uri: string, auth_type: string, last_connected: number|null }} SavedConnection */
/** @typedef {{ name: string, uuid: string, state: string, vcpus: number, memory_mb: number, graphics_type: string|null, has_serial: boolean }} VmInfo */

// Reactive state
let savedConnections = $state([]);
let connectionStates = $state({});
let vms = $state([]);
let networks = $state([]);
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
