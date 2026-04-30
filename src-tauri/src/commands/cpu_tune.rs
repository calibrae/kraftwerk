//! Round I: advanced CPU / memory tuning + iothreads Tauri commands.

use tauri::State;
use crate::app_state::AppState;
use crate::libvirt::cpu_tune_config::{CpuTuneSnapshot, CpuTunePatch};
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn get_cpu_tune(
    state: State<'_, AppState>,
    name: String,
) -> Result<CpuTuneSnapshot, VirtManagerError> {
    state.libvirt().get_cpu_tune(&name)
}

#[tauri::command]
pub fn apply_cpu_tune(
    state: State<'_, AppState>,
    name: String,
    patch: CpuTunePatch,
) -> Result<(), VirtManagerError> {
    state.libvirt().apply_cpu_tune(&name, &patch)
}

#[tauri::command]
pub fn set_vcpu_count(
    state: State<'_, AppState>,
    name: String,
    current: u32,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_vcpu_count(&name, current, live, config)
}

#[tauri::command]
pub fn set_iothread_count(
    state: State<'_, AppState>,
    name: String,
    count: u32,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_iothread_count(&name, count)
}

#[tauri::command]
pub fn get_nested_virt_state(
    state: State<'_, AppState>,
    name: String,
) -> Result<crate::libvirt::nested_virt::NestedVirtState, VirtManagerError> {
    state.libvirt().get_nested_virt_state(&name)
}

#[tauri::command]
pub fn set_nested_virt(
    state: State<'_, AppState>,
    name: String,
    enable: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_nested_virt(&name, enable)
}
