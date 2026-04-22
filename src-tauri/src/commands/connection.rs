use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::app_state::AppState;
use crate::models::connection::{AuthType, SavedConnection};
use crate::models::error::VirtManagerError;
use crate::models::state::ConnectionState;
use crate::models::vm::VmInfo;

/// Add a new saved connection.
#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
pub fn list_saved_connections(state: State<'_, AppState>) -> Vec<SavedConnection> {
    state.get_saved_connections()
}

/// Connect to a hypervisor by saved connection ID.
#[tauri::command]
pub fn connect(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<VmInfo>, VirtManagerError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;

    let conn = state
        .find_saved_connection(&uuid)
        .ok_or(VirtManagerError::ConnectionNotFound { id: id.clone() })?;

    state.set_connection_state(&uuid, ConnectionState::Connecting);

    match state.libvirt().open(&conn.uri) {
        Ok(()) => {
            state.set_current_uri(conn.uri.clone());
            state.set_connection_state(&uuid, ConnectionState::Connected);
            // Update last connected timestamp
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            state.update_last_connected(&uuid, now);
            // Return VM list immediately
            state.libvirt().list_all_domains()
        }
        Err(e) => {
            state.set_connection_state(&uuid, ConnectionState::Error(e.to_string()));
            Err(e)
        }
    }
}

/// Disconnect from the current hypervisor.
#[tauri::command]
pub fn disconnect(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), VirtManagerError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;
    state.set_connection_state(&uuid, ConnectionState::Disconnecting);
    state.libvirt().close();
    state.clear_current_uri();
    state.set_connection_state(&uuid, ConnectionState::Disconnected);
    Ok(())
}

/// Get the connection state for a saved connection.
#[tauri::command]
pub fn get_connection_state(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectionState, VirtManagerError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| VirtManagerError::ConnectionNotFound {
        id: id.clone(),
    })?;
    Ok(state.get_connection_state(&uuid))
}
