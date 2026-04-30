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

/// Replace a domain's persistent definition with the given XML. The
/// running VM is unaffected until the next start. Used by the raw-XML
/// editor.
#[tauri::command]
pub fn define_domain(state: State<'_, AppState>, xml: String) -> Result<(), VirtManagerError> {
    state.libvirt().define_domain_xml(&xml)
}

/// Get hotplug-relevant memory state: maxMemory config (if set) and
/// number of attached DIMM devices.
#[tauri::command]
pub fn get_memory_hotplug(
    state: State<'_, AppState>,
    name: String,
) -> Result<(Option<crate::libvirt::memory_hotplug::MaxMemoryConfig>, u32), VirtManagerError> {
    state.libvirt().get_memory_hotplug(&name)
}

/// Set the `<maxMemory slots>` element. Persistent only — VM reboot
/// required for the new slot count to take effect.
#[tauri::command]
pub fn set_max_memory_slots(
    state: State<'_, AppState>,
    name: String,
    max_mb: u64,
    slots: u32,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_max_memory_slots(&name, max_mb * 1024, slots)
}

/// Live-attach a DIMM device. Use `live=true, config=true` to grow the
/// running guest and persist for next boot.
#[tauri::command]
pub fn attach_memory_dimm(
    state: State<'_, AppState>,
    name: String,
    size_mb: u64,
    node: Option<u32>,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state
        .libvirt()
        .attach_memory_dimm(&name, size_mb * 1024, node, live, config)
}

/// Full-copy clone a shut-off VM. Each r/w disk volume is duplicated
/// in its own pool via virStorageVolCreateXMLFrom; CD-ROMs and
/// readonly/shareable disks pass through unchanged.
#[tauri::command]
pub fn clone_domain(
    state: State<'_, AppState>,
    source: String,
    target_name: String,
    randomize_macs: bool,
    start_after: bool,
) -> Result<String, VirtManagerError> {
    let opts = crate::libvirt::clone::CloneOptions {
        target_name,
        randomize_macs,
        start_after,
    };
    state.libvirt().clone_domain(&source, &opts)
}

/// Tail the qemu wrapper log for a domain. Reads over the active
/// connection's SSH target, or locally for `qemu:///system`.
#[tauri::command]
pub fn get_qemu_log(
    state: State<'_, AppState>,
    name: String,
    lines: u32,
) -> Result<String, VirtManagerError> {
    let uri = state
        .current_uri()
        .ok_or_else(|| VirtManagerError::OperationFailed {
            operation: "qemuLog".into(),
            reason: "no active connection".into(),
        })?;
    crate::libvirt::qemu_log::read_qemu_log(&uri, &name, lines)
}

/// Suspend the VM to libvirt-managed state. Next start resumes from it.
#[tauri::command]
pub fn managed_save_domain(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().managed_save(&name)
}

/// Whether the domain has a pending managed-save state.
#[tauri::command]
pub fn has_managed_save(state: State<'_, AppState>, name: String) -> Result<bool, VirtManagerError> {
    state.libvirt().has_managed_save(&name)
}

/// Discard the pending managed-save state without resuming.
#[tauri::command]
pub fn managed_save_remove(state: State<'_, AppState>, name: String) -> Result<(), VirtManagerError> {
    state.libvirt().managed_save_remove(&name)
}

/// Memory dump to a hypervisor-side file path.
#[tauri::command]
pub fn core_dump_domain(
    state: State<'_, AppState>,
    name: String,
    path: String,
    crash_after: bool,
    live: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().core_dump(&name, &path, crash_after, live)
}

/// Screenshot the guest console. Returns mime type + base64 PNG/PPM.
#[tauri::command]
pub fn screenshot_domain(
    state: State<'_, AppState>,
    name: String,
    screen: u32,
) -> Result<(String, String), VirtManagerError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let (mime, bytes) = state.libvirt().screenshot(&name, screen)?;
    Ok((mime, STANDARD.encode(bytes)))
}

// -- Backing chain (qcow2 overlays) --

#[tauri::command]
pub fn get_backing_chains(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<crate::libvirt::backing_chain::DiskBackingChain>, VirtManagerError> {
    state.libvirt().get_backing_chains(&name)
}

/// Flatten an overlay onto its disk image. Async — poll get_block_job
/// for progress.
#[tauri::command]
pub fn block_pull(
    state: State<'_, AppState>,
    name: String,
    disk: String,
    bandwidth_bps: u64,
) -> Result<(), VirtManagerError> {
    state.libvirt().block_pull(&name, &disk, bandwidth_bps)
}

/// Commit an overlay back into its parent. With `top` and `base` empty
/// strings the active overlay is committed into its immediate parent.
#[tauri::command]
pub fn block_commit(
    state: State<'_, AppState>,
    name: String,
    disk: String,
    top: Option<String>,
    base: Option<String>,
    bandwidth_bps: u64,
    active: bool,
    delete_after: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().block_commit(
        &name,
        &disk,
        top.as_deref().filter(|s| !s.is_empty()),
        base.as_deref().filter(|s| !s.is_empty()),
        bandwidth_bps,
        active,
        delete_after,
    )
}

#[tauri::command]
pub fn get_block_job(
    state: State<'_, AppState>,
    name: String,
    disk: String,
) -> Result<Option<crate::libvirt::backing_chain::BlockJobInfo>, VirtManagerError> {
    state.libvirt().get_block_job_info(&name, &disk)
}

#[tauri::command]
pub fn block_job_abort(
    state: State<'_, AppState>,
    name: String,
    disk: String,
    pivot: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().block_job_abort(&name, &disk, pivot)
}
