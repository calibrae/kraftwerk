//! Tauri commands for Round E virtio-adjacent devices.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::virtio_devices::{
    BalloonConfig, IommuConfig, PanicConfig, RngConfig, TpmConfig, VirtioDevicesSnapshot,
    VsockConfig, WatchdogConfig,
};
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn get_virtio_devices(
    state: State<'_, AppState>,
    name: String,
) -> Result<VirtioDevicesSnapshot, VirtManagerError> {
    state.libvirt().get_virtio_devices(&name)
}

#[tauri::command]
pub fn get_vtpm_info(
    state: State<'_, AppState>,
    name: String,
) -> Result<crate::libvirt::vtpm::VtpmInfo, VirtManagerError> {
    state.libvirt().get_vtpm_info(&name)
}

#[tauri::command]
pub fn set_tpm(
    state: State<'_, AppState>,
    name: String,
    cfg: Option<TpmConfig>,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_tpm(&name, cfg.as_ref(), false, true)
}

#[tauri::command]
pub fn set_watchdog(
    state: State<'_, AppState>,
    name: String,
    cfg: Option<WatchdogConfig>,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_watchdog(&name, cfg.as_ref(), false, true)
}

#[tauri::command]
pub fn set_panic(
    state: State<'_, AppState>,
    name: String,
    cfg: Option<PanicConfig>,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_panic(&name, cfg.as_ref(), false, true)
}

#[tauri::command]
pub fn set_balloon(
    state: State<'_, AppState>,
    name: String,
    cfg: Option<BalloonConfig>,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_balloon(&name, cfg.as_ref(), live, config)
}

#[tauri::command]
pub fn set_vsock(
    state: State<'_, AppState>,
    name: String,
    cfg: Option<VsockConfig>,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_vsock(&name, cfg.as_ref(), live, config)
}

#[tauri::command]
pub fn add_rng(
    state: State<'_, AppState>,
    name: String,
    cfg: RngConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_rng(&name, &cfg, live, config)
}

#[tauri::command]
pub fn remove_rng(
    state: State<'_, AppState>,
    name: String,
    cfg: RngConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().remove_rng(&name, &cfg, live, config)
}

#[tauri::command]
pub fn update_rng(
    state: State<'_, AppState>,
    name: String,
    cfg: RngConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().update_rng(&name, &cfg, live, config)
}

#[tauri::command]
pub fn set_iommu(
    state: State<'_, AppState>,
    name: String,
    cfg: Option<IommuConfig>,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_iommu(&name, cfg.as_ref(), false, true)
}
