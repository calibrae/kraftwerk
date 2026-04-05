use std::sync::Mutex;
use virt::connect::Connect;
use virt::domain::Domain;

use crate::libvirt::console::ConsoleSession;
use crate::models::error::VirtManagerError;
use crate::models::vm::{GraphicsType, VmInfo, VmState};
use crate::libvirt::xml_helpers;

/// Thread-safe wrapper around a libvirt connection.
///
/// All libvirt operations are blocking. The Mutex ensures only one
/// thread accesses the connection at a time. Tauri commands should
/// call these methods from async handlers — Tauri handles the
/// blocking-to-async bridge.
pub struct LibvirtConnection {
    inner: Mutex<Option<Connect>>,
}

impl LibvirtConnection {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Open a connection to the given libvirt URI. Blocking.
    pub fn open(&self, uri: &str) -> Result<(), VirtManagerError> {
        log::info!("Opening connection to {uri}");
        let conn = Connect::open(Some(uri)).map_err(|e| VirtManagerError::ConnectionFailed {
            host: uri.to_string(),
            reason: e.to_string(),
        })?;
        let mut guard = self.inner.lock().unwrap();
        if let Some(mut old) = guard.take() {
            let _ = old.close();
        }
        *guard = Some(conn);
        log::info!("Connected successfully to {uri}");
        Ok(())
    }

    /// Close the connection.
    pub fn close(&self) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(mut conn) = guard.take() {
            log::info!("Closing connection");
            let _ = conn.close();
        }
    }

    pub fn is_connected(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        guard.is_some()
    }

    /// Get the hypervisor hostname.
    pub fn hostname(&self) -> Result<String, VirtManagerError> {
        self.with_connection(|conn| {
            conn.get_hostname().map_err(|e| VirtManagerError::OperationFailed {
                operation: "getHostname".into(),
                reason: e.to_string(),
            })
        })
    }

    /// List all domains (VMs) on the hypervisor.
    pub fn list_all_domains(&self) -> Result<Vec<VmInfo>, VirtManagerError> {
        self.with_connection(|conn| {
            let domains = conn.list_all_domains(0).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "listAllDomains".into(),
                    reason: e.to_string(),
                }
            })?;

            let mut results = Vec::with_capacity(domains.len());
            for domain in &domains {
                if let Some(info) = Self::parse_domain(domain) {
                    results.push(info);
                }
            }
            Ok(results)
        })
    }

    /// Start a domain by name.
    pub fn start_domain(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_domain(name, "start", |d| d.create().map(|_| ()))
    }

    /// Gracefully shutdown a domain by name.
    pub fn shutdown_domain(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_domain(name, "shutdown", |d| d.shutdown().map(|_| ()))
    }

    /// Force stop a domain by name.
    pub fn destroy_domain(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_domain(name, "destroy", |d| d.destroy().map(|_| ()))
    }

    /// Suspend a domain by name.
    pub fn suspend_domain(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_domain(name, "suspend", |d| d.suspend().map(|_| ()))
    }

    /// Resume a paused domain by name.
    pub fn resume_domain(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_domain(name, "resume", |d| d.resume().map(|_| ()))
    }

    /// Reboot a domain by name.
    pub fn reboot_domain(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_domain(name, "reboot", |d| d.reboot(0).map(|_| ()))
    }

    /// Get the XML description for a domain.
    pub fn get_domain_xml(&self, name: &str, inactive: bool) -> Result<String, VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let flags = if inactive { 2 } else { 0 };
            domain
                .get_xml_desc(flags)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "getDomainXML".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Open a console session for a domain. The on_data callback receives
    /// bytes from the VM's serial console on a background thread.
    pub fn with_console<F>(
        &self,
        domain_name: &str,
        on_data: F,
    ) -> Result<ConsoleSession, VirtManagerError>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let guard = self.inner.lock().unwrap();
        match guard.as_ref() {
            Some(conn) => ConsoleSession::open(conn, domain_name, on_data),
            None => Err(VirtManagerError::NotConnected),
        }
    }

    // -- Private helpers --

    fn with_connection<F, T>(&self, f: F) -> Result<T, VirtManagerError>
    where
        F: FnOnce(&Connect) -> Result<T, VirtManagerError>,
    {
        let guard = self.inner.lock().unwrap();
        match guard.as_ref() {
            Some(conn) => f(conn),
            None => Err(VirtManagerError::NotConnected),
        }
    }

    fn lookup_domain(conn: &Connect, name: &str) -> Result<Domain, VirtManagerError> {
        Domain::lookup_by_name(conn, name).map_err(|_| VirtManagerError::DomainNotFound {
            name: name.to_string(),
        })
    }

    fn with_domain<F>(&self, name: &str, op_name: &str, op: F) -> Result<(), VirtManagerError>
    where
        F: FnOnce(&Domain) -> Result<(), virt::error::Error>,
    {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            op(&domain).map_err(|e| VirtManagerError::OperationFailed {
                operation: op_name.to_string(),
                reason: e.to_string(),
            })
        })
    }

    fn parse_domain(domain: &Domain) -> Option<VmInfo> {
        let name = domain.get_name().ok()?;
        let uuid = domain.get_uuid_string().ok()?;
        let info = domain.get_info().ok()?;

        let state = VmState::from_libvirt(info.state);
        let vcpus = info.nr_virt_cpu;
        let memory_kb = info.memory;

        let (graphics_type, has_serial) = match domain.get_xml_desc(0) {
            Ok(xml) => {
                let gfx = xml_helpers::extract_graphics_type(&xml).and_then(|s| match s.as_str() {
                    "vnc" => Some(GraphicsType::Vnc),
                    "spice" => Some(GraphicsType::Spice),
                    _ => None,
                });
                let serial = xml_helpers::has_serial_console(&xml);
                (gfx, serial)
            }
            Err(_) => (None, false),
        };

        Some(VmInfo {
            name,
            uuid,
            state,
            vcpus,
            memory_mb: memory_kb / 1024,
            graphics_type,
            has_serial,
        })
    }
}

impl Default for LibvirtConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LibvirtConnection {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(mut conn) = guard.take() {
                let _ = conn.close();
            }
        }
    }
}
