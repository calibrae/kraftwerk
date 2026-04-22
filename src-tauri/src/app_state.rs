use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use uuid::Uuid;

use crate::libvirt::connection::LibvirtConnection;
use crate::libvirt::console::ConsoleSession;
use crate::libvirt::vnc_proxy::VncSession;
use crate::libvirt::spice_proxy::SpiceSession;
use capsaicin_client::InputEvent as SpiceInput;
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
    vnc: Mutex<Option<VncSession>>,
    spice: Mutex<Option<SpiceSession>>,
    runtime: tokio::runtime::Runtime,
    current_uri: Mutex<Option<String>>,
    /// When set, mutations to saved_connections are persisted to this file.
    persistence_path: Option<PathBuf>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            libvirt: LibvirtConnection::new(),
            saved_connections: Mutex::new(Vec::new()),
            connection_states: Mutex::new(HashMap::new()),
            console: Mutex::new(None),
            vnc: Mutex::new(None),
            spice: Mutex::new(None),
            current_uri: Mutex::new(None),
            runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("virtmanager-net")
                .build()
                .expect("tokio runtime"),
            persistence_path: None,
        }
    }

    /// Build an AppState that persists saved_connections to \`path\`.
    /// If the file exists it is loaded into memory at construction.
    pub fn with_persistence(path: PathBuf) -> Self {
        let mut state = Self::new();
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(conns) = serde_json::from_slice::<Vec<SavedConnection>>(&bytes) {
                *state.saved_connections.lock().unwrap() = conns;
                log::info!("Loaded {} saved connections from {}", state.saved_connections.lock().unwrap().len(), path.display());
            } else {
                log::warn!("Could not deserialize saved connections from {}", path.display());
            }
        }
        state.persistence_path = Some(path);
        state
    }

    fn persist_connections(&self) {
        let Some(path) = self.persistence_path.as_ref() else { return };
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                log::warn!("Could not create parent dir for {}: {}", path.display(), e);
                return;
            }
        }
        let conns = self.saved_connections.lock().unwrap().clone();
        match serde_json::to_vec_pretty(&conns) {
            Ok(bytes) => {
                if let Err(e) = fs::write(path, bytes) {
                    log::warn!("Could not write saved connections to {}: {}", path.display(), e);
                }
            }
            Err(e) => log::warn!("Could not serialize saved connections: {}", e),
        }
    }

    pub fn libvirt(&self) -> &LibvirtConnection {
        &self.libvirt
    }

    // -- Saved connections --

    pub fn add_saved_connection(&self, conn: SavedConnection) {
        self.saved_connections.lock().unwrap().push(conn);
        self.persist_connections();
    }

    pub fn remove_saved_connection(&self, id: &Uuid) {
        self.saved_connections
            .lock()
            .unwrap()
            .retain(|c| c.id != *id);
        self.connection_states.lock().unwrap().remove(id);
        self.persist_connections();
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

    /// Update a saved connections mutable fields. Returns true if found.
    pub fn update_saved_connection(
        &self,
        id: &Uuid,
        display_name: String,
        uri: String,
        auth_type: crate::models::connection::AuthType,
    ) -> bool {
        let mut changed = false;
        {
            let mut guard = self.saved_connections.lock().unwrap();
            if let Some(conn) = guard.iter_mut().find(|c| c.id == *id) {
                conn.display_name = display_name;
                conn.uri = uri;
                conn.auth_type = auth_type;
                changed = true;
            }
        }
        if changed {
            self.persist_connections();
        }
        changed
    }

    pub fn update_last_connected(&self, id: &Uuid, timestamp: i64) {
        let mut changed = false;
        {
            let mut guard = self.saved_connections.lock().unwrap();
            if let Some(conn) = guard.iter_mut().find(|c| c.id == *id) {
                conn.last_connected = Some(timestamp);
                changed = true;
            }
        }
        if changed {
            self.persist_connections();
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



    pub fn set_current_uri(&self, uri: String) {
        *self.current_uri.lock().unwrap() = Some(uri);
    }

    pub fn clear_current_uri(&self) {
        *self.current_uri.lock().unwrap() = None;
    }

    pub fn current_uri(&self) -> Option<String> {
        self.current_uri.lock().unwrap().clone()
    }

    pub fn runtime_handle(&self) -> &tokio::runtime::Handle {
        self.runtime.handle()
    }

    pub fn set_vnc(&self, session: VncSession) {
        let mut guard = self.vnc.lock().unwrap();
        if let Some(old) = guard.take() {
            old.close();
        }
        *guard = Some(session);
    }


    pub fn set_spice(&self, session: SpiceSession) {
        let mut guard = self.spice.lock().unwrap();
        if let Some(mut old) = guard.take() {
            old.close();
        }
        *guard = Some(session);
    }


    /// Clone of the active SPICE input sender. Returns None when there's
    /// no session. Callers should prefer this over `spice_send_input` so
    /// they can drop the AppState mutex before awaiting.
    pub fn spice_sender(&self) -> Option<tokio::sync::mpsc::Sender<SpiceInput>> {
        let guard = self.spice.lock().unwrap();
        guard.as_ref().map(|s| s.input_tx.clone())
    }

    pub fn close_spice(&self) {
        let mut guard = self.spice.lock().unwrap();
        if let Some(mut session) = guard.take() {
            session.close();
        }
    }

    /// Push an input event to the active SPICE session.
    pub fn spice_send_input(&self, event: SpiceInput) -> std::io::Result<()> {
        let guard = self.spice.lock().unwrap();
        match guard.as_ref() {
            Some(session) => session.input_tx.try_send(event).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "SPICE session closed")
            }),
            None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no active SPICE session")),
        }
    }

    pub fn close_vnc(&self) {
        let mut guard = self.vnc.lock().unwrap();
        if let Some(session) = guard.take() {
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


#[cfg(test)]
mod persistence_tests {
    use super::*;
    use crate::models::connection::AuthType;

    #[test]
    fn round_trips_connections_across_new_instance() {
        let tmp = std::env::temp_dir().join(format!("kraftwerk-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&tmp);

        let s1 = AppState::with_persistence(tmp.clone());
        let a = SavedConnection::new("h1".into(), "qemu:///system".into(), AuthType::SshAgent);
        let b = SavedConnection::new("h2".into(), "qemu:///system".into(), AuthType::Password);
        s1.add_saved_connection(a.clone());
        s1.add_saved_connection(b.clone());
        drop(s1);

        let s2 = AppState::with_persistence(tmp.clone());
        let loaded = s2.get_saved_connections();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|c| c.id == a.id));
        assert!(loaded.iter().any(|c| c.id == b.id));

        s2.remove_saved_connection(&a.id);
        drop(s2);

        let s3 = AppState::with_persistence(tmp.clone());
        assert_eq!(s3.get_saved_connections().len(), 1);

        let _ = std::fs::remove_file(&tmp);
    }
}
