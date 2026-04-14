use tauri::State;
use crate::app_state::AppState;
use crate::libvirt::disk_config::DiskConfig;
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn list_domain_disks(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<DiskConfig>, VirtManagerError> {
    state.libvirt().list_domain_disks(&name)
}

#[tauri::command]
pub fn add_domain_disk(
    state: State<'_, AppState>,
    name: String,
    disk: DiskConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_domain_disk(&name, &disk, live, config)
}

#[tauri::command]
pub fn remove_domain_disk(
    state: State<'_, AppState>,
    name: String,
    target_dev: String,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().remove_domain_disk(&name, &target_dev, live, config)
}

#[tauri::command]
pub fn update_domain_disk(
    state: State<'_, AppState>,
    name: String,
    disk: DiskConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().update_domain_disk(&name, &disk, live, config)
}
