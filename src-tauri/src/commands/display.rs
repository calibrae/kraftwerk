use tauri::State;
use crate::app_state::AppState;
use crate::libvirt::display_config::{DisplayConfig, DisplayPatch};
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn get_display_config(
    state: State<'_, AppState>,
    name: String,
) -> Result<DisplayConfig, VirtManagerError> {
    state.libvirt().get_display_config(&name)
}

#[tauri::command]
pub fn apply_display_patch(
    state: State<'_, AppState>,
    name: String,
    patch: DisplayPatch,
) -> Result<(), VirtManagerError> {
    state.libvirt().apply_display_patch(&name, &patch)
}
