use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::libvirt::connection::LibvirtConnection;
use crate::libvirt::console::ConsoleSession;
use crate::models::connection::SavedConnection;
use crate::models::error::VirtManagerError;
use crate::models::state::ConnectionState;

/// Global application state, managed by Tauri.
///
/// Holds the libvirt connection, saved connections, per-connection state,
/// and the active console session.
pub struct AppState {
    libvirt: LibvirtConnection,
    saved_connections: Mutex<Vec<SavedConnection>>,
    connection_states: Mutex<HashMap<Uuid, ConnectionState>>,
    console: Mutex<Option<ConsoleSession>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            libvirt: LibvirtConnection::new(),
            saved_connections: Mutex::new(Vec::new()),
            connection_states: Mutex::new(HashMap::new()),
            console: Mutex::new(None),
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

    // -- Console session --

    /// Open a console session, using the current libvirt connection.
    pub fn open_console<F>(
        &self,
        domain_name: &str,
        on_data: F,
    ) -> Result<ConsoleSession, VirtManagerError>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        self.libvirt.with_console(domain_name, on_data)
    }

    pub fn set_console(&self, session: ConsoleSession) {
        let mut guard = self.console.lock().unwrap();
        // Close existing session if any
        if let Some(mut old) = guard.take() {
            old.close();
        }
        *guard = Some(session);
    }

    pub fn console_send(&self, data: &[u8]) -> Result<(), VirtManagerError> {
        let guard = self.console.lock().unwrap();
        match guard.as_ref() {
            Some(session) => {
                session.send(data)?;
                Ok(())
            }
            None => Err(VirtManagerError::OperationFailed {
                operation: "consoleSend".into(),
                reason: "No active console session".into(),
            }),
        }
    }

    pub fn console_is_active(&self) -> bool {
        self.console
            .lock()
            .unwrap()
            .as_ref()
            .map_or(false, |s| s.is_active())
    }

    pub fn close_console(&self) {
        let mut guard = self.console.lock().unwrap();
        if let Some(mut session) = guard.take() {
            session.close();
        }
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

    #[test]
    fn console_not_active_by_default() {
        let state = AppState::new();
        assert!(!state.console_is_active());
    }

    #[test]
    fn console_send_without_session_errors() {
        let state = AppState::new();
        let result = state.console_send(b"hello");
        assert!(result.is_err());
    }
}
