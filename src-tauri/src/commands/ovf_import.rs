//! Tauri commands for OVA / OVF import.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::ovf_import::OvfMetadata;
use crate::models::error::VirtManagerError;

/// Read an OVA's OVF descriptor and parse it. Doesn't extract any
/// disks — used by the wizard to preview metadata.
#[tauri::command]
pub fn inspect_ova(
    state: State<'_, AppState>,
    ova_path: String,
) -> Result<OvfMetadata, VirtManagerError> {
    state.libvirt().inspect_ova(&ova_path)
}

/// Import an OVA: convert each VMDK to qcow2 in `pool_name` and
/// define a domain XML. Returns the new domain's name.
#[tauri::command]
pub fn import_ova(
    state: State<'_, AppState>,
    ova_path: String,
    pool_name: String,
    target_name: Option<String>,
    network_name: Option<String>,
) -> Result<String, VirtManagerError> {
    state.libvirt().import_ova(
        &ova_path,
        &pool_name,
        target_name.as_deref(),
        network_name.as_deref(),
    )
}
