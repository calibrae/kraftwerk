//! Tauri commands for VM templates + clone-from-template.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::clone::CloneOptions;
use crate::libvirt::templates::CloudInitConfig;
use crate::models::error::VirtManagerError;
use crate::models::vm::VmInfo;

/// Mark / unmark a domain as a kraftwerk template.
#[tauri::command]
pub fn set_template_flag(
    state: State<'_, AppState>,
    name: String,
    mark: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_template_flag(&name, mark)
}

/// List domains marked as templates.
#[tauri::command]
pub fn list_templates(state: State<'_, AppState>) -> Result<Vec<VmInfo>, VirtManagerError> {
    state.libvirt().list_templates()
}

/// Clone a template into a new domain. Optional cloud-init seed is
/// generated as a NoCloud ISO on the hypervisor host and attached as
/// a CD-ROM on the new VM.
#[tauri::command]
pub fn clone_from_template(
    state: State<'_, AppState>,
    template_name: String,
    options: CloneOptions,
    cloud_init: Option<CloudInitConfig>,
) -> Result<String, VirtManagerError> {
    state.libvirt().clone_from_template(&template_name, &options, cloud_init.as_ref())
}
