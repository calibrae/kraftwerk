//! Tauri commands for VM snapshots (list / create / revert / delete).

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::snapshots::SnapshotInfo;
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn list_snapshots(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<SnapshotInfo>, VirtManagerError> {
    state.libvirt().list_snapshots(&name)
}

#[tauri::command]
pub fn create_snapshot(
    state: State<'_, AppState>,
    name: String,
    snap_name: String,
    description: Option<String>,
    flags: u32,
) -> Result<SnapshotInfo, VirtManagerError> {
    state
        .libvirt()
        .create_snapshot(&name, &snap_name, description.as_deref(), flags)
}

#[tauri::command]
pub fn revert_snapshot(
    state: State<'_, AppState>,
    name: String,
    snap_name: String,
    flags: u32,
) -> Result<(), VirtManagerError> {
    state.libvirt().revert_snapshot(&name, &snap_name, flags)
}

#[tauri::command]
pub fn delete_snapshot(
    state: State<'_, AppState>,
    name: String,
    snap_name: String,
    flags: u32,
) -> Result<(), VirtManagerError> {
    state.libvirt().delete_snapshot(&name, &snap_name, flags)
}
