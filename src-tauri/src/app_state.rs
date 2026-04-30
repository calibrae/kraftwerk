use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
/// Holds the libvirt connection pool, saved connections, per-connection
/// state, and the active console / VNC / SPICE sessions.
///
/// The connection pool is keyed by `SavedConnection` UUID. Currently we
/// keep "single active" UX semantics — connecting to a new id closes the
/// prior — but the multi-entry pool is the foundation for live migration
/// and side-by-side host views (phase 5.1).
pub struct AppState {
    /// All currently-open libvirt connections, keyed by saved-connection
    /// UUID. Each entry is shared via Arc so commands can hold references
    /// across mutex releases.
    connections: Mutex<HashMap<Uuid, Arc<LibvirtConnection>>>,
    /// The id of the currently-active connection (the one returned by
    /// `libvirt()`). `None` when nothing is connected.
    active_id: Mutex<Option<Uuid>>,
    /// A permanently-disconnected connection used as a fallback when
    /// callers ask for `libvirt()` while nothing is active. Its methods
    /// all return `NotConnected`. Avoids forcing every callsite to
    /// handle an Option.
    null_libvirt: Arc<LibvirtConnection>,
    saved_connections: Mutex<Vec<SavedConnection>>,
    connection_states: Mutex<HashMap<Uuid, ConnectionState>>,
    console: Mutex<Option<ConsoleSession>>,
    vnc: Mutex<Option<VncSession>>,
    spice: Mutex<Option<SpiceSession>>,
    runtime: tokio::runtime::Runtime,
    current_uri: Mutex<Option<String>>,
    /// When set, mutations to saved_connections are persisted to this file.
    persistence_path: Option<PathBuf>,
    /// Drained once by `start_event_loop` after Tauri setup.
    event_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<crate::libvirt::events::DomainEvent>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            active_id: Mutex::new(None),
            null_libvirt: Arc::new(LibvirtConnection::new()),
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
            event_rx: Mutex::new(None),
        }
    }

    /// Initialise libvirts default event loop and the per-process channel
    /// that lifecycle callbacks push to. Idempotent.
    pub fn init_events(&self) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // events::init_once is the actual gate; only the first call wins,
        // so subsequent AppState instances (e.g. test) don't relock.
        crate::libvirt::events::init_once(tx);
        let mut guard = self.event_rx.lock().unwrap();
        if guard.is_none() {
            *guard = Some(rx);
        }
    }

    /// Take the receiver out so the runtime can drain it.
    /// Returns None if events have never been initialised, or if a previous
    /// call already took the receiver.
    pub fn take_event_rx(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<crate::libvirt::events::DomainEvent>> {
        self.event_rx.lock().unwrap().take()
    }

    /// Borrow the runtime so external setup hooks can spawn the drain task.
    pub fn runtime(&self) -> &tokio::runtime::Runtime {
        &self.runtime
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
        state.init_events();
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

    /// Get the active libvirt connection. Returns a no-op disconnected
    /// connection (whose methods all return `NotConnected`) when no
    /// connection is active — preserves the pre-pool callsite ergonomics.
    pub fn libvirt(&self) -> Arc<LibvirtConnection> {
        let active = *self.active_id.lock().unwrap();
        if let Some(id) = active {
            if let Some(c) = self.connections.lock().unwrap().get(&id) {
                return Arc::clone(c);
            }
        }
        Arc::clone(&self.null_libvirt)
    }

    /// Get a connection by id (whether or not it's the active one).
    /// Used by migration commands that need to address source + target
    /// simultaneously.
    pub fn libvirt_for(&self, id: &Uuid) -> Option<Arc<LibvirtConnection>> {
        self.connections.lock().unwrap().get(id).cloned()
    }

    /// Open a connection for the given id. If an entry already exists in
    /// the pool, returns it (idempotent re-connect). Otherwise creates a
    /// new LibvirtConnection, opens it against `uri`, and stores it.
    /// Sets the active id on success.
    pub fn open_connection(&self, id: Uuid, uri: &str) -> Result<Arc<LibvirtConnection>, VirtManagerError> {
        let existing = self.connections.lock().unwrap().get(&id).cloned();
        let conn = match existing {
            Some(c) => c,
            None => {
                let c = Arc::new(LibvirtConnection::new());
                c.open(uri)?;
                self.connections.lock().unwrap().insert(id, Arc::clone(&c));
                c
            }
        };
        // Phase 5.1 step 1: keep single-active UX for now. A later step
        // will let multiple ids stay open simultaneously without each
        // connect() implicitly closing the prior.
        let prior = {
            let mut g = self.active_id.lock().unwrap();
            let prev = g.replace(id);
            prev
        };
        if let Some(prev) = prior {
            if prev != id {
                self.close_connection_internal(&prev);
            }
        }
        Ok(conn)
    }

    /// Close a specific connection by id. If it was the active one,
    /// clears `active_id`.
    pub fn close_connection(&self, id: &Uuid) {
        self.close_connection_internal(id);
        let mut g = self.active_id.lock().unwrap();
        if g.as_ref() == Some(id) {
            *g = None;
        }
    }

    fn close_connection_internal(&self, id: &Uuid) {
        let removed = self.connections.lock().unwrap().remove(id);
        if let Some(c) = removed {
            c.close();
        }
    }

    /// Currently-active connection id, if any.
    pub fn active_connection_id(&self) -> Option<Uuid> {
        *self.active_id.lock().unwrap()
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
        self.libvirt().with_console(domain_name, on_data)
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
