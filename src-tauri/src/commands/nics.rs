//! Tauri command surface for Round C — NIC add / remove / update.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::nic_config::NicConfig;
use crate::models::error::VirtManagerError;

/// Read every `<interface>` attached to the domain.
#[tauri::command]
pub fn list_domain_nics(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<NicConfig>, VirtManagerError> {
    state.libvirt().list_domain_nics(&name)
}

/// Attach a new NIC. `live` + `config` map to the libvirt
/// VIR_DOMAIN_AFFECT_LIVE / _CONFIG flags; at least one must be true.
#[tauri::command]
pub fn add_domain_nic(
    state: State<'_, AppState>,
    name: String,
    nic: NicConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_domain_nic(&name, &nic, live, config)
}

/// Detach the NIC identified by MAC (or `vnetN` target dev).
#[tauri::command]
pub fn remove_domain_nic(
    state: State<'_, AppState>,
    name: String,
    mac_or_target: String,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().remove_domain_nic(&name, &mac_or_target, live, config)
}

/// In-place NIC edit (link state flip, filterref change, etc).
/// `nic.mac` must match the existing device.
#[tauri::command]
pub fn update_domain_nic(
    state: State<'_, AppState>,
    name: String,
    nic: NicConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().update_domain_nic(&name, &nic, live, config)
}
