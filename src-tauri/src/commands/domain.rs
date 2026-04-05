use tauri::State;

use crate::app_state::AppState;
use crate::models::error::VirtManagerError;
use crate::models::vm::VmInfo;

/// List all VMs on the connected hypervisor.
#[tauri::command]
pub fn list_domains(state: State<'_, AppState>) -> Result<Vec<VmInfo>, VirtManagerError> {
    state.libvirt().list_all_domains()
}

/// Start a VM by name.
#[tauri::command]
pub fn start_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().start_domain(&name)
}

/// Gracefully shutdown a VM.
#[tauri::command]
pub fn shutdown_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().shutdown_domain(&name)
}

/// Force stop a VM.
#[tauri::command]
pub fn destroy_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().destroy_domain(&name)
}

/// Suspend a VM.
#[tauri::command]
pub fn suspend_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().suspend_domain(&name)
}

/// Resume a paused VM.
#[tauri::command]
pub fn resume_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().resume_domain(&name)
}

/// Reboot a VM.
#[tauri::command]
pub fn reboot_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().reboot_domain(&name)
}

/// Get the XML description for a VM.
#[tauri::command]
pub fn get_domain_xml(
    state: State<'_, AppState>,
    name: String,
    inactive: bool,
) -> Result<String, VirtManagerError> {
    state.libvirt().get_domain_xml(&name, inactive)
}
