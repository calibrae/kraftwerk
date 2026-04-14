//! Tauri commands for Round F char devices.

use tauri::State;
use crate::app_state::AppState;
use crate::libvirt::char_devices::{
    CharDevicesSnapshot, ChannelConfig, SerialConfig,
};
use crate::models::error::VirtManagerError;

#[tauri::command]
pub fn get_char_devices(
    state: State<'_, AppState>,
    name: String,
) -> Result<CharDevicesSnapshot, VirtManagerError> {
    state.libvirt().get_char_devices(&name)
}

#[tauri::command]
pub fn add_channel(
    state: State<'_, AppState>,
    name: String,
    channel: ChannelConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_channel(&name, &channel, live, config)
}

#[tauri::command]
pub fn remove_channel(
    state: State<'_, AppState>,
    name: String,
    target_name: String,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().remove_channel(&name, &target_name, live, config)
}

#[tauri::command]
pub fn add_serial(
    state: State<'_, AppState>,
    name: String,
    serial: SerialConfig,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_serial(&name, &serial, live, config)
}

#[tauri::command]
pub fn remove_serial(
    state: State<'_, AppState>,
    name: String,
    port: u32,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().remove_serial(&name, port, live, config)
}

#[tauri::command]
pub fn add_guest_agent_channel(
    state: State<'_, AppState>,
    name: String,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_guest_agent_channel(&name, live, config)
}

#[tauri::command]
pub fn add_spice_vdagent_channel(
    state: State<'_, AppState>,
    name: String,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().add_spice_vdagent_channel(&name, live, config)
}
