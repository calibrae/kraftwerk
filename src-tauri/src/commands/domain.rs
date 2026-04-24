use tauri::State;

use crate::app_state::AppState;
use crate::models::error::VirtManagerError;
use crate::models::vm::VmInfo;

/// List all VMs on the connected hypervisor.
#[tauri::command]
pub fn list_domains(state: State<'_, AppState>) -> Result<Vec<VmInfo>, VirtManagerError> {
    state.libvirt().list_all_domains()
}

/// Start a VM by name.
#[tauri::command]
pub fn start_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().start_domain(&name)
}

/// Gracefully shutdown a VM.
#[tauri::command]
pub fn shutdown_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().shutdown_domain(&name)
}

/// Force stop a VM.
#[tauri::command]
pub fn destroy_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().destroy_domain(&name)
}

/// Suspend a VM.
#[tauri::command]
pub fn suspend_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().suspend_domain(&name)
}

/// Resume a paused VM.
#[tauri::command]
pub fn resume_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().resume_domain(&name)
}

/// Reboot a VM.
#[tauri::command]
pub fn reboot_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().reboot_domain(&name)
}

/// Get the XML description for a VM.
#[tauri::command]
pub fn get_domain_xml(
    state: State<'_, AppState>,
    name: String,
    inactive: bool,
) -> Result<String, VirtManagerError> {
    state.libvirt().get_domain_xml(&name, inactive)
}

/// Get parsed domain configuration for a VM.
#[tauri::command]
pub fn get_domain_config(
    state: State<'_, AppState>,
    name: String,
    inactive: bool,
) -> Result<crate::libvirt::domain_config::DomainConfig, VirtManagerError> {
    state.libvirt().get_domain_config(&name, inactive)
}

/// Set the vCPU count for a VM. `live=true` affects running VM, `config=true` persists.
#[tauri::command]
pub fn set_vcpus(
    state: State<'_, AppState>,
    name: String,
    count: u32,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_vcpus(&name, count, live, config)
}

/// Set memory (in MiB) for a VM.
#[tauri::command]
pub fn set_memory_mb(
    state: State<'_, AppState>,
    name: String,
    memory_mb: u64,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_memory(&name, memory_mb * 1024, live, config)
}


/// Set the **maximum (boot-time) memory** (in MiB) for a VM.
/// Only applies to the persistent config. Usually requires the VM to
/// be shut off for the change to take effect on next boot.
#[tauri::command]
pub fn set_max_memory_mb(
    state: State<'_, AppState>,
    name: String,
    memory_mb: u64,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_max_memory(&name, memory_mb * 1024)
}

/// Set the **maximum (boot-time) vCPU count** for a VM.
/// Only applies to the persistent config. Usually requires the VM to
/// be shut off for the change to take effect on next boot.
#[tauri::command]
pub fn set_max_vcpus_count(
    state: State<'_, AppState>,
    name: String,
    count: u32,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_max_vcpus(&name, count)
}

/// Remove a VM's persistent configuration. VM must be shut off.
#[tauri::command]
pub fn undefine_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().undefine_domain(&name)
}


/// Sample live CPU/memory/disk/network stats for a domain.
#[tauri::command]
pub fn get_domain_stats(
    state: State<'_, AppState>,
    name: String,
) -> Result<crate::libvirt::domain_stats::DomainStatsSample, VirtManagerError> {
    state.libvirt().sample_domain_stats(&name)
}
