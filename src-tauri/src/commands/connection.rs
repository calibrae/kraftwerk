use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::app_state::AppState;
use crate::models::connection::{AuthType, SavedConnection};
use crate::models::error::VirtManagerError;
use crate::models::state::ConnectionState;
use crate::models::vm::VmInfo;

/// Add a new saved connection.
#[tauri::command(async)]
pub fn add_connection(
    state: State<'_, AppState>,
    display_name: String,
    uri: String,
    auth_type: AuthType,
) -> Result<SavedConnection, VirtManagerError> {
    let conn = SavedConnection::new(display_name, uri, auth_type);
    state.add_saved_connection(conn.clone());
    Ok(conn)
}

/// Update the mutable fields (display name, URI, auth type) of a
/// saved connection. The UUID is preserved.
#[tauri::command(async)]
pub fn update_connection(
    state: State<'_, AppState>,
    id: String,
    display_name: String,
    uri: String,
    auth_type: AuthType,
) -> Result<SavedConnection, VirtManagerError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;
    if !state.update_saved_connection(&uuid, display_name, uri, auth_type) {
        return Err(VirtManagerError::ConnectionNotFound { id });
    }
    state
        .find_saved_connection(&uuid)
        .ok_or(VirtManagerError::ConnectionNotFound { id: uuid.to_string() })
}

/// Remove a saved connection by ID.
#[tauri::command(async)]
pub fn remove_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), VirtManagerError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;
    state.remove_saved_connection(&uuid);
    Ok(())
}

/// List all saved connections.
#[tauri::command(async)]
pub fn list_saved_connections(state: State<'_, AppState>) -> Vec<SavedConnection> {
    state.get_saved_connections()
}

/// Connect to a hypervisor by saved connection ID.
///
/// MUST be `async fn`. Tauri v2's `#[tauri::command]` runs sync `fn`
/// commands on the WebView's main thread; the synchronous libvirt
/// `Connect::open` blocks for the kernel TCP timeout (~75s) on
/// unreachable hosts, which would beachball the entire UI. Marking
/// the command async hands dispatch off to Tauri's tokio runtime
/// — the main thread stays free.
///
/// The libvirt RPC itself is still synchronous, so we further park
/// it on the blocking pool via `spawn_blocking` to keep the async
/// worker threads themselves free for concurrent invokes.
#[tauri::command(async)]
pub async fn connect(
    app: tauri::AppHandle,
    id: String,
) -> Result<Vec<VmInfo>, VirtManagerError> {
    use tauri::Manager;

    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;

    let conn = {
        let state = app.state::<AppState>();
        state.find_saved_connection(&uuid)
            .ok_or(VirtManagerError::ConnectionNotFound { id: id.clone() })?
    };

    {
        let state = app.state::<AppState>();
        state.set_connection_state(&uuid, ConnectionState::Connecting);
    }

    // Move the blocking open into the blocking pool. AppHandle is
    // cheaply cloneable; state is re-acquired inside the closure.
    let uri = conn.uri.clone();
    let app_for_blocking = app.clone();
    let open_result = tauri::async_runtime::spawn_blocking(move || {
        let state = app_for_blocking.state::<AppState>();
        state.open_connection(uuid, &uri)
    })
    .await
    .map_err(|e| VirtManagerError::OperationFailed {
        operation: "connectJoin".into(),
        reason: e.to_string(),
    })?;

    let state = app.state::<AppState>();
    match open_result {
        Ok(libvirt) => {
            state.set_current_uri(conn.uri.clone());
            state.set_connection_state(&uuid, ConnectionState::Connected);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            state.update_last_connected(&uuid, now);
            // list_all_domains is also blocking; same pattern.
            let app_for_list = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let st = app_for_list.state::<AppState>();
                let _ = st; // ensure state lives through the call
                libvirt.list_all_domains()
            })
            .await
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "listDomainsJoin".into(),
                reason: e.to_string(),
            })?
        }
        Err(e) => {
            state.set_connection_state(&uuid, ConnectionState::Error(e.to_string()));
            Err(e)
        }
    }
}

/// Disconnect from the current hypervisor.
#[tauri::command(async)]
pub fn disconnect(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), VirtManagerError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;
    state.set_connection_state(&uuid, ConnectionState::Disconnecting);
    state.close_connection(&uuid);
    state.clear_current_uri();
    state.set_connection_state(&uuid, ConnectionState::Disconnected);
    Ok(())
}

/// List the IDs of every connection currently open in the pool. The
/// frontend uses this to render which saved connections have a live
/// libvirt session and to enable multi-connection actions like live
/// migration target picking.
#[tauri::command(async)]
pub fn list_open_connections(state: State<'_, AppState>) -> Vec<String> {
    state
        .list_open_connections()
        .into_iter()
        .map(|u| u.to_string())
        .collect()
}

/// Switch which open connection is the "active" one without opening a
/// new session. Errors when the id isn't already in the pool.
#[tauri::command(async)]
pub fn set_active_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), VirtManagerError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;
    state.set_active_connection(uuid)?;
    if let Some(conn) = state.find_saved_connection(&uuid) {
        state.set_current_uri(conn.uri);
    }
    Ok(())
}

/// Get the connection state for a saved connection.
#[tauri::command(async)]
pub fn get_connection_state(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectionState, VirtManagerError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;
    Ok(state.get_connection_state(&uuid))
}

/// Probe a host's SSH key and compare against the local known_hosts.
/// Used by the connect flow to surface a TOFU prompt before handing
/// off to libvirt's ssh (which has no TTY for the standard prompt).
#[tauri::command(async)]
pub fn check_host_key(
    host: String,
    port: Option<u16>,
) -> Result<crate::libvirt::ssh_known_hosts::HostKeyInfo, VirtManagerError> {
    crate::libvirt::ssh_known_hosts::check_host_key(&host, port.unwrap_or(22))
}

/// Append a verbatim ssh-keyscan line to ~/.ssh/known_hosts. The line
/// MUST come from the previous check_host_key result — rejecting
/// multi-line input is a defense against a hostile webview snippet
/// trying to splice extra entries in.
#[tauri::command(async)]
pub fn accept_host_key(keyscan_line: String) -> Result<(), VirtManagerError> {
    crate::libvirt::ssh_known_hosts::append_host_key(&keyscan_line)
}

/// Remove all known_hosts entries for host[:port]. Required before
/// re-trusting a Changed-status host (otherwise libvirt's ssh refuses).
#[tauri::command(async)]
pub fn forget_host_key(
    host: String,
    port: Option<u16>,
) -> Result<(), VirtManagerError> {
    crate::libvirt::ssh_known_hosts::forget_host_key(&host, port.unwrap_or(22))
}
