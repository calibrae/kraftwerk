//! Controllers (Round H) — Tauri commands.
//!
//! Edits the bus-like devices: USB / SCSI / virtio-serial / IDE / SATA
//! / PCI(read-only) / CCID / FDC. All operations default to persistent
//! (next-boot); most controller changes require shutdown anyway.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::controller_config::ControllerConfig;
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn list_controllers(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<ControllerConfig>, VirtManagerError> {
    state.libvirt().list_controllers(&name)
}

#[tauri::command]
pub fn add_controller(
    state: State<'_, AppState>,
    name: String,
    controller: ControllerConfig,
    live: Option<bool>,
    config: Option<bool>,
) -> Result<(), VirtManagerError> {
    state
        .libvirt()
        .add_controller(&name, &controller, live.unwrap_or(false), config.unwrap_or(true))
}

#[tauri::command]
pub fn remove_controller(
    state: State<'_, AppState>,
    name: String,
    controller_type: String,
    index: u32,
    live: Option<bool>,
    config: Option<bool>,
) -> Result<(), VirtManagerError> {
    state.libvirt().remove_controller(
        &name,
        &controller_type,
        index,
        live.unwrap_or(false),
        config.unwrap_or(true),
    )
}

#[tauri::command]
pub fn update_controller(
    state: State<'_, AppState>,
    name: String,
    controller_type: String,
    index: u32,
    controller: ControllerConfig,
) -> Result<(), VirtManagerError> {
    state
        .libvirt()
        .update_controller(&name, &controller_type, index, &controller)
}
