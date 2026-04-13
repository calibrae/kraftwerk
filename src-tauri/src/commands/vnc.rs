use tauri::State;

use crate::app_state::AppState;
use crate::libvirt::vnc_proxy::{self, VncSession};
use crate::models::error::VirtManagerError;

/// Open a VNC proxy for a VM. Returns the local WebSocket port.
/// Tunnels via SSH when the libvirt connection is qemu+ssh://.
#[tauri::command]
pub fn open_vnc(state: State<'_, AppState>, name: String) -> Result<u16, VirtManagerError> {
    // Close prior session
    state.close_vnc();

    // Get the VM's VNC port from its XML
    let xml = state.libvirt().get_domain_xml(&name, false)?;
    let (listen, port) = vnc_proxy::parse_vnc_endpoint(&xml).ok_or_else(|| {
        VirtManagerError::OperationFailed {
            operation: "parseVncEndpoint".into(),
            reason: "VM has no active VNC graphics port (autoport not resolved or SPICE-only)".into(),
        }
    })?;

    // Determine SSH target from the current libvirt URI
    let uri = state.current_uri().ok_or(VirtManagerError::NotConnected)?;
    let ssh_target = vnc_proxy::parse_ssh_target(&uri).ok_or_else(|| {
        VirtManagerError::OperationFailed {
            operation: "parseUri".into(),
            reason: format!("VNC requires qemu+ssh:// URI; got: {uri}"),
        }
    })?;

    let session = VncSession::start(&ssh_target, &listen, port, state.runtime_handle())?;
    let ws_port = session.port;
    state.set_vnc(session);
    Ok(ws_port)
}

#[tauri::command]
pub fn close_vnc(state: State<'_, AppState>) {
    state.close_vnc();
}
