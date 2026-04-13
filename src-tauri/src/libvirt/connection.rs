use std::sync::Mutex;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::network::Network;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;

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


    /// Define a new domain (persistent). Does not start it.
    pub fn define_domain_xml(&self, xml: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            Domain::define_xml(conn, xml).map(|_| ()).map_err(|e| VirtManagerError::OperationFailed {
                operation: "defineDomainXML".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Undefine a persistent domain configuration. VM must be shut off.
    pub fn undefine_domain(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_domain(name, "undefineDomain", |d| d.undefine().map(|_| ()))
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


    /// Open the graphics (VNC/SPICE) FD for a domain. Returns a raw file descriptor
    /// that speaks the native graphics protocol (VNC for VNC-configured VMs,
    /// SPICE for SPICE-configured VMs). The caller takes ownership of the FD.
    pub fn open_graphics_fd(&self, domain_name: &str) -> Result<i32, VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, domain_name)?;
            // VIR_DOMAIN_OPEN_GRAPHICS_SKIPAUTH = 1 (skip auth since tunneled over SSH)
            let fd = domain.open_graphics_fd(0, 1).map_err(|e| VirtManagerError::OperationFailed {
                operation: "openGraphicsFD".into(),
                reason: e.to_string(),
            })?;
            Ok(fd as i32)
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


    /// Set vCPU count for a domain. `live` affects running VM, `config` persists.
    pub fn set_vcpus(
        &self,
        name: &str,
        count: u32,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let flags = domain_modify_flags(live, config);
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .set_vcpus_flags(count, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "setVcpus".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Set memory for a domain in KiB. `live` affects running VM, `config` persists.
    pub fn set_memory(
        &self,
        name: &str,
        memory_kib: u64,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let flags = domain_modify_flags(live, config);
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .set_memory_flags(memory_kib, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "setMemory".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Get parsed domain configuration.
    pub fn get_domain_config(
        &self,
        name: &str,
        inactive: bool,
    ) -> Result<crate::libvirt::domain_config::DomainConfig, VirtManagerError> {
        let xml = self.get_domain_xml(name, inactive)?;
        crate::libvirt::domain_config::parse(&xml)
    }


    // -- Network Management --

    /// List all virtual networks on the hypervisor.
    pub fn list_networks(&self) -> Result<Vec<crate::models::network::NetworkInfo>, VirtManagerError> {
        self.with_connection(|conn| {
            let nets = conn.list_all_networks(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "listNetworks".into(),
                reason: e.to_string(),
            })?;
            let mut results = Vec::with_capacity(nets.len());
            for net in &nets {
                if let Some(info) = Self::parse_network(net) {
                    results.push(info);
                }
            }
            Ok(results)
        })
    }

    /// Get the XML for a named network.
    pub fn get_network_xml(&self, name: &str) -> Result<String, VirtManagerError> {
        self.with_connection(|conn| {
            let net = Network::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupNetwork".into(),
                    reason: format!("network '{name}' not found"),
                }
            })?;
            net.get_xml_desc(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "getNetworkXML".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Get parsed network config.
    pub fn get_network_config(
        &self,
        name: &str,
    ) -> Result<crate::libvirt::network_config::NetworkConfig, VirtManagerError> {
        let xml = self.get_network_xml(name)?;
        crate::libvirt::network_config::parse(&xml)
    }

    /// Start an inactive network.
    pub fn start_network(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_network(name, "startNetwork", |n| n.create().map(|_| ()))
    }

    /// Stop an active network.
    pub fn stop_network(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_network(name, "stopNetwork", |n| n.destroy().map(|_| ()))
    }

    /// Define a new network from XML (without starting it).
    pub fn define_network(&self, xml: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            Network::define_xml(conn, xml).map(|_| ()).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "defineNetwork".into(),
                    reason: e.to_string(),
                }
            })
        })
    }

    /// Create (define + start) a network from XML.
    pub fn create_network(&self, xml: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let net = Network::define_xml(conn, xml).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "defineNetwork".into(),
                    reason: e.to_string(),
                }
            })?;
            net.create().map(|_| ()).map_err(|e| VirtManagerError::OperationFailed {
                operation: "startNetwork".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Undefine (remove) a network. Stops it first if active.
    pub fn delete_network(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let net = Network::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupNetwork".into(),
                    reason: format!("network '{name}' not found"),
                }
            })?;
            if net.is_active().unwrap_or(false) {
                let _ = net.destroy();
            }
            net.undefine().map_err(|e| VirtManagerError::OperationFailed {
                operation: "undefineNetwork".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Set the autostart flag for a network.
    pub fn set_network_autostart(&self, name: &str, autostart: bool) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let net = Network::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupNetwork".into(),
                    reason: format!("network '{name}' not found"),
                }
            })?;
            net.set_autostart(autostart).map(|_| ()).map_err(|e| VirtManagerError::OperationFailed {
                operation: "setAutostart".into(),
                reason: e.to_string(),
            })
        })
    }

    // -- Storage Management --

    /// List all storage pools on the hypervisor.
    pub fn list_storage_pools(&self) -> Result<Vec<crate::models::storage::StoragePoolInfo>, VirtManagerError> {
        self.with_connection(|conn| {
            let pools = conn.list_all_storage_pools(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "listStoragePools".into(),
                reason: e.to_string(),
            })?;
            let mut results = Vec::with_capacity(pools.len());
            for pool in &pools {
                if let Some(info) = Self::parse_storage_pool(pool) {
                    results.push(info);
                }
            }
            Ok(results)
        })
    }

    pub fn get_pool_xml(&self, name: &str) -> Result<String, VirtManagerError> {
        self.with_connection(|conn| {
            let pool = StoragePool::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupPool".into(),
                    reason: format!("pool '{name}' not found"),
                }
            })?;
            pool.get_xml_desc(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "getPoolXML".into(),
                reason: e.to_string(),
            })
        })
    }

    pub fn get_pool_config(&self, name: &str) -> Result<crate::libvirt::storage_config::PoolConfig, VirtManagerError> {
        let xml = self.get_pool_xml(name)?;
        crate::libvirt::storage_config::parse_pool(&xml)
    }

    pub fn start_pool(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_pool(name, "startPool", |p| p.create(0).map(|_| ()))
    }

    pub fn stop_pool(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_pool(name, "stopPool", |p| p.destroy())
    }

    pub fn refresh_pool(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_pool(name, "refreshPool", |p| p.refresh(0).map(|_| ()))
    }

    pub fn set_pool_autostart(&self, name: &str, autostart: bool) -> Result<(), VirtManagerError> {
        self.with_pool(name, "setPoolAutostart", move |p| {
            p.set_autostart(autostart).map(|_| ())
        })
    }

    /// Define a pool from XML. Optionally builds the target directory and starts it.
    pub fn define_pool(&self, xml: &str, build: bool, start: bool) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let pool = StoragePool::define_xml(conn, xml, 0).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "definePool".into(),
                    reason: e.to_string(),
                }
            })?;
            if build {
                // Build may fail if already exists; that's OK
                let _ = pool.build(0);
            }
            if start {
                pool.create(0).map(|_| ()).map_err(|e| VirtManagerError::OperationFailed {
                    operation: "startPool".into(),
                    reason: e.to_string(),
                })?;
            }
            Ok(())
        })
    }

    /// Undefine a pool. Stops it first if active.
    pub fn delete_pool(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let pool = StoragePool::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupPool".into(),
                    reason: format!("pool '{name}' not found"),
                }
            })?;
            if pool.is_active().unwrap_or(false) {
                let _ = pool.destroy();
            }
            pool.undefine().map_err(|e| VirtManagerError::OperationFailed {
                operation: "undefinePool".into(),
                reason: e.to_string(),
            })
        })
    }

    /// List volumes in a pool.
    pub fn list_volumes(&self, pool_name: &str) -> Result<Vec<crate::models::storage::StorageVolumeInfo>, VirtManagerError> {
        self.with_connection(|conn| {
            let pool = StoragePool::lookup_by_name(conn, pool_name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupPool".into(),
                    reason: format!("pool '{pool_name}' not found"),
                }
            })?;
            let vols = pool.list_all_volumes(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "listVolumes".into(),
                reason: e.to_string(),
            })?;
            let mut results = Vec::with_capacity(vols.len());
            for vol in &vols {
                if let Some(info) = Self::parse_storage_volume(vol, pool_name) {
                    results.push(info);
                }
            }
            Ok(results)
        })
    }

    /// Create a volume from XML inside a pool.
    pub fn create_volume(&self, pool_name: &str, xml: &str) -> Result<String, VirtManagerError> {
        self.with_connection(|conn| {
            let pool = StoragePool::lookup_by_name(conn, pool_name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupPool".into(),
                    reason: format!("pool '{pool_name}' not found"),
                }
            })?;
            let vol = StorageVol::create_xml(&pool, xml, 0).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "createVolume".into(),
                    reason: e.to_string(),
                }
            })?;
            vol.get_path().map_err(|e| VirtManagerError::OperationFailed {
                operation: "getVolumePath".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Delete a volume by its path.
    pub fn delete_volume(&self, path: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let vol = StorageVol::lookup_by_path(conn, path).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupVolume".into(),
                    reason: format!("volume '{path}' not found"),
                }
            })?;
            vol.delete(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "deleteVolume".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Resize a volume to the given capacity in bytes.
    pub fn resize_volume(&self, path: &str, capacity_bytes: u64) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let vol = StorageVol::lookup_by_path(conn, path).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupVolume".into(),
                    reason: format!("volume '{path}' not found"),
                }
            })?;
            vol.resize(capacity_bytes, 0).map(|_| ()).map_err(|e| VirtManagerError::OperationFailed {
                operation: "resizeVolume".into(),
                reason: e.to_string(),
            })
        })
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



    fn with_network<F>(&self, name: &str, op_name: &str, op: F) -> Result<(), VirtManagerError>
    where
        F: FnOnce(&Network) -> Result<(), virt::error::Error>,
    {
        self.with_connection(|conn| {
            let net = Network::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupNetwork".into(),
                    reason: format!("network '{name}' not found"),
                }
            })?;
            op(&net).map_err(|e| VirtManagerError::OperationFailed {
                operation: op_name.to_string(),
                reason: e.to_string(),
            })
        })
    }

    /// Parse a virt::Network into NetworkInfo summary.
    fn parse_network(net: &Network) -> Option<crate::models::network::NetworkInfo> {
        let name = net.get_name().ok()?;
        let uuid = net.get_uuid_string().ok()?;
        let is_active = net.is_active().unwrap_or(false);
        let is_persistent = net.is_persistent().unwrap_or(false);
        let autostart = net.get_autostart().unwrap_or(false);
        let bridge = net.get_bridge_name().ok();

        // Parse XML for forward mode + IP summary
        let (forward_mode, ipv4_summary, ipv6_summary) = match net.get_xml_desc(0) {
            Ok(xml) => {
                if let Ok(cfg) = crate::libvirt::network_config::parse(&xml) {
                    let v4 = cfg.ipv4.as_ref().map(crate::libvirt::network_config::ip_summary);
                    let v6 = cfg.ipv6.as_ref().map(crate::libvirt::network_config::ip_summary);
                    let mode = if cfg.forward_mode.is_empty() {
                        "isolated".to_string()
                    } else {
                        cfg.forward_mode
                    };
                    (mode, v4, v6)
                } else {
                    ("unknown".to_string(), None, None)
                }
            }
            Err(_) => ("unknown".to_string(), None, None),
        };

        Some(crate::models::network::NetworkInfo {
            name,
            uuid,
            is_active,
            is_persistent,
            autostart,
            bridge,
            forward_mode,
            ipv4_summary,
            ipv6_summary,
        })
    }

    fn with_pool<F>(&self, name: &str, op_name: &str, op: F) -> Result<(), VirtManagerError>
    where
        F: FnOnce(&StoragePool) -> Result<(), virt::error::Error>,
    {
        self.with_connection(|conn| {
            let pool = StoragePool::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupPool".into(),
                    reason: format!("pool '{name}' not found"),
                }
            })?;
            op(&pool).map_err(|e| VirtManagerError::OperationFailed {
                operation: op_name.to_string(),
                reason: e.to_string(),
            })
        })
    }

    /// Parse a virt::StoragePool into our summary info.
    fn parse_storage_pool(pool: &StoragePool) -> Option<crate::models::storage::StoragePoolInfo> {
        let name = pool.get_name().ok()?;
        let uuid = pool.get_uuid_string().ok()?;
        let is_active = pool.is_active().unwrap_or(false);
        let is_persistent = pool.is_persistent().unwrap_or(false);
        let autostart = pool.get_autostart().unwrap_or(false);
        let num_volumes = pool.num_of_volumes().unwrap_or(0);

        let mut pool_type = "unknown".to_string();
        let mut capacity = 0u64;
        let mut allocation = 0u64;
        let mut available = 0u64;
        let mut target_path = None;

        if let Ok(info) = pool.get_info() {
            capacity = info.capacity;
            allocation = info.allocation;
            available = info.available;
        }
        if let Ok(xml) = pool.get_xml_desc(0) {
            if let Ok(cfg) = crate::libvirt::storage_config::parse_pool(&xml) {
                if !cfg.pool_type.is_empty() {
                    pool_type = cfg.pool_type;
                }
                target_path = cfg.target_path;
            }
        }

        Some(crate::models::storage::StoragePoolInfo {
            name,
            uuid,
            pool_type,
            is_active,
            is_persistent,
            autostart,
            capacity,
            allocation,
            available,
            target_path,
            num_volumes,
        })
    }

    fn parse_storage_volume(
        vol: &StorageVol,
        pool_name: &str,
    ) -> Option<crate::models::storage::StorageVolumeInfo> {
        let name = vol.get_name().ok()?;
        let path = vol.get_path().ok().unwrap_or_default();
        let key = vol.get_key().ok().unwrap_or_default();

        let mut capacity = 0u64;
        let mut allocation = 0u64;
        if let Ok(info) = vol.get_info() {
            capacity = info.capacity;
            allocation = info.allocation;
        }

        let mut format = String::new();
        if let Ok(xml) = vol.get_xml_desc(0) {
            if let Ok(cfg) = crate::libvirt::storage_config::parse_volume(&xml) {
                format = cfg.format;
            }
        }
        // Fallback: detect format from extension
        if format.is_empty() {
            if name.ends_with(".qcow2") {
                format = "qcow2".into();
            } else if name.ends_with(".iso") {
                format = "iso".into();
            } else {
                format = "raw".into();
            }
        }

        Some(crate::models::storage::StorageVolumeInfo {
            name,
            path,
            key,
            capacity,
            allocation,
            format,
            pool_name: pool_name.to_string(),
        })
    }

}
/// VIR_DOMAIN_AFFECT_LIVE=1, VIR_DOMAIN_AFFECT_CONFIG=2
fn domain_modify_flags(live: bool, config: bool) -> u32 {
    let mut flags: u32 = 0;
    if live { flags |= 1; }
    if config { flags |= 2; }
    if flags == 0 { flags = 2; }  // default to config
    flags
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
