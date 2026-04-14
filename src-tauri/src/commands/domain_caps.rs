use tauri::State;
use crate::app_state::AppState;
use crate::libvirt::domain_caps::DomainCaps;
use crate::models::error::VirtManagerError;

/// Return host domain capabilities, optionally constrained to a specific
/// (emulator, arch, machine, virttype). Passing `None` for any field lets
/// libvirt pick sensible defaults.
#[tauri::command]
pub fn get_domain_capabilities(
    state: State<'_, AppState>,
    emulator: Option<String>,
    arch: Option<String>,
    machine: Option<String>,
    virttype: Option<String>,
) -> Result<DomainCaps, VirtManagerError> {
    state.libvirt().get_domain_capabilities(
        emulator.as_deref(), arch.as_deref(),
        machine.as_deref(), virttype.as_deref(),
    )
}
