//! Tauri command surface for filesystem passthrough + shared memory.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::filesystem_config::{FilesystemConfig, ShmemConfig};
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn list_filesystems(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<FilesystemConfig>, VirtManagerError> {
    state.libvirt().list_filesystems(&name)
}

#[tauri::command]
pub fn add_filesystem(
    state: State<'_, AppState>,
    name: String,
    fs: FilesystemConfig,
    force_memory_backing: bool,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_filesystem(&name, &fs, force_memory_backing, live, config)
}

#[tauri::command]
pub fn remove_filesystem(
    state: State<'_, AppState>,
    name: String,
    target_dir: String,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().remove_filesystem(&name, &target_dir, live, config)
}

#[tauri::command]
pub fn update_filesystem(
    state: State<'_, AppState>,
    name: String,
    fs: FilesystemConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().update_filesystem(&name, &fs, live, config)
}

#[tauri::command]
pub fn list_shmems(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<ShmemConfig>, VirtManagerError> {
    state.libvirt().list_shmems(&name)
}

#[tauri::command]
pub fn add_shmem(
    state: State<'_, AppState>,
    name: String,
    shmem: ShmemConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_shmem(&name, &shmem, live, config)
}

#[tauri::command]
pub fn remove_shmem(
    state: State<'_, AppState>,
    name: String,
    shmem_name: String,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().remove_shmem(&name, &shmem_name, live, config)
}

#[tauri::command]
pub fn enable_shared_memory_backing(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), VirtManagerError> {
    state.libvirt().enable_shared_memory_backing(&name)
}
