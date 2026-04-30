//! Tauri commands for launch-security (SEV / SEV-SNP / TDX).

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::launch_security::LaunchSecurityConfig;
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn get_launch_security(
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<LaunchSecurityConfig>, VirtManagerError> {
    state.libvirt().get_launch_security(&name)
}

#[tauri::command]
pub fn set_launch_security(
    state: State<'_, AppState>,
    name: String,
    cfg: Option<LaunchSecurityConfig>,
) -> Result<(), VirtManagerError> {
    state.libvirt().set_launch_security(&name, cfg.as_ref())
}
