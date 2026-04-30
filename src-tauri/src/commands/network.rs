use serde::Deserialize;
use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::network_config::{
    self, Ipv4BuildParams, Ipv6BuildParams, NetworkBuildParams, NetworkConfig,
};
use crate::models::error::VirtManagerError;
use crate::models::network::NetworkInfo;

#[derive(Debug, Deserialize)]
pub struct Ipv4Params {
    pub address: String,
    pub netmask: String,
    #[serde(default)]
    pub dhcp_start: Option<String>,
    #[serde(default)]
    pub dhcp_end: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Ipv6Params {
    pub address: String,
    pub prefix: u32,
    #[serde(default)]
    pub dhcp_start: Option<String>,
    #[serde(default)]
    pub dhcp_end: Option<String>,
}

/// Request body for creating any kind of network.
#[derive(Debug, Deserialize)]
pub struct CreateNetworkRequest {
    pub name: String,
    /// "nat" | "route" | "open" | "bridge" | "isolated" (empty = isolated)
    pub forward_mode: String,
    pub bridge_name: String,
    #[serde(default)]
    pub forward_dev: Option<String>,
    #[serde(default)]
    pub domain_name: Option<String>,
    #[serde(default)]
    pub ipv4: Option<Ipv4Params>,
    #[serde(default)]
    pub ipv6: Option<Ipv6Params>,
    /// If true, define + start. If false, just define.
    #[serde(default = "default_true")]
    pub start: bool,
    /// If true, enable autostart on boot.
    #[serde(default)]
    pub autostart: bool,
}

fn default_true() -> bool { true }

/// List all virtual networks.
#[tauri::command]
pub fn list_networks(state: State<'_, AppState>) -> Result<Vec<NetworkInfo>, VirtManagerError> {
    state.libvirt().list_networks()
}

#[tauri::command]
pub fn get_network_config(
    state: State<'_, AppState>,
    name: String,
) -> Result<NetworkConfig, VirtManagerError> {
    state.libvirt().get_network_config(&name)
}

#[tauri::command]
pub fn get_network_xml(
    state: State<'_, AppState>,
    name: String,
) -> Result<String, VirtManagerError> {
    state.libvirt().get_network_xml(&name)
}

#[tauri::command]
pub fn start_network(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().start_network(&name)
}

#[tauri::command]
pub fn stop_network(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().stop_network(&name)
}

#[tauri::command]
pub fn delete_network(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().delete_network(&name)
}

#[tauri::command]
pub fn set_network_autostart(
    state: State<'_, AppState>,
    name: String,
    autostart: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_network_autostart(&name, autostart)
}

/// Create a network of any supported mode from structured params.
/// This is the new unified creation command.
#[tauri::command]
pub fn create_network(
    state: State<'_, AppState>,
    req: CreateNetworkRequest,
) -> Result<(), VirtManagerError> {
    let ipv4 = req.ipv4.as_ref().map(|v| Ipv4BuildParams {
        address: v.address.as_str(),
        netmask: v.netmask.as_str(),
        dhcp_start: v.dhcp_start.as_deref(),
        dhcp_end: v.dhcp_end.as_deref(),
    });
    let ipv6 = req.ipv6.as_ref().map(|v| Ipv6BuildParams {
        address: v.address.as_str(),
        prefix: v.prefix,
        dhcp_start: v.dhcp_start.as_deref(),
        dhcp_end: v.dhcp_end.as_deref(),
    });

    let xml = network_config::build_network_xml(&NetworkBuildParams {
        name: req.name.as_str(),
        forward_mode: req.forward_mode.as_str(),
        bridge_name: req.bridge_name.as_str(),
        forward_dev: req.forward_dev.as_deref(),
        domain_name: req.domain_name.as_deref(),
        ipv4,
        ipv6,
    });

    let lv = state.libvirt();
    if req.start {
        lv.create_network(&xml)?;
    } else {
        lv.define_network(&xml)?;
    }
    if req.autostart {
        // Best-effort autostart
        let _ = lv.set_network_autostart(&req.name, true);
    }
    Ok(())
}

/// Legacy command — kept for back-compat. Prefer `create_network`.
#[tauri::command]
pub fn create_nat_network(
    state: State<'_, AppState>,
    name: String,
    bridge: String,
    ipv4_address: String,
    ipv4_netmask: String,
    dhcp_start: Option<String>,
    dhcp_end: Option<String>,
) -> Result<(), VirtManagerError> {
    let xml = network_config::build_nat_network_xml(
        &name,
        &bridge,
        &ipv4_address,
        &ipv4_netmask,
        dhcp_start.as_deref(),
        dhcp_end.as_deref(),
    );
    state.libvirt().create_network(&xml)
}

// ── Per-host DHCP / DNS entries (phase 4.1) ──

#[tauri::command]
pub fn add_dhcp_host(
    state: State<'_, AppState>,
    network: String,
    mac: Option<String>,
    name: Option<String>,
    ip: String,
) -> Result<(), VirtManagerError> {
    let snippet = crate::libvirt::network_config::build_dhcp_host_xml(
        mac.as_deref().filter(|s| !s.is_empty()),
        name.as_deref().filter(|s| !s.is_empty()),
        &ip,
    );
    // VIR_NETWORK_UPDATE_COMMAND_ADD_LAST = 3
    // VIR_NETWORK_SECTION_IP_DHCP_HOST    = 4
    state.libvirt().network_update_section(&network, 3, 4, &snippet)
}

#[tauri::command]
pub fn remove_dhcp_host(
    state: State<'_, AppState>,
    network: String,
    mac: Option<String>,
    name: Option<String>,
    ip: String,
) -> Result<(), VirtManagerError> {
    let snippet = crate::libvirt::network_config::build_dhcp_host_xml(
        mac.as_deref().filter(|s| !s.is_empty()),
        name.as_deref().filter(|s| !s.is_empty()),
        &ip,
    );
    // VIR_NETWORK_UPDATE_COMMAND_DELETE = 2
    state.libvirt().network_update_section(&network, 2, 4, &snippet)
}

#[tauri::command]
pub fn add_dns_host(
    state: State<'_, AppState>,
    network: String,
    ip: String,
    hostnames: Vec<String>,
) -> Result<(), VirtManagerError> {
    let snippet = crate::libvirt::network_config::build_dns_host_xml(&ip, &hostnames);
    // SECTION_DNS_HOST = 10
    state.libvirt().network_update_section(&network, 3, 10, &snippet)
}

#[tauri::command]
pub fn remove_dns_host(
    state: State<'_, AppState>,
    network: String,
    ip: String,
    hostnames: Vec<String>,
) -> Result<(), VirtManagerError> {
    let snippet = crate::libvirt::network_config::build_dns_host_xml(&ip, &hostnames);
    state.libvirt().network_update_section(&network, 2, 10, &snippet)
}

#[tauri::command]
pub fn add_network_route(
    state: State<'_, AppState>,
    network: String,
    family: String,
    address: String,
    prefix: u32,
    gateway: String,
) -> Result<(), VirtManagerError> {
    let route = crate::libvirt::network_config::NetworkRoute {
        family, address, prefix, gateway,
    };
    state.libvirt().add_network_route(&network, &route)
}

#[tauri::command]
pub fn remove_network_route(
    state: State<'_, AppState>,
    network: String,
    family: String,
    address: String,
    prefix: u32,
    gateway: String,
) -> Result<(), VirtManagerError> {
    let route = crate::libvirt::network_config::NetworkRoute {
        family, address, prefix, gateway,
    };
    state.libvirt().remove_network_route(&network, &route)
}

#[tauri::command]
pub fn list_nw_filters(
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::nwfilter::NwFilterInfo>, VirtManagerError> {
    state.libvirt().list_nw_filters()
}

#[tauri::command]
pub fn get_nw_filter_xml(
    state: State<'_, AppState>,
    name: String,
) -> Result<String, VirtManagerError> {
    state.libvirt().get_nw_filter_xml(&name)
}
