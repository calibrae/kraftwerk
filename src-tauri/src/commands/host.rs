//! Tauri commands surfacing host (hypervisor) info for the dashboard view.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::host_info::{HostInfo, HostMemory};
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn get_host_info(state: State<'_, AppState>) -> Result<HostInfo, VirtManagerError> {
    state.libvirt().get_host_info()
}

#[tauri::command]
pub fn get_host_memory(state: State<'_, AppState>) -> Result<HostMemory, VirtManagerError> {
    state.libvirt().get_host_memory()
}
