//! Tauri command surface for host device passthrough.

use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::hostdev::{HostDevice, HostPciDevice, HostUsbDevice};
use crate::models::error::VirtManagerError;

/// All PCI devices on the hypervisor host.
#[tauri::command]
pub fn list_host_pci_devices(
    state: State<'_, AppState>,
) -> Result<Vec<HostPciDevice>, VirtManagerError> {
    state.libvirt().list_host_pci_devices()
}

/// All USB devices on the hypervisor host.
#[tauri::command]
pub fn list_host_usb_devices(
    state: State<'_, AppState>,
) -> Result<Vec<HostUsbDevice>, VirtManagerError> {
    state.libvirt().list_host_usb_devices()
}

/// The hostdev entries already attached to a given domain.
#[tauri::command]
pub fn list_domain_hostdevs(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<HostDevice>, VirtManagerError> {
    state.libvirt().list_domain_hostdevs(&name)
}

#[tauri::command]
pub fn attach_hostdev(
    state: State<'_, AppState>,
    name: String,
    dev: HostDevice,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().attach_hostdev(&name, &dev, live, config)
}

#[tauri::command]
pub fn detach_hostdev(
    state: State<'_, AppState>,
    name: String,
    dev: HostDevice,
    live: bool,
    config: bool,
) -> Result<(), VirtManagerError> {
    state.libvirt().detach_hostdev(&name, &dev, live, config)
}
