use tauri::State;
use crate::app_state::AppState;
use crate::libvirt::boot_config::{BootConfig, BootPatch};
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn get_boot_config(state: State<'_, AppState>, name: String) -> Result<BootConfig, VirtManagerError> {
    state.libvirt().get_boot_config(&name)
}

#[tauri::command]
pub fn apply_boot_patch(
    state: State<'_, AppState>,
    name: String,
    patch: BootPatch,
) -> Result<(), VirtManagerError> {
    state.libvirt().apply_boot_patch(&name, &patch)
}
