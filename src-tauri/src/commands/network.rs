use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::network_config::{self, NetworkConfig};
use crate::models::error::VirtManagerError;
use crate::models::network::NetworkInfo;

/// List all virtual networks.
#[tauri::command]
pub fn list_networks(state: State<'_, AppState>) -> Result<Vec<NetworkInfo>, VirtManagerError> {
    state.libvirt().list_networks()
}

/// Get parsed network config by name.
#[tauri::command]
pub fn get_network_config(
    state: State<'_, AppState>,
    name: String,
) -> Result<NetworkConfig, VirtManagerError> {
    state.libvirt().get_network_config(&name)
}

/// Get the raw XML for a network.
#[tauri::command]
pub fn get_network_xml(
    state: State<'_, AppState>,
    name: String,
) -> Result<String, VirtManagerError> {
    state.libvirt().get_network_xml(&name)
}

/// Start a network.
#[tauri::command]
pub fn start_network(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().start_network(&name)
}

/// Stop a network.
#[tauri::command]
pub fn stop_network(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().stop_network(&name)
}

/// Delete a network (stops it first if active).
#[tauri::command]
pub fn delete_network(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().delete_network(&name)
}

/// Set autostart flag.
#[tauri::command]
pub fn set_network_autostart(
    state: State<'_, AppState>,
    name: String,
    autostart: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_network_autostart(&name, autostart)
}

/// Create a NAT network with minimal config.
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
