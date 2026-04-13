use serde::Deserialize;
use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::storage_config::{self, PoolBuildParams, VolumeBuildParams};
use crate::models::error::VirtManagerError;
use crate::models::storage::{StoragePoolInfo, StorageVolumeInfo};

#[derive(Debug, Deserialize)]
pub struct CreatePoolRequest {
    pub name: String,
    /// "dir" | "netfs" | "logical" | "iscsi"
    pub pool_type: String,
    pub target_path: Option<String>,
    pub source_host: Option<String>,
    pub source_dir: Option<String>,
    pub source_name: Option<String>,
    #[serde(default = "default_true")]
    pub build: bool,
    #[serde(default = "default_true")]
    pub start: bool,
    #[serde(default)]
    pub autostart: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateVolumeRequest {
    pub pool_name: String,
    pub name: String,
    pub capacity_bytes: u64,
    /// "qcow2" | "raw" | "iso"
    pub format: String,
    pub allocation_bytes: Option<u64>,
}

fn default_true() -> bool { true }

// ── Pool commands ──

#[tauri::command]
pub fn list_storage_pools(state: State<'_, AppState>) -> Result<Vec<StoragePoolInfo>, VirtManagerError> {
    state.libvirt().list_storage_pools()
}

#[tauri::command]
pub fn get_pool_xml(state: State<'_, AppState>, name: String) -> Result<String, VirtManagerError> {
    state.libvirt().get_pool_xml(&name)
}

#[tauri::command]
pub fn get_pool_config(
    state: State<'_, AppState>,
    name: String,
) -> Result<storage_config::PoolConfig, VirtManagerError> {
    state.libvirt().get_pool_config(&name)
}

#[tauri::command]
pub fn start_pool(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().start_pool(&name)
}

#[tauri::command]
pub fn stop_pool(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().stop_pool(&name)
}

#[tauri::command]
pub fn refresh_pool(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().refresh_pool(&name)
}

#[tauri::command]
pub fn delete_pool(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().delete_pool(&name)
}

#[tauri::command]
pub fn set_pool_autostart(
    state: State<'_, AppState>,
    name: String,
    autostart: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_pool_autostart(&name, autostart)
}

#[tauri::command]
pub fn create_pool(
    state: State<'_, AppState>,
    req: CreatePoolRequest,
) -> Result<(), VirtManagerError> {
    let xml = storage_config::build_pool_xml(&PoolBuildParams {
        name: &req.name,
        pool_type: &req.pool_type,
        target_path: req.target_path.as_deref(),
        source_host: req.source_host.as_deref(),
        source_dir: req.source_dir.as_deref(),
        source_name: req.source_name.as_deref(),
    });
    state.libvirt().define_pool(&xml, req.build, req.start)?;
    if req.autostart {
        let _ = state.libvirt().set_pool_autostart(&req.name, true);
    }
    Ok(())
}

// ── Volume commands ──

#[tauri::command]
pub fn list_volumes(
    state: State<'_, AppState>,
    pool_name: String,
) -> Result<Vec<StorageVolumeInfo>, VirtManagerError> {
    state.libvirt().list_volumes(&pool_name)
}

#[tauri::command]
pub fn create_volume(
    state: State<'_, AppState>,
    req: CreateVolumeRequest,
) -> Result<String, VirtManagerError> {
    let xml = storage_config::build_volume_xml(&VolumeBuildParams {
        name: &req.name,
        capacity_bytes: req.capacity_bytes,
        format: &req.format,
        allocation_bytes: req.allocation_bytes,
    });
    state.libvirt().create_volume(&req.pool_name, &xml)
}

#[tauri::command]
pub fn delete_volume(state: State<'_, AppState>, path: String) -> Result<(), VirtManagerError> {
    state.libvirt().delete_volume(&path)
}

#[tauri::command]
pub fn resize_volume(
    state: State<'_, AppState>,
    path: String,
    capacity_bytes: u64,
) -> Result<(), VirtManagerError> {
    state.libvirt().resize_volume(&path, capacity_bytes)
}
