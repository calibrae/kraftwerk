use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::libvirt::connection::LibvirtConnection;
use crate::models::connection::SavedConnection;
use crate::models::state::ConnectionState;

/// Global application state, managed by Tauri.
///
/// Holds the libvirt connection, saved connections, and per-connection state.
/// Interior mutability via Mutex for thread-safe access from Tauri commands.
pub struct AppState {
    libvirt: LibvirtConnection,
    saved_connections: Mutex<Vec<SavedConnection>>,
    connection_states: Mutex<HashMap<Uuid, ConnectionState>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            libvirt: LibvirtConnection::new(),
            saved_connections: Mutex::new(Vec::new()),
            connection_states: Mutex::new(HashMap::new()),
        }
    }

    pub fn libvirt(&self) -> &LibvirtConnection {
        &self.libvirt
    }

    // -- Saved connections --

    pub fn add_saved_connection(&self, conn: SavedConnection) {
        self.saved_connections.lock().unwrap().push(conn);
    }

    pub fn remove_saved_connection(&self, id: &Uuid) {
        self.saved_connections
            .lock()
            .unwrap()
            .retain(|c| c.id != *id);
        self.connection_states.lock().unwrap().remove(id);
    }

    pub fn get_saved_connections(&self) -> Vec<SavedConnection> {
        self.saved_connections.lock().unwrap().clone()
    }

    pub fn find_saved_connection(&self, id: &Uuid) -> Option<SavedConnection> {
        self.saved_connections
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.id == *id)
            .cloned()
    }

    pub fn update_last_connected(&self, id: &Uuid, timestamp: i64) {
        if let Some(conn) = self
            .saved_connections
            .lock()
            .unwrap()
            .iter_mut()
            .find(|c| c.id == *id)
        {
            conn.last_connected = Some(timestamp);
        }
    }

    // -- Connection states --

    pub fn set_connection_state(&self, id: &Uuid, state: ConnectionState) {
        self.connection_states
            .lock()
            .unwrap()
            .insert(*id, state);
    }

    pub fn get_connection_state(&self, id: &Uuid) -> ConnectionState {
        self.connection_states
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or(ConnectionState::Disconnected)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::connection::AuthType;

    #[test]
    fn add_and_find_connection() {
        let state = AppState::new();
        let conn = SavedConnection::new("test".into(), "qemu:///system".into(), AuthType::SshAgent);
        let id = conn.id;
        state.add_saved_connection(conn);

        let found = state.find_saved_connection(&id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().display_name, "test");
    }

    #[test]
    fn remove_connection() {
        let state = AppState::new();
        let conn = SavedConnection::new("rm".into(), "qemu:///system".into(), AuthType::SshAgent);
        let id = conn.id;
        state.add_saved_connection(conn);
        state.remove_saved_connection(&id);
        assert!(state.find_saved_connection(&id).is_none());
    }

    #[test]
    fn connection_state_defaults_to_disconnected() {
        let state = AppState::new();
        let id = Uuid::new_v4();
        assert_eq!(state.get_connection_state(&id), ConnectionState::Disconnected);
    }

    #[test]
    fn update_connection_state() {
        let state = AppState::new();
        let id = Uuid::new_v4();
        state.set_connection_state(&id, ConnectionState::Connected);
        assert_eq!(state.get_connection_state(&id), ConnectionState::Connected);
    }

    #[test]
    fn update_last_connected() {
        let state = AppState::new();
        let conn = SavedConnection::new("ts".into(), "qemu:///system".into(), AuthType::SshAgent);
        let id = conn.id;
        state.add_saved_connection(conn);
        state.update_last_connected(&id, 1234567890);
        let found = state.find_saved_connection(&id).unwrap();
        assert_eq!(found.last_connected, Some(1234567890));
    }
}
