use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;
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
    ///
    /// For \`qemu+ssh://\` URIs we do a 5-second TCP pre-flight on the SSH
    /// port before handing off to libvirt. Without this, an offline
    /// hypervisor wedges the caller for ~2 minutes on the system TCP
    /// timeout, which freezes the Tauri IPC worker.
    pub fn open(&self, uri: &str) -> Result<(), VirtManagerError> {
        // Preflight can be disabled by setting KRAFTWERK_SKIP_PREFLIGHT=1
        // (used by integration tests, which tolerate the longer libvirt-side
        // timeout and may run in sandboxed contexts where raw TCP probes
        // behave differently from libvirt's ssh-spawned child).
        let skip = std::env::var("KRAFTWERK_SKIP_PREFLIGHT").ok().filter(|v| !v.is_empty()).is_some();
        if !skip {
            if let Some((host, port)) = parse_ssh_host_port(uri) {
                let addr = (host.as_str(), port)
                    .to_socket_addrs()
                    .map_err(|e| VirtManagerError::Timeout { host: format!("{host}: {e}") })?
                    .next()
                    .ok_or_else(|| VirtManagerError::Timeout { host: host.clone() })?;
                TcpStream::connect_timeout(&addr, Duration::from_secs(5))
                    .map_err(|_| VirtManagerError::Timeout { host: format!("{host}:{port}") })?;
            }
        }
        let conn = Connect::open(Some(uri)).map_err(|e| VirtManagerError::ConnectionFailed {
            host: redact_uri(uri),
            reason: e.to_string(),
        })?;
        // Register the lifecycle event callback before we install the new
        // Connect into the guard, so we can roll back on registration error.
        if let Err(e) = crate::libvirt::events::register(conn.as_ptr()) {
            log::warn!("event registration failed; falling back to polling: {e}");
        }
        let mut guard = self.inner.lock().unwrap();
        if let Some(mut old) = guard.take() {
            crate::libvirt::events::deregister(old.as_ptr());
            let _ = old.close();
        }
        *guard = Some(conn);
        Ok(())
    }

    /// Close the connection.
    pub fn close(&self) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(mut conn) = guard.take() {
            log::info!("Closing connection");
            crate::libvirt::events::deregister(conn.as_ptr());
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

    /// Aggregate host info for the dashboard view.
    pub fn get_host_info(&self) -> Result<crate::libvirt::host_info::HostInfo, crate::models::error::VirtManagerError> {
        use crate::libvirt::host_info::{format_lib_version, HostInfo};
        use crate::models::error::VirtManagerError;
        self.with_connection(|conn| {
            let hostname = conn.get_hostname().map_err(|e| VirtManagerError::OperationFailed {
                operation: "getHostname".into(), reason: e.to_string(),
            })?;
            let info = conn.get_node_info().map_err(|e| VirtManagerError::OperationFailed {
                operation: "getNodeInfo".into(), reason: e.to_string(),
            })?;
            let hypervisor_type = conn.get_type().unwrap_or_else(|_| "unknown".into());
            let lib_v = conn.get_lib_version().unwrap_or(0);
            Ok(HostInfo {
                hostname,
                hypervisor_type,
                libvirt_version: format_lib_version(lib_v),
                cpu_model: info.model,
                cpu_count: info.cpus,
                cpu_mhz: info.mhz,
                cpu_sockets: info.sockets,
                cpu_cores_per_socket: info.cores,
                cpu_threads_per_core: info.threads,
                numa_nodes: info.nodes,
                memory_kib: info.memory,
            })
        })
    }

    /// Live-ish host memory snapshot.
    ///
    /// Uses `virNodeGetMemoryStats` to retrieve total / free / buffers /
    /// cached, then computes `available = free + buffers + cached` (the
    /// /proc/meminfo `MemAvailable` semantics — what users actually care
    /// about, since "free" excludes the reclaimable page cache and is
    /// almost always misleadingly small).
    pub fn get_host_memory(&self) -> Result<crate::libvirt::host_info::HostMemory, crate::models::error::VirtManagerError> {
        use crate::libvirt::host_info::HostMemory;
        use crate::models::error::VirtManagerError;
        use std::ffi::CStr;
        self.with_connection(|conn| {
            // Discover number of stats fields first.
            let conn_ptr = conn.as_ptr();
            let mut nparams: libc::c_int = 0;
            let r = unsafe {
                virt_sys::virNodeGetMemoryStats(
                    conn_ptr,
                    virt_sys::VIR_NODE_MEMORY_STATS_ALL_CELLS,
                    std::ptr::null_mut(),
                    &mut nparams,
                    0,
                )
            };
            if r < 0 || nparams <= 0 {
                return Err(VirtManagerError::OperationFailed {
                    operation: "virNodeGetMemoryStats(probe)".into(),
                    reason: format!("returned {r}, nparams={nparams}"),
                });
            }
            let mut params: Vec<virt_sys::virNodeMemoryStats> =
                vec![unsafe { std::mem::zeroed() }; nparams as usize];
            let r = unsafe {
                virt_sys::virNodeGetMemoryStats(
                    conn_ptr,
                    virt_sys::VIR_NODE_MEMORY_STATS_ALL_CELLS,
                    params.as_mut_ptr(),
                    &mut nparams,
                    0,
                )
            };
            if r < 0 {
                return Err(VirtManagerError::OperationFailed {
                    operation: "virNodeGetMemoryStats".into(),
                    reason: format!("returned {r}"),
                });
            }
            let mut total_kib = 0u64;
            let mut free_kib = 0u64;
            let mut buffers_kib = 0u64;
            let mut cached_kib = 0u64;
            for p in params.iter().take(nparams as usize) {
                let field = unsafe { CStr::from_ptr(p.field.as_ptr()) }.to_string_lossy();
                let v = p.value as u64;
                match field.as_ref() {
                    "total" => total_kib = v,
                    "free" => free_kib = v,
                    "buffers" => buffers_kib = v,
                    "cached" => cached_kib = v,
                    _ => {}
                }
            }
            // Fallback if total wasnt provided by the driver.
            if total_kib == 0 {
                if let Ok(info) = conn.get_node_info() {
                    total_kib = info.memory;
                }
            }
            let available_kib = free_kib + buffers_kib + cached_kib;
            Ok(HostMemory {
                total_kib,
                free_kib,
                buffers_kib,
                cached_kib,
                available_kib,
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
    ///
    /// Libvirt flags bit-field:
    ///   0x01 VIR_DOMAIN_XML_SECURE   — include secure fields (VNC/SPICE password)
    ///   0x02 VIR_DOMAIN_XML_INACTIVE — return the persistent config
    pub fn get_domain_xml(&self, name: &str, inactive: bool) -> Result<String, VirtManagerError> {
        self.get_domain_xml_flags(name, inactive, false)
    }

    /// Variant that optionally requests secure fields (for SPICE/VNC
    /// password extraction). Requires sufficient libvirt privileges.
    pub fn get_domain_xml_flags(
        &self,
        name: &str,
        inactive: bool,
        secure: bool,
    ) -> Result<String, VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let mut flags: u32 = 0;
            if secure { flags |= 1; }   // VIR_DOMAIN_XML_SECURE
            if inactive { flags |= 2; } // VIR_DOMAIN_XML_INACTIVE
            domain
                .get_xml_desc(flags)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "getDomainXML".into(),
                    reason: e.to_string(),
                })
        })
    }



    /// Query domain capabilities for the given (emulator, arch, machine, virttype).
    /// All parameters can be empty strings — libvirt fills in sensible defaults
    /// for the host.
    pub fn get_domain_capabilities(
        &self,
        emulator: Option<&str>,
        arch: Option<&str>,
        machine: Option<&str>,
        virttype: Option<&str>,
    ) -> Result<crate::libvirt::domain_caps::DomainCaps, VirtManagerError> {
        self.with_connection(|conn| {
            let xml = conn
                .get_domain_capabilities(emulator, arch, machine, virttype, 0)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "getDomainCapabilities".into(),
                    reason: e.to_string(),
                })?;
            crate::libvirt::domain_caps::parse(&xml)
        })
    }


    /// Parse boot / firmware / machine / events from a domain XML.
    pub fn get_boot_config(&self, name: &str) -> Result<crate::libvirt::boot_config::BootConfig, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        crate::libvirt::boot_config::parse(&xml)
    }

    /// Apply a BootPatch to a domain. Defaults to persistent (config) only —
    /// most boot/firmware changes require a restart anyway.
    pub fn apply_boot_patch(
        &self,
        name: &str,
        patch: &crate::libvirt::boot_config::BootPatch,
    ) -> Result<(), VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?; // inactive definition
        let new_xml = crate::libvirt::boot_config::apply(&xml, patch)?;
        self.define_domain_xml(&new_xml)
    }


    // ═════════════════════════════════════════════════════════════════════
    // Round I: advanced CPU + memory tuning + iothreads
    // ═════════════════════════════════════════════════════════════════════

    /// Read the full CPU / vCPU / cputune / memtune / NUMA / hugepages /
    /// iothreads snapshot from the inactive (persistent) domain XML.
    pub fn get_cpu_tune(
        &self,
        name: &str,
    ) -> Result<crate::libvirt::cpu_tune_config::CpuTuneSnapshot, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        crate::libvirt::cpu_tune_config::parse(&xml)
    }

    /// Apply a CpuTunePatch to the persistent definition. Most of this
    /// only takes effect on next boot; vCPU count and iothread count
    /// have their own dedicated live-apply methods below.
    pub fn apply_cpu_tune(
        &self,
        name: &str,
        patch: &crate::libvirt::cpu_tune_config::CpuTunePatch,
    ) -> Result<(), VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = crate::libvirt::cpu_tune_config::apply(&xml, patch)?;
        // Validate before redefine so we give a useful error.
        let snap = crate::libvirt::cpu_tune_config::parse(&new_xml)?;
        crate::libvirt::cpu_tune_config::validate(&snap)?;
        self.define_domain_xml(&new_xml)
    }

    /// Set the current vCPU count. Supports live hotplug when `live=true`
    /// (requires the guest kernel to support CPU hotplug). Persistent
    /// change via `config=true` always works.
    pub fn set_vcpu_count(
        &self,
        name: &str,
        current: u32,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        // Reuse the existing wrapper — same semantics.
        self.set_vcpus(name, current, live, config)
    }

    /// Set the iothread count. We prefer virDomainAddIOThread /
    /// virDomainDelIOThread when we need to adjust a running guest
    /// (via virt-sys since the safe wrapper doesn't expose it). For
    /// the persistent / offline case we rewrite the XML and redefine,
    /// which is simpler and always works.
    pub fn set_iothread_count(
        &self,
        name: &str,
        count: u32,
    ) -> Result<(), VirtManagerError> {
        use crate::libvirt::cpu_tune_config::{CpuTunePatch, IoThreadsConfig};
        let patch = CpuTunePatch {
            iothreads: Some(IoThreadsConfig { count }),
            ..Default::default()
        };
        self.apply_cpu_tune(name, &patch)
    }

    /// Parse the full display bundle (graphics / video / sound / input)
    /// from a domain XML. The domain XML is fetched INACTIVE, i.e. the
    /// persistent definition — NOT with VIR_DOMAIN_XML_SECURE, so the
    /// `graphics.passwd` field will be absent for SPICE/VNC VMs that
    /// have one set. See display_config.rs docstring.
    pub fn get_display_config(
        &self,
        name: &str,
    ) -> Result<crate::libvirt::display_config::DisplayConfig, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        Ok(crate::libvirt::display_config::DisplayConfig {
            graphics: crate::libvirt::display_config::parse_graphics(&xml)?,
            video: crate::libvirt::display_config::parse_video(&xml)?,
            sound: crate::libvirt::display_config::parse_sound(&xml)?,
            input: crate::libvirt::display_config::parse_input(&xml)?,
        })
    }

    /// Apply a DisplayPatch to a domain (persistent / config flag). Each
    /// subsection in the patch is applied independently against the
    /// successive rewritten XML, so `Some(graphics) + Some(video)` both
    /// take effect. Live hotplug of graphics type changes rarely works,
    /// so we only update the persistent definition — a shutdown + start
    /// is expected for most display changes.
    pub fn apply_display_patch(
        &self,
        name: &str,
        patch: &crate::libvirt::display_config::DisplayPatch,
    ) -> Result<(), VirtManagerError> {
        let mut xml = self.get_domain_xml(name, true)?;
        if let Some(ref g) = patch.graphics {
            xml = crate::libvirt::display_config::apply_replace_graphics(&xml, g)?;
        }
        if let Some(ref v) = patch.video {
            xml = crate::libvirt::display_config::apply_replace_video(&xml, v)?;
        }
        if let Some(ref s) = patch.sound {
            xml = crate::libvirt::display_config::apply_replace_sound(&xml, s)?;
        }
        if let Some(ref inputs) = patch.inputs {
            xml = crate::libvirt::display_config::apply_replace_inputs(&xml, inputs)?;
        }
        self.define_domain_xml(&xml)
    }

    // ═════════════════════════════════════════════════════════════════════
    // Round E: virtio-adjacent devices (TPM, RNG, watchdog, panic,
    // memballoon, vsock, IOMMU).
    // ═════════════════════════════════════════════════════════════════════

    /// Read all virtio-adjacent devices from the inactive (persistent)
    /// domain XML. Inactive so edits reflect what takes effect on next
    /// boot for persistent-only devices.
    pub fn get_virtio_devices(
        &self,
        name: &str,
    ) -> Result<crate::libvirt::virtio_devices::VirtioDevicesSnapshot, VirtManagerError> {
        use crate::libvirt::virtio_devices as v;
        let xml = self.get_domain_xml(name, true)?;
        Ok(v::VirtioDevicesSnapshot {
            tpm: v::parse_tpm(&xml)?,
            rngs: v::parse_rngs(&xml)?,
            watchdog: v::parse_watchdog(&xml)?,
            panic: v::parse_panic(&xml)?,
            balloon: v::parse_balloon(&xml)?,
            vsock: v::parse_vsock(&xml)?,
            iommu: v::parse_iommu(&xml)?,
        })
    }

    /// Set or remove the TPM. Persistent only — `live` must be false.
    pub fn set_tpm(
        &self,
        name: &str,
        cfg: Option<&crate::libvirt::virtio_devices::TpmConfig>,
        live: bool,
        _config: bool,
    ) -> Result<(), VirtManagerError> {
        if live {
            return Err(VirtManagerError::OperationFailed {
                operation: "setTpm".into(),
                reason: "TPM hotplug is not supported; persistent only".into(),
            });
        }
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = crate::libvirt::virtio_devices::apply_set_tpm(&xml, cfg)?;
        self.define_domain_xml(&new_xml)
    }

    /// Set or remove the watchdog. Persistent only.
    pub fn set_watchdog(
        &self,
        name: &str,
        cfg: Option<&crate::libvirt::virtio_devices::WatchdogConfig>,
        live: bool,
        _config: bool,
    ) -> Result<(), VirtManagerError> {
        if live {
            return Err(VirtManagerError::OperationFailed {
                operation: "setWatchdog".into(),
                reason: "watchdog hotplug is not supported; persistent only".into(),
            });
        }
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = crate::libvirt::virtio_devices::apply_set_watchdog(&xml, cfg)?;
        self.define_domain_xml(&new_xml)
    }

    /// Set or remove the panic notifier. Persistent only.
    pub fn set_panic(
        &self,
        name: &str,
        cfg: Option<&crate::libvirt::virtio_devices::PanicConfig>,
        live: bool,
        _config: bool,
    ) -> Result<(), VirtManagerError> {
        if live {
            return Err(VirtManagerError::OperationFailed {
                operation: "setPanic".into(),
                reason: "panic hotplug is not supported; persistent only".into(),
            });
        }
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = crate::libvirt::virtio_devices::apply_set_panic(&xml, cfg)?;
        self.define_domain_xml(&new_xml)
    }

    /// Set or remove the memballoon. Model/flag changes are persistent;
    /// stats_period_secs is applied live via virDomainSetMemoryStatsPeriod
    /// in addition when `live` is true.
    pub fn set_balloon(
        &self,
        name: &str,
        cfg: Option<&crate::libvirt::virtio_devices::BalloonConfig>,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        if config {
            let xml = self.get_domain_xml(name, true)?;
            let new_xml = crate::libvirt::virtio_devices::apply_set_balloon(&xml, cfg)?;
            self.define_domain_xml(&new_xml)?;
        }
        if live {
            // Only the stats period is hot-settable.
            if let Some(c) = cfg {
                if let Some(period) = c.stats_period_secs {
                    self.with_connection(|conn| {
                        let domain = Self::lookup_domain(conn, name)?;
                        // flags=1 = VIR_DOMAIN_AFFECT_LIVE
                        domain
                            .set_memory_stats_period(period as i32, 1)
                            .map(|_| ())
                            .map_err(|e| VirtManagerError::OperationFailed {
                                operation: "setMemoryStatsPeriod".into(),
                                reason: e.to_string(),
                            })
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Set or remove vsock. Supports live hotplug and/or persistent.
    pub fn set_vsock(
        &self,
        name: &str,
        cfg: Option<&crate::libvirt::virtio_devices::VsockConfig>,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        if let Some(c) = cfg { c.validate()?; }
        // Persistent edit first (authoritative).
        if config {
            let xml = self.get_domain_xml(name, true)?;
            let new_xml = crate::libvirt::virtio_devices::apply_set_vsock(&xml, cfg)?;
            self.define_domain_xml(&new_xml)?;
        }
        // Live attach/detach.
        if live {
            // Get the current live vsock to know whether we are replacing.
            let live_xml = self.get_domain_xml(name, false)?;
            let current = crate::libvirt::virtio_devices::parse_vsock(&live_xml)?;
            if let Some(old) = &current {
                let frag = crate::libvirt::virtio_devices::build_vsock_xml(old);
                let _ = self.detach_device_public(name, &frag, true, false);
            }
            if let Some(c) = cfg {
                let frag = crate::libvirt::virtio_devices::build_vsock_xml(c);
                self.attach_device_public(name, &frag, true, false)?;
            }
        }
        Ok(())
    }

    /// Add an RNG device (hotplug or persistent).
    pub fn add_rng(
        &self,
        name: &str,
        cfg: &crate::libvirt::virtio_devices::RngConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let frag = crate::libvirt::virtio_devices::build_rng_xml(cfg);
        self.attach_device_public(name, &frag, live, config)
    }

    /// Remove an RNG device matching the config shape (hotplug or persistent).
    pub fn remove_rng(
        &self,
        name: &str,
        cfg: &crate::libvirt::virtio_devices::RngConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let frag = crate::libvirt::virtio_devices::build_rng_xml(cfg);
        self.detach_device_public(name, &frag, live, config)
    }

    /// Update an existing RNG device (matches by MAC-equivalent here: the
    /// serialised XML must match existing rate/backend). Uses
    /// virDomainUpdateDeviceFlags which is narrower than detach+attach.
    pub fn update_rng(
        &self,
        name: &str,
        cfg: &crate::libvirt::virtio_devices::RngConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let frag = crate::libvirt::virtio_devices::build_rng_xml(cfg);
        let flags = {
            let mut f: u32 = 0;
            if live { f |= 1; }
            if config { f |= 2; }
            if f == 0 { f = 2; }
            f
        };
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .update_device_flags(&frag, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "updateDevice".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Set or remove IOMMU. Persistent only.
    pub fn set_iommu(
        &self,
        name: &str,
        cfg: Option<&crate::libvirt::virtio_devices::IommuConfig>,
        live: bool,
        _config: bool,
    ) -> Result<(), VirtManagerError> {
        if live {
            return Err(VirtManagerError::OperationFailed {
                operation: "setIommu".into(),
                reason: "IOMMU hotplug is not supported; persistent only".into(),
            });
        }
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = crate::libvirt::virtio_devices::apply_set_iommu(&xml, cfg)?;
        self.define_domain_xml(&new_xml)
    }

    /// Public attach_device wrapper used by the virtio methods (and, in
    /// future, other device editors). Kept distinct from the private
    /// hostdev-only helper.
    pub fn attach_device_public(
        &self,
        name: &str,
        xml: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        self.attach_device(name, xml, live, config)
    }

    pub fn detach_device_public(
        &self,
        name: &str,
        xml: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        self.detach_device(name, xml, live, config)
    }
    // -- Char devices (Round F) --

    /// Get a snapshot of all character-devices on a domain
    /// (serials, consoles, channels, parallels).
    pub fn get_char_devices(
        &self,
        name: &str,
    ) -> Result<crate::libvirt::char_devices::CharDevicesSnapshot, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        Ok(crate::libvirt::char_devices::CharDevicesSnapshot {
            serials: crate::libvirt::char_devices::parse_serials(&xml)?,
            consoles: crate::libvirt::char_devices::parse_consoles(&xml)?,
            channels: crate::libvirt::char_devices::parse_channels(&xml)?,
            parallels: crate::libvirt::char_devices::parse_parallels(&xml)?,
        })
    }

    /// Add a channel to a domain (e.g. qemu-ga, vdagent). libvirt will
    /// auto-add the required virtio-serial controller on first channel.
    pub fn add_channel(
        &self,
        name: &str,
        cfg: &crate::libvirt::char_devices::ChannelConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let xml = crate::libvirt::char_devices::build_channel(cfg);
        self.attach_device(name, &xml, live, config)
    }

    /// Remove a channel matched by <target name='...'>.
    pub fn remove_channel(
        &self,
        name: &str,
        target_name: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        // We need the full channel XML to detach_device_flags — match by
        // name against our parsed channels.
        let snap = self.get_char_devices(name)?;
        let ch = snap.channels.iter()
            .find(|c| c.target_name.as_deref() == Some(target_name))
            .ok_or_else(|| VirtManagerError::OperationFailed {
                operation: "removeChannel".into(),
                reason: format!("no channel with target name '{}'", target_name),
            })?;
        let xml = crate::libvirt::char_devices::build_channel(ch);
        self.detach_device(name, &xml, live, config)
    }

    /// Add a serial port.
    pub fn add_serial(
        &self,
        name: &str,
        cfg: &crate::libvirt::char_devices::SerialConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let xml = crate::libvirt::char_devices::build_serial(cfg);
        self.attach_device(name, &xml, live, config)
    }

    /// Remove a serial port by target port number.
    pub fn remove_serial(
        &self,
        name: &str,
        port: u32,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let snap = self.get_char_devices(name)?;
        let s = snap.serials.iter()
            .find(|s| s.target_port == Some(port))
            .ok_or_else(|| VirtManagerError::OperationFailed {
                operation: "removeSerial".into(),
                reason: format!("no serial with target port {}", port),
            })?;
        let xml = crate::libvirt::char_devices::build_serial(s);
        self.detach_device(name, &xml, live, config)
    }

    /// Preset: add the standard qemu-guest-agent channel.
    pub fn add_guest_agent_channel(
        &self,
        name: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let cfg = crate::libvirt::char_devices::guest_agent_channel();
        self.add_channel(name, &cfg, live, config)
    }

    /// Preset: add the SPICE vdagent channel.
    pub fn add_spice_vdagent_channel(
        &self,
        name: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let cfg = crate::libvirt::char_devices::spice_vdagent_channel();
        self.add_channel(name, &cfg, live, config)
    }


    // ── Round G: filesystem passthrough + shared memory ──────────────

    pub fn list_filesystems(
        &self,
        name: &str,
    ) -> Result<Vec<crate::libvirt::filesystem_config::FilesystemConfig>, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        crate::libvirt::filesystem_config::parse_filesystems(&xml)
    }

    /// Add a `<filesystem>` device to the domain. If the caller passes a
    /// virtiofs filesystem and the domain does not yet have
    /// `<memoryBacking><access mode='shared'/></memoryBacking>`, the call
    /// fails. Pass `force_memory_backing=true` to first patch the
    /// persistent definition to enable shared memoryBacking (and then add
    /// the filesystem).
    ///
    /// The memoryBacking change is persistent-only - a live hot-plug is
    /// impossible if the running QEMU wasn't started with shared memory.
    pub fn add_filesystem(
        &self,
        name: &str,
        fs: &crate::libvirt::filesystem_config::FilesystemConfig,
        force_memory_backing: bool,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        use crate::libvirt::filesystem_config as fsc;

        let needs_shared = fs.driver_type == fsc::FilesystemDriver::Virtiofs;
        if needs_shared {
            let cur_xml = self.get_domain_xml(name, true)?;
            if !fsc::has_shared_memory_backing(&cur_xml) {
                if !force_memory_backing {
                    return Err(VirtManagerError::OperationFailed {
                        operation: "add_filesystem".into(),
                        reason: "virtiofs requires <memoryBacking><access mode='shared'/></memoryBacking>; re-run with force_memory_backing=true to enable it persistently".into(),
                    });
                }
                self.enable_shared_memory_backing(name)?;
            }
        }

        let fragment = fsc::build_filesystem_xml(fs)?;
        self.attach_device(name, &fragment, live, config)
    }

    pub fn remove_filesystem(
        &self,
        name: &str,
        target_dir: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        // Look up the current entry so we can synthesise a matching
        // fragment for the detach call - libvirt matches on the element
        // shape, not just a target_dir string.
        let list = self.list_filesystems(name)?;
        let fs = list
            .iter()
            .find(|f| f.target_dir == target_dir)
            .ok_or_else(|| VirtManagerError::OperationFailed {
                operation: "remove_filesystem".into(),
                reason: format!("no filesystem with target_dir='{target_dir}'"),
            })?;
        let fragment = crate::libvirt::filesystem_config::build_filesystem_xml(fs)?;
        self.detach_device(name, &fragment, live, config)
    }

    /// Update a filesystem in place by target_dir. Detach + attach
    /// (update_device_flags is finicky about virtiofs; a clean cycle is
    /// more predictable).
    pub fn update_filesystem(
        &self,
        name: &str,
        fs: &crate::libvirt::filesystem_config::FilesystemConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        // Remove by target_dir, then re-add with the new config.
        let _ = self.remove_filesystem(name, &fs.target_dir, live, config);
        self.add_filesystem(name, fs, false, live, config)
    }

    pub fn list_shmems(
        &self,
        name: &str,
    ) -> Result<Vec<crate::libvirt::filesystem_config::ShmemConfig>, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        crate::libvirt::filesystem_config::parse_shmems(&xml)
    }

    pub fn add_shmem(
        &self,
        name: &str,
        sh: &crate::libvirt::filesystem_config::ShmemConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let fragment = crate::libvirt::filesystem_config::build_shmem_xml(sh)?;
        self.attach_device(name, &fragment, live, config)
    }

    pub fn remove_shmem(
        &self,
        name: &str,
        shmem_name: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let list = self.list_shmems(name)?;
        let sh = list
            .iter()
            .find(|s| s.name == shmem_name)
            .ok_or_else(|| VirtManagerError::OperationFailed {
                operation: "remove_shmem".into(),
                reason: format!("no shmem named '{shmem_name}'"),
            })?;
        let fragment = crate::libvirt::filesystem_config::build_shmem_xml(sh)?;
        self.detach_device(name, &fragment, live, config)
    }

    /// Add `<memoryBacking><access mode='shared'/></memoryBacking>` to
    /// the persistent domain definition if it isn't already there.
    /// Persistent-only - the running QEMU must be restarted to pick it
    /// up. Noop if the element is already present.
    pub fn enable_shared_memory_backing(&self, name: &str) -> Result<(), VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        if crate::libvirt::filesystem_config::has_shared_memory_backing(&xml) {
            return Ok(());
        }
        let new_xml = crate::libvirt::filesystem_config::apply_enable_shared_memory_backing(&xml)?;
        self.define_domain_xml(&new_xml)
    }

    /// Remove `<memoryBacking>` from the persistent definition. Used by
    /// integration-test cleanup so we don't leave the test VM forever in
    /// shared-memory mode after a virtiofs probe.
    pub fn remove_memory_backing(&self, name: &str) -> Result<(), VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = crate::libvirt::filesystem_config::apply_remove_memory_backing(&xml)?;
        self.define_domain_xml(&new_xml)
    }
    // ────────── controllers (Round H) ──────────

    /// List all <controller> entries from a domain's persistent XML.
    pub fn list_controllers(
        &self,
        name: &str,
    ) -> Result<Vec<crate::libvirt::controller_config::ControllerConfig>, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        crate::libvirt::controller_config::parse_controllers(&xml)
    }

    /// Add a controller. Persistent-only by default (most controller
    /// changes require restart); caller can opt into live via flags.
    pub fn add_controller(
        &self,
        name: &str,
        cfg: &crate::libvirt::controller_config::ControllerConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let frag = crate::libvirt::controller_config::build_controller_xml(cfg)?;
        self.attach_device(name, &frag, live, config)
    }

    /// Detach a controller by (type, index). Persistent-only by default.
    pub fn remove_controller(
        &self,
        name: &str,
        ctype: &str,
        index: u32,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        // Build a minimal <controller/> stub — libvirt matches on type+index.
        let frag = format!(
            "<controller type='{}' index='{}'/>",
            crate::libvirt::xml_helpers::escape_xml(ctype),
            index,
        );
        self.detach_device(name, &frag, live, config)
    }

    /// Update a controller: rebuild the full <controller> block by
    /// splicing into the persistent XML, then redefine.
    ///
    /// Persistent-only. Most controller model changes require VM shutdown
    /// — libvirt will reject live updates for these anyway.
    pub fn update_controller(
        &self,
        name: &str,
        ctype: &str,
        index: u32,
        new_cfg: &crate::libvirt::controller_config::ControllerConfig,
    ) -> Result<(), VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = crate::libvirt::controller_config::apply_update_controller(
            &xml, ctype, index, new_cfg,
        )?;
        self.define_domain_xml(&new_xml)
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


    /// Sample live stats for a domain.
    pub fn sample_domain_stats(
        &self,
        name: &str,
    ) -> Result<crate::libvirt::domain_stats::DomainStatsSample, VirtManagerError> {
        self.with_connection(|conn| crate::libvirt::domain_stats::sample(conn, name))
    }


    // -- Host device enumeration --

    /// Enumerate PCI devices on the hypervisor host.
    pub fn list_host_pci_devices(
        &self,
    ) -> Result<Vec<crate::libvirt::hostdev::HostPciDevice>, VirtManagerError> {
        use virt::sys::VIR_CONNECT_LIST_NODE_DEVICES_CAP_PCI_DEV;
        self.with_connection(|conn| {
            let devs = conn
                .list_all_node_devices(VIR_CONNECT_LIST_NODE_DEVICES_CAP_PCI_DEV)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "listHostPciDevices".into(),
                    reason: e.to_string(),
                })?;
            let mut out = Vec::with_capacity(devs.len());
            for d in &devs {
                if let Ok(xml) = d.get_xml_desc(0) {
                    if let Ok(parsed) = crate::libvirt::hostdev::parse_pci_node_device(&xml) {
                        out.push(parsed);
                    }
                }
            }
            // Sort stable by BDF for a predictable UI.
            out.sort_by_key(|d| (d.domain, d.bus, d.slot, d.function));
            Ok(out)
        })
    }

    /// Enumerate USB devices on the hypervisor host.
    pub fn list_host_usb_devices(
        &self,
    ) -> Result<Vec<crate::libvirt::hostdev::HostUsbDevice>, VirtManagerError> {
        use virt::sys::VIR_CONNECT_LIST_NODE_DEVICES_CAP_USB_DEV;
        self.with_connection(|conn| {
            let devs = conn
                .list_all_node_devices(VIR_CONNECT_LIST_NODE_DEVICES_CAP_USB_DEV)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "listHostUsbDevices".into(),
                    reason: e.to_string(),
                })?;
            let mut out = Vec::with_capacity(devs.len());
            for d in &devs {
                if let Ok(xml) = d.get_xml_desc(0) {
                    if let Ok(parsed) = crate::libvirt::hostdev::parse_usb_node_device(&xml) {
                        out.push(parsed);
                    }
                }
            }
            out.sort_by_key(|d| (d.bus, d.device));
            Ok(out)
        })
    }

    /// List the PCI/USB passthrough entries currently attached to a domain.
    pub fn list_domain_hostdevs(
        &self,
        name: &str,
    ) -> Result<Vec<crate::libvirt::hostdev::HostDevice>, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        crate::libvirt::hostdev::parse_hostdevs(&xml)
    }

    /// Attach a hostdev entry to a domain, live and/or persistent.
    pub fn attach_hostdev(
        &self,
        name: &str,
        dev: &crate::libvirt::hostdev::HostDevice,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let xml = crate::libvirt::hostdev::build_hostdev_xml(dev);
        self.attach_device(name, &xml, live, config)
    }

    /// Detach a hostdev entry from a domain.
    pub fn detach_hostdev(
        &self,
        name: &str,
        dev: &crate::libvirt::hostdev::HostDevice,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let xml = crate::libvirt::hostdev::build_hostdev_xml(dev);
        self.detach_device(name, &xml, live, config)
    }

    /// Generic attach_device wrapper. Kept narrow — hostdev only for now.
    fn attach_device(&self, name: &str, xml: &str, live: bool, config: bool) -> Result<(), VirtManagerError> {
        let flags = domain_modify_flags(live, config);
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .attach_device_flags(xml, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "attachDevice".into(),
                    reason: e.to_string(),
                })
        })
    }

    fn detach_device(&self, name: &str, xml: &str, live: bool, config: bool) -> Result<(), VirtManagerError> {
        let flags = domain_modify_flags(live, config);
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .detach_device_flags(xml, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "detachDevice".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Generic update_device wrapper. Used for live-editing devices that
    /// libvirt supports in-place updates for (CD-ROM media, NIC link
    /// state, etc). Unlike attach/detach, this mutates the existing
    /// device identified by a stable key in the XML (MAC for NICs,
    /// target dev for disks).
    fn update_device(&self, name: &str, xml: &str, live: bool, config: bool) -> Result<(), VirtManagerError> {
        let flags = domain_modify_flags(live, config);
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .update_device_flags(xml, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "updateDevice".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Parse the disks attached to a domain from its inactive XML.
    pub fn list_domain_disks(
        &self,
        name: &str,
    ) -> Result<Vec<crate::libvirt::disk_config::DiskConfig>, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        crate::libvirt::disk_config::parse_disks_full(&xml)
    }

    /// Attach a new disk to a domain (live and/or persistent).
    /// Uses virDomainAttachDeviceFlags.
    pub fn add_domain_disk(
        &self,
        name: &str,
        disk: &crate::libvirt::disk_config::DiskConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        crate::libvirt::disk_config::validate(disk)?;
        let xml = crate::libvirt::disk_config::build_disk_xml(disk);
        self.attach_device(name, &xml, live, config)
    }

    /// Detach a disk from a domain by target dev name.
    /// Builds a minimal `<disk>` fragment matching the current config so
    /// libvirt can find the device.
    pub fn remove_domain_disk(
        &self,
        name: &str,
        target_dev: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let disks = self.list_domain_disks(name)?;
        let disk = disks
            .iter()
            .find(|d| d.target == target_dev)
            .ok_or_else(|| VirtManagerError::OperationFailed {
                operation: "removeDomainDisk".into(),
                reason: format!("disk with target '{}' not found", target_dev),
            })?;
        let xml = crate::libvirt::disk_config::build_disk_xml(disk);
        self.detach_device(name, &xml, live, config)
    }

    /// Update a disk in place — used for CD-ROM media change. Matched by
    /// target dev. Uses virDomainUpdateDeviceFlags.
    pub fn update_domain_disk(
        &self,
        name: &str,
        disk: &crate::libvirt::disk_config::DiskConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        crate::libvirt::disk_config::validate(disk)?;
        let xml = crate::libvirt::disk_config::build_disk_xml(disk);
        self.update_device(name, &xml, live, config)
    }

    // -- NIC management (Round C) --

    /// List every `<interface>` attached to the domain, in document order.
    pub fn list_domain_nics(
        &self,
        name: &str,
    ) -> Result<Vec<crate::libvirt::nic_config::NicConfig>, VirtManagerError> {
        let xml = self.get_domain_xml(name, false)?;
        crate::libvirt::nic_config::parse_nics(&xml)
    }

    /// Hot-add or persistent-add a NIC to a domain.
    pub fn add_domain_nic(
        &self,
        name: &str,
        nic: &crate::libvirt::nic_config::NicConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        crate::libvirt::nic_config::validate(nic)?;
        let xml = crate::libvirt::nic_config::build_nic_xml(nic);
        self.attach_device(name, &xml, live, config)
    }

    /// Remove the NIC identified by MAC (or target dev, as fallback).
    /// We look it up in the current domain XML so libvirt gets the
    /// full original device fragment — detach is picky about that.
    pub fn remove_domain_nic(
        &self,
        name: &str,
        mac_or_target: &str,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let dom_xml = self.get_domain_xml(name, !live)?;
        let nics = crate::libvirt::nic_config::parse_nics(&dom_xml)?;
        let needle = mac_or_target.to_ascii_lowercase();
        let nic = nics.into_iter().find(|n| {
            n.mac.as_deref().map(|m| m.to_ascii_lowercase()) == Some(needle.clone())
                || n.target_dev.as_deref().map(|t| t.to_ascii_lowercase()) == Some(needle.clone())
        }).ok_or_else(|| VirtManagerError::OperationFailed {
            operation: "removeDomainNic".into(),
            reason: format!("no interface matching '{mac_or_target}' on {name}"),
        })?;
        let xml = crate::libvirt::nic_config::build_nic_xml(&nic);
        self.detach_device(name, &xml, live, config)
    }

    /// Update an existing NIC in place (e.g. link state flip). The NIC's
    /// MAC address in `nic.mac` is the key libvirt uses to find the
    /// existing device; callers must preserve it across updates.
    pub fn update_domain_nic(
        &self,
        name: &str,
        nic: &crate::libvirt::nic_config::NicConfig,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        crate::libvirt::nic_config::validate(nic)?;
        let xml = crate::libvirt::nic_config::build_nic_xml(nic);
        self.update_device(name, &xml, live, config)
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

    /// Get the persistent `<maxMemory slots>` config plus a count of
    /// already-attached DIMM devices. Returns `(config_or_none, dimm_count)`.
    pub fn get_memory_hotplug(
        &self,
        name: &str,
    ) -> Result<(Option<crate::libvirt::memory_hotplug::MaxMemoryConfig>, u32), VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        let cfg = crate::libvirt::memory_hotplug::parse_max_memory(&xml);
        let count = crate::libvirt::memory_hotplug::count_dimms(&xml);
        Ok((cfg, count))
    }

    /// Set the `<maxMemory slots="N">` element on a domain. Persistent
    /// only — libvirt requires the VM to be shut off for this to take
    /// effect on next boot. Rewrites the XML and redefines.
    pub fn set_max_memory_slots(
        &self,
        name: &str,
        max_kib: u64,
        slots: u32,
    ) -> Result<(), VirtManagerError> {
        use crate::libvirt::memory_hotplug::{apply_max_memory, MaxMemoryConfig};
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = apply_max_memory(&xml, &MaxMemoryConfig { max_kib, slots });
        self.define_domain_xml(&new_xml)
    }

    /// Live-attach a DIMM device. Requires that the domain has at least
    /// one free `<maxMemory slots>` AND that base+attached <= max.
    /// `live` and `config` map to AFFECT_LIVE / AFFECT_CONFIG.
    pub fn attach_memory_dimm(
        &self,
        name: &str,
        size_kib: u64,
        node: Option<u32>,
        live: bool,
        config: bool,
    ) -> Result<(), VirtManagerError> {
        let xml = crate::libvirt::memory_hotplug::build_dimm_xml(size_kib, node);
        let flags = domain_modify_flags(live, config);
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .attach_device_flags(&xml, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "attachDeviceDimm".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Set the **maximum (boot-time) memory** of a domain in KiB.
    ///
    /// libvirt requires this for the persistent config only; live runtime
    /// max-memory increase requires pre-declared memory hotplug slots,
    /// which we don't model yet, so we only touch the config domain here.
    /// The VM typically needs to be shut off for the change to take effect
    /// on next boot.
    ///
    /// VIR_DOMAIN_MEM_MAXIMUM = 4, VIR_DOMAIN_AFFECT_CONFIG = 2.
    pub fn set_max_memory(
        &self,
        name: &str,
        memory_kib: u64,
    ) -> Result<(), VirtManagerError> {
        let flags: u32 = 4 | 2; // MEM_MAXIMUM | AFFECT_CONFIG
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .set_memory_flags(memory_kib, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "setMaxMemory".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Set the **maximum (boot-time) vCPU count** of a domain.
    ///
    /// libvirt requires this for the persistent config only; raising max
    /// vCPUs generally requires the VM to be shut off for the change to
    /// take effect on next boot.
    ///
    /// VIR_DOMAIN_VCPU_MAXIMUM = 4, VIR_DOMAIN_AFFECT_CONFIG = 2.
    pub fn set_max_vcpus(
        &self,
        name: &str,
        count: u32,
    ) -> Result<(), VirtManagerError> {
        let flags: u32 = 4 | 2; // VCPU_MAXIMUM | AFFECT_CONFIG
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .set_vcpus_flags(count, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "setMaxVcpus".into(),
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


    /// List snapshots for a domain. Returns SnapshotInfo entries with
    /// is_current populated. Empty list when no snapshots exist.
    pub fn list_snapshots(&self, name: &str) -> Result<Vec<crate::libvirt::snapshots::SnapshotInfo>, VirtManagerError> {
        use crate::libvirt::snapshots::{parse_snapshot_xml, SnapshotInfo};
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let snaps = domain.list_all_snapshots(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "listAllSnapshots".into(),
                reason: e.to_string(),
            })?;
            let mut out: Vec<SnapshotInfo> = Vec::with_capacity(snaps.len());
            for snap in &snaps {
                let xml = match snap.get_xml_desc(0) {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                let mut info = parse_snapshot_xml(&xml);
                info.is_current = snap.is_current(0).unwrap_or(false);
                info.has_metadata = snap.has_metadata(0).unwrap_or(true);
                out.push(info);
            }
            Ok(out)
        })
    }

    /// Create a snapshot. The flags param is forwarded — pass 0 for the
    /// default behaviour (libvirt picks internal vs external based on
    /// disk format). Common flags:
    /// VIR_DOMAIN_SNAPSHOT_CREATE_HALT = 1
    /// VIR_DOMAIN_SNAPSHOT_CREATE_DISK_ONLY = 2
    /// VIR_DOMAIN_SNAPSHOT_CREATE_REUSE_EXT = 4
    /// VIR_DOMAIN_SNAPSHOT_CREATE_QUIESCE = 8
    /// VIR_DOMAIN_SNAPSHOT_CREATE_ATOMIC = 16
    /// VIR_DOMAIN_SNAPSHOT_CREATE_LIVE = 32
    pub fn create_snapshot(
        &self,
        name: &str,
        snap_name: &str,
        description: Option<&str>,
        flags: u32,
    ) -> Result<crate::libvirt::snapshots::SnapshotInfo, VirtManagerError> {
        use crate::libvirt::snapshots::{build_create_xml, parse_snapshot_xml};
        let xml = build_create_xml(snap_name, description);
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let snap = virt::domain_snapshot::DomainSnapshot::create_xml(&domain, &xml, flags)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "snapshotCreateXML".into(),
                    reason: e.to_string(),
                })?;
            let xml_back = snap.get_xml_desc(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "snapshotGetXMLDesc".into(),
                reason: e.to_string(),
            })?;
            let mut info = parse_snapshot_xml(&xml_back);
            info.is_current = snap.is_current(0).unwrap_or(true);
            info.has_metadata = snap.has_metadata(0).unwrap_or(true);
            Ok(info)
        })
    }

    /// Revert the domain to a named snapshot.
    /// Common flags:
    /// VIR_DOMAIN_SNAPSHOT_REVERT_RUNNING = 1 (force running after revert)
    /// VIR_DOMAIN_SNAPSHOT_REVERT_PAUSED = 2
    /// VIR_DOMAIN_SNAPSHOT_REVERT_FORCE = 4 (allow risky reverts)
    /// VIR_DOMAIN_SNAPSHOT_REVERT_RESET_NVRAM = 8
    pub fn revert_snapshot(&self, name: &str, snap_name: &str, flags: u32) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let snap = virt::domain_snapshot::DomainSnapshot::lookup_by_name(&domain, snap_name, 0)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "snapshotLookupByName".into(),
                    reason: e.to_string(),
                })?;
            snap.revert(flags).map_err(|e| VirtManagerError::OperationFailed {
                operation: "snapshotRevert".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Delete a named snapshot. By default deletes only this snapshot —
    /// children are re-parented to its parent. Pass DELETE_CHILDREN = 1 to
    /// recursively delete the whole subtree, DELETE_METADATA_ONLY = 2 to
    /// remove only the libvirt metadata (overlay files keep), or
    /// DELETE_CHILDREN_ONLY = 4 to keep the snapshot but drop its kids.
    pub fn delete_snapshot(&self, name: &str, snap_name: &str, flags: u32) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let snap = virt::domain_snapshot::DomainSnapshot::lookup_by_name(&domain, snap_name, 0)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "snapshotLookupByName".into(),
                    reason: e.to_string(),
                })?;
            snap.delete(flags).map_err(|e| VirtManagerError::OperationFailed {
                operation: "snapshotDelete".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Clone a shut-off domain. Iterates the source's disks; for each
    /// r/w file-backed disk we look up the source volume + its pool,
    /// build a `<volume>` XML for the target, call
    /// `virStorageVolCreateXMLFrom` to copy bytes, then rewrite the
    /// domain XML's `file=...` reference to the new path. CD-ROMs and
    /// readonly/shareable disks pass through untouched.
    ///
    /// Returns the new domain's name on success.
    pub fn clone_domain(
        &self,
        source: &str,
        opts: &crate::libvirt::clone::CloneOptions,
    ) -> Result<String, VirtManagerError> {
        use crate::libvirt::clone::{build_clone_volume_xml, rewrite_domain_xml};
        use virt::storage_pool::StoragePool;
        use virt::storage_vol::StorageVol;

        // Source state must be shut off; full-copy clone of a running
        // VM races with guest writes. We keep the existing VM intact
        // and return a clear error.
        let src_xml = self.get_domain_xml(source, true)?;

        // Collect the source disk paths from the XML to drive volume cloning.
        let mut disk_paths: Vec<String> = Vec::new();
        let mut rest = src_xml.as_str();
        while let Some(i) = rest.find("<disk ") {
            rest = &rest[i..];
            let close = rest.find("</disk>").unwrap_or(rest.len());
            let block = &rest[..close];
            // Skip read-only / cdrom devices (they pass through).
            let readonly = block.contains("<readonly/>") || block.contains("device='cdrom'") || block.contains("device=\"cdrom\"");
            if !readonly {
                if let Some(p) = extract_attr_value(block, "source", "file")
                    .or_else(|| extract_attr_value(block, "source", "dev")) {
                    disk_paths.push(p);
                }
            }
            rest = &rest[close..];
        }

        // Copy each volume.
        let mut path_map: Vec<(String, String)> = Vec::new();
        let target_name = opts.target_name.clone();
        self.with_connection(|conn| -> Result<(), VirtManagerError> {
            for (idx, src_path) in disk_paths.iter().enumerate() {
                let src_vol = StorageVol::lookup_by_path(conn, src_path).map_err(|e| {
                    VirtManagerError::OperationFailed {
                        operation: "lookupSourceVolume".into(),
                        reason: format!("{src_path}: {e}"),
                    }
                })?;
                let pool = StoragePool::lookup_by_volume(&src_vol).map_err(|e| {
                    VirtManagerError::OperationFailed {
                        operation: "lookupVolumePool".into(),
                        reason: e.to_string(),
                    }
                })?;
                let info = src_vol.get_info().map_err(|e| VirtManagerError::OperationFailed {
                    operation: "getVolInfo".into(),
                    reason: e.to_string(),
                })?;
                let format = detect_volume_format(&src_vol).unwrap_or_else(|| "qcow2".into());
                // Build target name: append index for uniqueness across disks.
                let new_vol_name = if disk_paths.len() == 1 {
                    format!("{}.{}", target_name, format)
                } else {
                    format!("{}-{}.{}", target_name, idx, format)
                };
                let vol_xml = build_clone_volume_xml(&new_vol_name, info.capacity, &format);
                let new_vol = StorageVol::create_xml_from(&pool, &vol_xml, &src_vol, 0).map_err(|e| {
                    VirtManagerError::OperationFailed {
                        operation: "createVolFromSource".into(),
                        reason: e.to_string(),
                    }
                })?;
                let new_path = new_vol.get_path().map_err(|e| VirtManagerError::OperationFailed {
                    operation: "getNewVolPath".into(),
                    reason: e.to_string(),
                })?;
                path_map.push((src_path.clone(), new_path));
            }
            Ok(())
        })?;

        // Rewrite the source XML and define the clone.
        let new_xml = rewrite_domain_xml(&src_xml, &opts.target_name, &path_map);
        self.define_domain_xml(&new_xml)?;

        if opts.start_after {
            // Best-effort start; if it fails we still return the clone name
            // so the user can retry from the UI.
            if let Err(e) = self.start_domain(&opts.target_name) {
                log::warn!("clone defined but start failed: {e}");
            }
        }
        Ok(opts.target_name.clone())
    }

    /// Managed-save: libvirt suspends the VM to its own state file and
    /// shuts the qemu process down. The next `start_domain` resumes
    /// from that file automatically; no caller-managed paths.
    /// Equivalent to `virsh managedsave`.
    pub fn managed_save(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .managed_save(0)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "managedSave".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Whether the domain currently has a managed-save state on disk
    /// waiting to be resumed.
    pub fn has_managed_save(&self, name: &str) -> Result<bool, VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .has_managed_save(0)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "hasManagedSave".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Discard a pending managed-save state without resuming. Next
    /// `start_domain` will boot fresh from disk.
    pub fn managed_save_remove(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .managed_save_remove(0)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "managedSaveRemove".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Dump VM memory to a hypervisor-side file path. `crash` controls
    /// whether the VM is left in a CRASHED state after dump (otherwise
    /// it resumes). Format: 0 = raw, 1 = compressed-zlib, 2 = lz4.
    /// VIR_DUMP_LIVE = 1 (don't pause for dump if possible),
    /// VIR_DUMP_CRASH = 2, VIR_DUMP_BYPASS_CACHE = 4.
    pub fn core_dump(
        &self,
        name: &str,
        path: &str,
        crash_after: bool,
        live: bool,
    ) -> Result<(), VirtManagerError> {
        let mut flags: u32 = 0;
        if live { flags |= 1; }
        if crash_after { flags |= 2; }
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            domain
                .core_dump(path, flags)
                .map(|_| ())
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "coreDump".into(),
                    reason: e.to_string(),
                })
        })
    }

    /// Capture a screenshot of the guest console for the given screen
    /// (0 = primary). Returns mime type + raw bytes which the frontend
    /// can base64-encode for display.
    pub fn screenshot(
        &self,
        name: &str,
        screen: u32,
    ) -> Result<(String, Vec<u8>), VirtManagerError> {
        use virt::stream::Stream;
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let stream = Stream::new(conn, 0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "streamNew".into(),
                reason: e.to_string(),
            })?;
            let mime = domain.screenshot(&stream, screen, 0).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "screenshot".into(),
                    reason: e.to_string(),
                }
            })?;
            // Drain the stream into memory.
            let mut bytes: Vec<u8> = Vec::with_capacity(256 * 1024);
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = stream.recv(&mut buf).map_err(|e| {
                    VirtManagerError::OperationFailed {
                        operation: "streamRecv".into(),
                        reason: e.to_string(),
                    }
                })?;
                if n == 0 { break; }
                bytes.extend_from_slice(&buf[..n as usize]);
                if bytes.len() > 50 * 1024 * 1024 {
                    return Err(VirtManagerError::OperationFailed {
                        operation: "screenshot".into(),
                        reason: "screenshot exceeds 50 MiB".into(),
                    });
                }
            }
            let _ = stream.finish();
            Ok((mime, bytes))
        })
    }

    /// Get the parsed backing chain (one entry per `<disk>`) for a domain.
    /// Reads the inactive XML so chains reflect the persistent definition,
    /// not what qemu happens to have open right now.
    pub fn get_backing_chains(
        &self,
        name: &str,
    ) -> Result<Vec<crate::libvirt::backing_chain::DiskBackingChain>, VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        Ok(crate::libvirt::backing_chain::parse_chains(&xml))
    }

    /// virDomainBlockPull — flatten an overlay onto the active disk
    /// image. After completion, the chain is reduced to a single image.
    /// Async: returns immediately; the job runs in the background and
    /// progress is queried via `get_block_job_info`.
    ///
    /// `bandwidth` is bytes/sec (0 = unlimited). Pass
    /// `flags = VIR_DOMAIN_BLOCK_PULL_BANDWIDTH_BYTES (64)` so libvirt
    /// interprets `bandwidth` as bytes (legacy default is MiB/s).
    pub fn block_pull(
        &self,
        name: &str,
        disk: &str,
        bandwidth: u64,
    ) -> Result<(), VirtManagerError> {
        use std::ffi::CString;
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let disk_c = CString::new(disk).map_err(|_| VirtManagerError::OperationFailed {
                operation: "blockPull".into(),
                reason: "disk name has nul byte".into(),
            })?;
            let r = unsafe {
                virt_sys::virDomainBlockPull(
                    domain.as_ptr(),
                    disk_c.as_ptr(),
                    bandwidth,
                    64, // VIR_DOMAIN_BLOCK_PULL_BANDWIDTH_BYTES
                )
            };
            if r < 0 {
                return Err(VirtManagerError::OperationFailed {
                    operation: "blockPull".into(),
                    reason: format!("virDomainBlockPull returned {r}"),
                });
            }
            Ok(())
        })
    }

    /// virDomainBlockCommit — commit an overlay's contents into a lower
    /// image in the chain. With `top` and `base` empty strings (passed
    /// as null) libvirt commits the active overlay into the next-below
    /// backing image. Pass `delete_after = true` to set
    /// `VIR_DOMAIN_BLOCK_COMMIT_DELETE` so libvirt unlinks the now-empty
    /// top image when the job finishes (still requires the file to be
    /// inside a libvirt-managed pool).
    ///
    /// `active = true` means commit the currently-running overlay (the
    /// `<source>` itself); requires `VIR_DOMAIN_BLOCK_COMMIT_ACTIVE = 4`
    /// and a follow-up `block_job_abort(pivot=true)` to swap pointers.
    pub fn block_commit(
        &self,
        name: &str,
        disk: &str,
        top: Option<&str>,
        base: Option<&str>,
        bandwidth: u64,
        active: bool,
        delete_after: bool,
    ) -> Result<(), VirtManagerError> {
        use std::ffi::CString;
        let mut flags: u32 = 16; // VIR_DOMAIN_BLOCK_COMMIT_BANDWIDTH_BYTES
        if active { flags |= 4; }
        if delete_after { flags |= 2; }
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let disk_c = CString::new(disk).map_err(|_| VirtManagerError::OperationFailed {
                operation: "blockCommit".into(),
                reason: "disk name has nul byte".into(),
            })?;
            let top_c = top
                .map(|s| CString::new(s))
                .transpose()
                .map_err(|_| VirtManagerError::OperationFailed {
                    operation: "blockCommit".into(),
                    reason: "top has nul byte".into(),
                })?;
            let base_c = base
                .map(|s| CString::new(s))
                .transpose()
                .map_err(|_| VirtManagerError::OperationFailed {
                    operation: "blockCommit".into(),
                    reason: "base has nul byte".into(),
                })?;
            let r = unsafe {
                virt_sys::virDomainBlockCommit(
                    domain.as_ptr(),
                    disk_c.as_ptr(),
                    base_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                    top_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                    bandwidth,
                    flags,
                )
            };
            if r < 0 {
                return Err(VirtManagerError::OperationFailed {
                    operation: "blockCommit".into(),
                    reason: format!("virDomainBlockCommit returned {r}"),
                });
            }
            Ok(())
        })
    }

    /// Poll the running block job for `disk`. Returns None when no job
    /// is in flight. Caller can divide `cur` / `end` for a 0..1 progress
    /// fraction.
    pub fn get_block_job_info(
        &self,
        name: &str,
        disk: &str,
    ) -> Result<Option<crate::libvirt::backing_chain::BlockJobInfo>, VirtManagerError> {
        use std::ffi::CString;
        use std::mem::MaybeUninit;
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let disk_c = CString::new(disk).map_err(|_| VirtManagerError::OperationFailed {
                operation: "blockJobInfo".into(),
                reason: "disk name has nul byte".into(),
            })?;
            let mut info: MaybeUninit<virt_sys::virDomainBlockJobInfo> = MaybeUninit::zeroed();
            let r = unsafe {
                virt_sys::virDomainGetBlockJobInfo(
                    domain.as_ptr(),
                    disk_c.as_ptr(),
                    info.as_mut_ptr(),
                    0,
                )
            };
            match r {
                0 => Ok(None),               // no active job
                1 => {
                    let info = unsafe { info.assume_init() };
                    let kind = match info.type_ {
                        1 => "pull",
                        2 => "copy",
                        3 => "commit",
                        4 => "active_commit",
                        5 => "backup",
                        _ => "unknown",
                    };
                    Ok(Some(crate::libvirt::backing_chain::BlockJobInfo {
                        kind: kind.to_string(),
                        bandwidth: info.bandwidth as u64,
                        cur: info.cur as u64,
                        end: info.end as u64,
                    }))
                }
                _ => Err(VirtManagerError::OperationFailed {
                    operation: "blockJobInfo".into(),
                    reason: format!("virDomainGetBlockJobInfo returned {r}"),
                }),
            }
        })
    }

    /// virDomainBlockJobAbort. With `pivot = true` and an active commit,
    /// swaps the live image pointer to the lower base — required to
    /// finalise an active commit job.
    pub fn block_job_abort(
        &self,
        name: &str,
        disk: &str,
        pivot: bool,
    ) -> Result<(), VirtManagerError> {
        use std::ffi::CString;
        let flags: u32 = if pivot { 2 } else { 0 }; // VIR_DOMAIN_BLOCK_JOB_ABORT_PIVOT
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            let disk_c = CString::new(disk).map_err(|_| VirtManagerError::OperationFailed {
                operation: "blockJobAbort".into(),
                reason: "disk name has nul byte".into(),
            })?;
            let r = unsafe {
                virt_sys::virDomainBlockJobAbort(domain.as_ptr(), disk_c.as_ptr(), flags)
            };
            if r < 0 {
                return Err(VirtManagerError::OperationFailed {
                    operation: "blockJobAbort".into(),
                    reason: format!("virDomainBlockJobAbort returned {r}"),
                });
            }
            Ok(())
        })
    }

    /// Upload a local file's contents into an existing storage volume
    /// over libvirt's stream RPC. The volume must already exist (use
    /// create_volume first); upload only writes bytes, it doesn't
    /// allocate. Calls the supplied progress callback after each chunk.
    ///
    /// `chunk_size` is bytes-per-iteration. 1 MiB is a reasonable
    /// default; larger means fewer round-trips but coarser progress.
    pub fn upload_volume_from_path(
        &self,
        pool_name: &str,
        vol_name: &str,
        source_path: &str,
        chunk_size: usize,
        on_progress: impl Fn(u64, u64),
    ) -> Result<u64, VirtManagerError> {
        use std::io::Read;
        use virt::storage_pool::StoragePool;
        use virt::storage_vol::StorageVol;
        use virt::stream::Stream;
        let chunk_size = chunk_size.max(64 * 1024).min(16 * 1024 * 1024);
        let path = source_path.to_string();
        let metadata = std::fs::metadata(&path).map_err(|e| VirtManagerError::OperationFailed {
            operation: "uploadVolumeStat".into(),
            reason: format!("{path}: {e}"),
        })?;
        let total = metadata.len();
        let mut file = std::fs::File::open(&path).map_err(|e| VirtManagerError::OperationFailed {
            operation: "uploadVolumeOpen".into(),
            reason: format!("{path}: {e}"),
        })?;
        self.with_connection(|conn| {
            let pool = StoragePool::lookup_by_name(conn, pool_name).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "uploadVolumeLookupPool".into(),
                    reason: e.to_string(),
                }
            })?;
            let vol = StorageVol::lookup_by_name(&pool, vol_name).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "uploadVolumeLookupVol".into(),
                    reason: e.to_string(),
                }
            })?;
            let stream = Stream::new(conn, 0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "uploadVolumeStream".into(),
                reason: e.to_string(),
            })?;
            vol.upload(&stream, 0, total, 0).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "uploadVolumeAttach".into(),
                    reason: e.to_string(),
                }
            })?;

            let mut buf = vec![0u8; chunk_size];
            let mut sent: u64 = 0;
            on_progress(0, total);
            loop {
                let n = file.read(&mut buf).map_err(|e| VirtManagerError::OperationFailed {
                    operation: "uploadVolumeRead".into(),
                    reason: e.to_string(),
                })?;
                if n == 0 { break; }
                let mut offset = 0;
                while offset < n {
                    let written = stream.send(&buf[offset..n]).map_err(|e| {
                        VirtManagerError::OperationFailed {
                            operation: "streamSend".into(),
                            reason: e.to_string(),
                        }
                    })?;
                    offset += written as usize;
                }
                sent += n as u64;
                on_progress(sent, total);
            }
            stream.finish().map_err(|e| VirtManagerError::OperationFailed {
                operation: "streamFinish".into(),
                reason: e.to_string(),
            })?;
            Ok(sent)
        })
    }

    // -- Secrets (libvirt-managed credentials, used for LUKS volumes,
    //    Ceph, iSCSI CHAP, vTPM persistence, etc.) --

    pub fn list_secrets(&self) -> Result<Vec<crate::libvirt::secrets::SecretInfo>, VirtManagerError> {
        use virt::secret::Secret;
        self.with_connection(|conn| {
            let secrets = conn.list_all_secrets(0).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "listAllSecrets".into(),
                    reason: e.to_string(),
                }
            })?;
            let mut out = Vec::with_capacity(secrets.len());
            for sec in &secrets {
                let xml = match sec.get_xml_desc(0) {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                let mut info = match crate::libvirt::secrets::parse_secret_xml(&xml) {
                    Some(i) => i,
                    None => continue,
                };
                // has_value: getValue is the only way; if private=yes,
                // libvirt refuses with VIR_ERR_INVALID_SECRET. Treat any
                // success as "yes", any error as "unknown but assume yes
                // since most secrets are populated immediately after define".
                info.has_value = match Secret::lookup_by_uuid_string(conn, &info.uuid) {
                    Ok(s) => s.get_value(0).is_ok(),
                    Err(_) => false,
                };
                out.push(info);
            }
            Ok(out)
        })
    }

    pub fn define_secret(
        &self,
        usage: crate::libvirt::secrets::SecretUsage,
        usage_id: Option<&str>,
        description: Option<&str>,
        ephemeral: bool,
        private: bool,
    ) -> Result<String, VirtManagerError> {
        use virt::secret::Secret;
        let xml = crate::libvirt::secrets::build_secret_xml(
            usage, usage_id, description, ephemeral, private,
        );
        self.with_connection(|conn| {
            let sec = Secret::define_xml(conn, &xml, 0).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "secretDefineXML".into(),
                    reason: e.to_string(),
                }
            })?;
            sec.get_uuid_string().map_err(|e| VirtManagerError::OperationFailed {
                operation: "secretGetUuid".into(),
                reason: e.to_string(),
            })
        })
    }

    pub fn set_secret_value(&self, uuid: &str, value: &[u8]) -> Result<(), VirtManagerError> {
        use virt::secret::Secret;
        self.with_connection(|conn| {
            let sec = Secret::lookup_by_uuid_string(conn, uuid).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "secretLookup".into(),
                    reason: e.to_string(),
                }
            })?;
            sec.set_value(value, 0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "secretSetValue".into(),
                reason: e.to_string(),
            })
        })
    }

    pub fn delete_secret(&self, uuid: &str) -> Result<(), VirtManagerError> {
        use virt::secret::Secret;
        self.with_connection(|conn| {
            let sec = Secret::lookup_by_uuid_string(conn, uuid).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "secretLookup".into(),
                    reason: e.to_string(),
                }
            })?;
            sec.undefine().map_err(|e| VirtManagerError::OperationFailed {
                operation: "secretUndefine".into(),
                reason: e.to_string(),
            })
        })
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
        // VIR_NETWORK_XML_INACTIVE=1 — return the persistent definition,
        // not the running snapshot. We want the editor to reflect what
        // the operator just modified (routes, DHCP host entries) even
        // for sections that don't propagate to live state until restart.
        // Live runtime state (active leases, autogen MAC) is not surfaced
        // by kraftwerk anyway.
        const VIR_NETWORK_XML_INACTIVE: u32 = 1;
        self.with_connection(|conn| {
            let net = Network::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupNetwork".into(),
                    reason: format!("network '{name}' not found"),
                }
            })?;
            net.get_xml_desc(VIR_NETWORK_XML_INACTIVE).map_err(|e| VirtManagerError::OperationFailed {
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

    /// Add or remove a per-host DHCP / DNS entry on a virtual network
    /// using `virNetworkUpdate`. Affects both the live dnsmasq state
    /// AND the persistent definition by default — a one-shot fix that
    /// virt-manager users have done with virsh net-update for years.
    ///
    /// `command`: 3 = ADD_LAST, 2 = DELETE.
    /// `section`: 4 = IP_DHCP_HOST, 10 = DNS_HOST,
    ///            5 = IP_DHCP_RANGE, 14 = FORWARD_INTERFACE (etc).
    pub fn network_update_section(
        &self,
        name: &str,
        command: u32,
        section: u32,
        xml_snippet: &str,
    ) -> Result<(), VirtManagerError> {
        use std::ffi::CString;
        use virt::network::Network;
        let snippet = CString::new(xml_snippet).map_err(|_| {
            VirtManagerError::OperationFailed {
                operation: "networkUpdate".into(),
                reason: "snippet has nul byte".into(),
            }
        })?;
        self.with_connection(|conn| {
            let net = Network::lookup_by_name(conn, name).map_err(|_| {
                VirtManagerError::OperationFailed {
                    operation: "lookupNetwork".into(),
                    reason: format!("network '{name}' not found"),
                }
            })?;
            // VIR_NETWORK_UPDATE_AFFECT_LIVE | _CONFIG = 3.
            // parentIndex = -1 (libvirt picks the only matching parent).
            let r = unsafe {
                virt_sys::virNetworkUpdate(
                    net.as_ptr(),
                    command,
                    section,
                    -1,
                    snippet.as_ptr(),
                    3,
                )
            };
            if r < 0 {
                return Err(VirtManagerError::OperationFailed {
                    operation: "virNetworkUpdate".into(),
                    reason: format!("returned {r}"),
                });
            }
            Ok(())
        })
    }

    /// Bundle the nested-virt state for a domain: host vendor, domain
    /// CPU mode, whether the domain XML implies nested, and the host
    /// kernel module's nested parameter.
    pub fn get_nested_virt_state(
        &self,
        name: &str,
    ) -> Result<crate::libvirt::nested_virt::NestedVirtState, VirtManagerError> {
        use crate::libvirt::cpu_tune_config;
        use crate::libvirt::nested_virt::{
            domain_nested_enabled, parse_host_vendor, NestedVirtState,
        };

        let caps = self.get_host_capabilities_xml()?;
        let vendor = parse_host_vendor(&caps);

        let xml = self.get_domain_xml(name, true)?;
        let snap = cpu_tune_config::parse(&xml)?;
        let cpu_mode = snap.cpu.mode.clone();
        let features: Vec<(String, String)> = snap
            .cpu
            .features
            .iter()
            .map(|f| (f.name.clone(), f.policy.clone()))
            .collect();
        let enabled_in_domain = domain_nested_enabled(vendor, &cpu_mode, &features);

        let enabled_in_host = self.read_host_nested_param(vendor)?;
        Ok(NestedVirtState {
            vendor,
            cpu_mode,
            enabled_in_domain,
            enabled_in_host,
        })
    }

    /// Toggle the vmx/svm CPU feature on a domain. No-op when the
    /// host vendor is unknown or when the mode is host-passthrough
    /// (passthrough already inherits). Persistent-only — VM has to
    /// reboot for the change to take effect.
    pub fn set_nested_virt(&self, name: &str, enable: bool) -> Result<(), VirtManagerError> {
        use crate::libvirt::cpu_tune_config::{self, CpuConfig, CpuFeature, CpuTunePatch};
        use crate::libvirt::nested_virt::parse_host_vendor;

        let caps = self.get_host_capabilities_xml()?;
        let vendor = parse_host_vendor(&caps);
        let Some(needed) = vendor.nested_feature() else {
            return Err(VirtManagerError::OperationFailed {
                operation: "setNestedVirt".into(),
                reason: "host CPU vendor unknown — cannot pick vmx vs svm".into(),
            });
        };

        let xml = self.get_domain_xml(name, true)?;
        let snap = cpu_tune_config::parse(&xml)?;

        if snap.cpu.mode == "host-passthrough" {
            return Err(VirtManagerError::OperationFailed {
                operation: "setNestedVirt".into(),
                reason: "domain uses host-passthrough — nested already inherits from host. Toggle the host kernel module instead.".into(),
            });
        }

        // Build a new CpuConfig with the feature added or removed.
        let mut new_features: Vec<CpuFeature> = snap
            .cpu
            .features
            .iter()
            .filter(|f| f.name != needed)
            .cloned()
            .collect();
        if enable {
            new_features.push(CpuFeature {
                name: needed.into(),
                policy: "require".into(),
            });
        }
        let new_cpu = CpuConfig { features: new_features, ..snap.cpu };
        let patch = CpuTunePatch {
            cpu: Some(new_cpu),
            ..Default::default()
        };
        self.apply_cpu_tune(name, &patch)
    }

    /// Read the host's libvirt capabilities XML (for vendor / arch /
    /// supported guest types). Cached at the libvirt-driver level so
    /// repeat calls are cheap.
    pub fn get_host_capabilities_xml(&self) -> Result<String, VirtManagerError> {
        self.with_connection(|conn| {
            conn.get_capabilities().map_err(|e| VirtManagerError::OperationFailed {
                operation: "getCapabilities".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Detect the host's `kvm_intel` / `kvm_amd` `nested` parameter via
    /// SSH (or local fs read for `qemu:///system`). Returns None when
    /// the path doesn't exist (vendor mismatch) or read fails.
    pub fn read_host_nested_param(&self, vendor: crate::libvirt::nested_virt::CpuVendor) -> Result<Option<bool>, VirtManagerError> {
        let Some(path) = vendor.nested_module_path() else {
            return Ok(None);
        };
        let uri = self.uri()?;
        // Reuse the qemu_log SSH helper's read pattern: spawn `cat path`
        // remotely, parse the result. Inline here so we don't fold the
        // qemu_log abstraction into something it isn't.
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::time::Duration;
        use wait_timeout::ChildExt;

        if let Some(target) = crate::libvirt::vnc_proxy::parse_ssh_target(&uri) {
            let remote_cmd = format!("cat {path}");
            let mut child = Command::new("ssh")
                .arg("-o").arg("BatchMode=yes")
                .arg("-o").arg("ConnectTimeout=5")
                .arg("-o").arg("StrictHostKeyChecking=accept-new")
                .arg(&target)
                .arg(&remote_cmd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "nestedSpawnSsh".into(),
                    reason: e.to_string(),
                })?;
            let status = match child.wait_timeout(Duration::from_secs(8)) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(None);
                }
                Err(_) => return Ok(None),
            };
            if !status.success() {
                return Ok(None);
            }
            let mut out = String::new();
            if let Some(mut s) = child.stdout.take() { let _ = s.read_to_string(&mut out); }
            Ok(Some(crate::libvirt::nested_virt::parse_nested_module_value(&out)))
        } else {
            // Local read.
            match std::fs::read_to_string(path) {
                Ok(s) => Ok(Some(crate::libvirt::nested_virt::parse_nested_module_value(&s))),
                Err(_) => Ok(None),
            }
        }
    }

    /// Get the URI we connected with. Used by helpers that need to
    /// re-derive the SSH target.
    pub fn uri(&self) -> Result<String, VirtManagerError> {
        self.with_connection(|conn| {
            conn.get_uri().map_err(|e| VirtManagerError::OperationFailed {
                operation: "connectGetUri".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Resolve the vTPM persistent-state directory and probe whether
    /// it exists on the hypervisor host. Pure read — never sudos.
    pub fn get_vtpm_info(&self, name: &str) -> Result<crate::libvirt::vtpm::VtpmInfo, VirtManagerError> {
        use crate::libvirt::{vtpm, virtio_devices};
        let xml = self.get_domain_xml(name, true)?;
        let uuid = self.with_connection(|conn| {
            let d = Self::lookup_domain(conn, name)?;
            d.get_uuid_string().map_err(|e| VirtManagerError::OperationFailed {
                operation: "domainGetUuid".into(),
                reason: e.to_string(),
            })
        })?;
        let tpm = virtio_devices::parse_tpm(&xml)?;
        let state_path = tpm.as_ref().and_then(|t| {
            if vtpm::has_persistent_state(t) {
                Some(vtpm::swtpm_state_path(&uuid, t.backend_version.as_deref()))
            } else {
                None
            }
        });
        let state_path_exists = match &state_path {
            Some(p) => self.probe_remote_path(p).ok(),
            None => None,
        };
        Ok(crate::libvirt::vtpm::VtpmInfo {
            uuid,
            tpm,
            state_path,
            state_path_exists,
        })
    }

    /// SSH `test -d <path>` against the connection's host. Returns Ok(true)
    /// when the directory exists, Ok(false) when missing. Errors when we
    /// can't even reach the host. Local URIs probe the local filesystem.
    fn probe_remote_path(&self, path: &str) -> Result<bool, VirtManagerError> {
        use std::process::{Command, Stdio};
        use std::time::Duration;
        use wait_timeout::ChildExt;

        let uri = self.uri()?;
        if let Some(target) = crate::libvirt::vnc_proxy::parse_ssh_target(&uri) {
            // Path is a libvirt-controlled prefix + uuid (hex+hyphens) +
            // a literal subdir name. No metacharacters possible — single
            // quoting belt-and-suspenders.
            let remote_cmd = format!("test -d '{path}'");
            let mut child = Command::new("ssh")
                .arg("-o").arg("BatchMode=yes")
                .arg("-o").arg("ConnectTimeout=5")
                .arg("-o").arg("StrictHostKeyChecking=accept-new")
                .arg(&target)
                .arg(&remote_cmd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "vtpmProbeSsh".into(),
                    reason: e.to_string(),
                })?;
            match child.wait_timeout(Duration::from_secs(8)) {
                Ok(Some(s)) => Ok(s.success()),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(VirtManagerError::OperationFailed {
                        operation: "vtpmProbeSsh".into(),
                        reason: "timed out".into(),
                    })
                }
                Err(e) => Err(VirtManagerError::OperationFailed {
                    operation: "vtpmProbeSsh".into(),
                    reason: e.to_string(),
                }),
            }
        } else {
            Ok(std::path::Path::new(path).is_dir())
        }
    }

    /// List all nwfilters defined on the hypervisor — built-in libvirt
    /// filters (clean-traffic, no-mac-spoofing, no-ip-spoofing, allow-arp,
    /// no-arp-spoofing, etc.) plus any user-defined ones.
    pub fn list_nw_filters(&self) -> Result<Vec<crate::models::nwfilter::NwFilterInfo>, VirtManagerError> {
        self.with_connection(|conn| {
            let filters = conn.list_all_nw_filters(0).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "listAllNWFilters".into(),
                    reason: e.to_string(),
                }
            })?;
            let mut out = Vec::with_capacity(filters.len());
            for f in &filters {
                let name = f.get_name().unwrap_or_default();
                let uuid = f.get_uuid_string().unwrap_or_default();
                out.push(crate::models::nwfilter::NwFilterInfo { name, uuid });
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        })
    }

    /// Fetch the XML for a single nwfilter by name. Read-only — for
    /// the inspect-this-filter view in the UI.
    pub fn get_nw_filter_xml(&self, name: &str) -> Result<String, VirtManagerError> {
        use virt::nwfilter::NWFilter;
        self.with_connection(|conn| {
            let f = NWFilter::lookup_by_name(conn, name).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "lookupNWFilter".into(),
                    reason: e.to_string(),
                }
            })?;
            f.get_xml_desc(0).map_err(|e| VirtManagerError::OperationFailed {
                operation: "nwFilterGetXMLDesc".into(),
                reason: e.to_string(),
            })
        })
    }

    /// Add a static `<route>` to a virtual network. libvirt has no
    /// virNetworkUpdate section for routes, so we rewrite the XML and
    /// redefine. Re-routing takes effect on the host immediately if the
    /// network is active; libvirt re-applies the iptables rules.
    pub fn add_network_route(
        &self,
        name: &str,
        route: &crate::libvirt::network_config::NetworkRoute,
    ) -> Result<(), VirtManagerError> {
        let xml = self.get_network_xml(name)?;
        let new_xml = crate::libvirt::network_config::add_route_to_network_xml(&xml, route);
        self.define_network(&new_xml)
    }

    /// Remove a matching static `<route>` from a virtual network.
    /// Match is on (family, address, prefix, gateway) — first hit wins.
    pub fn remove_network_route(
        &self,
        name: &str,
        route: &crate::libvirt::network_config::NetworkRoute,
    ) -> Result<(), VirtManagerError> {
        let xml = self.get_network_xml(name)?;
        let new_xml = crate::libvirt::network_config::remove_route_from_network_xml(&xml, route);
        self.define_network(&new_xml)
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

        let (graphics_type, has_serial, is_template) = match domain.get_xml_desc(0) {
            Ok(xml) => {
                let gfx = xml_helpers::extract_graphics_type(&xml).and_then(|s| match s.as_str() {
                    "vnc" => Some(GraphicsType::Vnc),
                    "spice" => Some(GraphicsType::Spice),
                    _ => None,
                });
                let serial = xml_helpers::has_serial_console(&xml);
                let tpl = crate::libvirt::templates::is_template(&xml);
                (gfx, serial, tpl)
            }
            Err(_) => (None, false, false),
        };

        Some(VmInfo {
            name,
            uuid,
            state,
            vcpus,
            memory_mb: memory_kb / 1024,
            graphics_type,
            has_serial,
            is_template,
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

    /// Read the current `<launchSecurity>` block from a domain. Returns
    /// Ok(None) when absent. Inactive XML is used so configuration shows
    /// up even when the guest is off.
    pub fn get_launch_security(&self, name: &str)
        -> Result<Option<crate::libvirt::launch_security::LaunchSecurityConfig>, VirtManagerError>
    {
        let xml = self.get_domain_xml(name, true)?;
        crate::libvirt::launch_security::parse_launch_security(&xml)
    }

    /// Enumerate active mediated devices on the hypervisor host (vGPUs,
    /// vfio-mdev). Each entry has the UUID needed to attach via
    /// `<hostdev type='mdev'>`.
    pub fn list_host_mdevs(&self)
        -> Result<Vec<crate::libvirt::hostdev::HostMdev>, VirtManagerError>
    {
        use virt::sys::VIR_CONNECT_LIST_NODE_DEVICES_CAP_MDEV;
        self.with_connection(|conn| {
            let devs = conn
                .list_all_node_devices(VIR_CONNECT_LIST_NODE_DEVICES_CAP_MDEV)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "listHostMdevs".into(),
                    reason: e.to_string(),
                })?;
            let mut out = Vec::with_capacity(devs.len());
            for d in &devs {
                if let Ok(xml) = d.get_xml_desc(0) {
                    if let Ok(parsed) = crate::libvirt::hostdev::parse_mdev_node_device(&xml) {
                        out.push(parsed);
                    }
                }
            }
            out.sort_by(|a, b| a.uuid.cmp(&b.uuid));
            Ok(out)
        })
    }

    /// Enumerate mdev TYPE catalogs across the host. We hit every
    /// node device that can host mdevs (libvirt's
    /// `CAP_MDEV_TYPES` flag) and parse the `mdev_types` capability
    /// out of its XML. Operators pick a type and create instances
    /// out-of-band (sysfs / mdevctl) — kraftwerk surfaces the
    /// catalog only.
    pub fn list_host_mdev_types(&self)
        -> Result<Vec<crate::libvirt::hostdev::MdevType>, VirtManagerError>
    {
        use virt::sys::VIR_CONNECT_LIST_NODE_DEVICES_CAP_MDEV_TYPES;
        self.with_connection(|conn| {
            let devs = conn
                .list_all_node_devices(VIR_CONNECT_LIST_NODE_DEVICES_CAP_MDEV_TYPES)
                .map_err(|e| VirtManagerError::OperationFailed {
                    operation: "listHostMdevTypes".into(),
                    reason: e.to_string(),
                })?;
            let mut out = Vec::new();
            for d in &devs {
                let parent_name = d.get_name().unwrap_or_default();
                if let Ok(xml) = d.get_xml_desc(0) {
                    if let Ok(types) = crate::libvirt::hostdev::parse_mdev_types(&parent_name, &xml) {
                        out.extend(types);
                    }
                }
            }
            out.sort_by(|a, b| (a.parent.as_str(), a.type_id.as_str())
                .cmp(&(b.parent.as_str(), b.type_id.as_str())));
            Ok(out)
        })
    }

    /// Clone a template domain into a new VM, optionally seeded by a
    /// cloud-init NoCloud ISO. Wraps `clone_domain`, then post-edits
    /// the cloned XML to (a) drop the template marker so the new VM
    /// is a regular guest, and (b) attach a fresh cloud-init seed ISO
    /// when `cloud_init` is provided.
    ///
    /// The seed ISO is written next to the cloned VM's primary disk
    /// (same directory) so it inherits whatever pool the disk lives in.
    pub fn clone_from_template(
        &self,
        template_name: &str,
        opts: &crate::libvirt::clone::CloneOptions,
        cloud_init: Option<&crate::libvirt::templates::CloudInitConfig>,
    ) -> Result<String, VirtManagerError> {
        // Caller may have toggled start_after, but we always start last
        // (after the seed ISO is attached) so cloud-init runs on first boot.
        let mut clone_opts = opts.clone();
        let user_wants_start = clone_opts.start_after;
        clone_opts.start_after = false;
        let new_name = self.clone_domain(template_name, &clone_opts)?;

        // Strip the template marker on the clone — clones are guests, not templates.
        if let Ok(cloned_xml) = self.get_domain_xml(&new_name, true) {
            let stripped = crate::libvirt::templates::remove_template_marker(&cloned_xml);
            if stripped != cloned_xml {
                let _ = self.define_domain_xml(&stripped);
            }
        }

        if let Some(ci) = cloud_init {
            let cloned_xml = self.get_domain_xml(&new_name, true)?;
            // Derive the seed dir from the first disk path in the clone.
            let disk_path = first_disk_source(&cloned_xml).ok_or_else(|| VirtManagerError::OperationFailed {
                operation: "cloudInitSeed".into(),
                reason: "cloned VM has no disk path to derive seed dir from".into(),
            })?;
            let dest_dir = std::path::Path::new(&disk_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/var/lib/libvirt/images".into());
            let iso_filename = format!("{new_name}-seed.iso");

            let hostname = ci
                .hostname
                .clone()
                .unwrap_or_else(|| new_name.clone());
            let meta = crate::libvirt::templates::build_meta_data(&new_name, &hostname);
            let user = crate::libvirt::templates::build_user_data(ci);
            let iso_path = self.build_cloud_init_iso(
                &dest_dir,
                &iso_filename,
                &meta,
                &user,
                ci.network_config.as_deref(),
            )?;

            // Splice the cdrom into the cloned XML at the first free
            // sda/sdb/sdc slot. virsh + libvirt accept duplicate slots
            // only across different buses.
            let target_dev = first_free_sd_slot(&cloned_xml);
            let cdrom_xml = crate::libvirt::templates::build_seed_iso_disk_xml(&iso_path, &target_dev);
            let new_xml = inject_disk_before_close(&cloned_xml, &cdrom_xml);
            self.define_domain_xml(&new_xml)?;
        }

        if user_wants_start {
            if let Err(e) = self.start_domain(&new_name) {
                log::warn!("template clone defined but start failed: {e}");
            }
        }
        Ok(new_name)
    }

    /// Resolve the curated image catalog against a storage pool's
    /// existing volumes — each entry comes back with `local_path`
    /// populated when its filename is already present.
    pub fn list_catalog_images(
        &self,
        pool_name: &str,
    ) -> Result<Vec<crate::libvirt::image_catalog::CatalogImageStatus>, VirtManagerError> {
        use crate::libvirt::image_catalog::{builtin_catalog, CatalogImageStatus};
        let vols = self.list_volumes(pool_name)?;
        let mut out = Vec::new();
        for img in builtin_catalog() {
            let hit = vols.iter().find(|v| v.name == img.filename);
            let (local_path, local_size_bytes) = match hit {
                Some(v) => (Some(v.path.clone()), Some(v.capacity)),
                None => (None, None),
            };
            out.push(CatalogImageStatus { image: img, local_path, local_size_bytes });
        }
        Ok(out)
    }

    /// Download a catalog image into the named pool's target dir over
    /// SSH. Uses curl on the hypervisor host. Refreshes the pool so
    /// libvirt picks up the new volume on success.
    pub fn download_catalog_image(
        &self,
        image_id: &str,
        pool_name: &str,
    ) -> Result<String, VirtManagerError> {
        use std::process::{Command, Stdio};
        use std::time::Duration;
        use wait_timeout::ChildExt;

        let img = crate::libvirt::image_catalog::find_image(image_id).ok_or_else(|| {
            VirtManagerError::OperationFailed {
                operation: "downloadImage".into(),
                reason: format!("unknown catalog id {image_id}"),
            }
        })?;
        let pool_cfg = self.get_pool_config(pool_name)?;
        let pool_path = pool_cfg.target_path.ok_or_else(|| VirtManagerError::OperationFailed {
            operation: "downloadImage".into(),
            reason: format!("pool {pool_name} has no target path (only `dir` pools are supported here)"),
        })?;
        let dest = format!("{}/{}", pool_path.trim_end_matches('/'), img.filename);

        let uri = self.uri()?;
        let target = crate::libvirt::vnc_proxy::parse_ssh_target(&uri).ok_or_else(|| {
            VirtManagerError::OperationFailed {
                operation: "downloadImage".into(),
                reason: "image download requires a qemu+ssh URI".into(),
            }
        })?;

        // -fLsS = fail on HTTP errors, follow redirects, silent except errors.
        // -o writes to a tmp file first; mv at the end to avoid a half-
        // downloaded file appearing in the pool.
        let cmd = format!(
            "set -eu; tmp=$(mktemp '{dest}.XXXXXX'); curl -fLsS -o \"$tmp\" '{url}'; mv \"$tmp\" '{dest}'; echo OK",
            dest = dest, url = img.url,
        );
        let mut child = Command::new("ssh")
            .arg("-o").arg("BatchMode=yes")
            .arg("-o").arg("ConnectTimeout=8")
            .arg(&target)
            .arg(&cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "downloadImageSpawn".into(),
                reason: e.to_string(),
            })?;
        let status = child
            .wait_timeout(Duration::from_secs(30 * 60)) // big images take a while
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "downloadImageWait".into(),
                reason: e.to_string(),
            })?
            .ok_or_else(|| VirtManagerError::OperationFailed {
                operation: "downloadImageWait".into(),
                reason: "timed out (>30 min)".into(),
            })?;
        if !status.success() {
            use std::io::Read;
            let mut errbuf = String::new();
            if let Some(mut s) = child.stderr.take() { let _ = s.read_to_string(&mut errbuf); }
            return Err(VirtManagerError::OperationFailed {
                operation: "downloadImage".into(),
                reason: format!("ssh+curl failed: {}", errbuf.trim()),
            });
        }

        // Make libvirt pick up the new volume.
        let _ = self.refresh_pool(pool_name);
        Ok(dest)
    }

    /// Toggle the kraftwerk template marker on a domain. Persistent
    /// only; the marker is preserved across libvirt restarts because
    /// it lives in the domain's `<metadata>` block.
    pub fn set_template_flag(&self, name: &str, mark: bool) -> Result<(), VirtManagerError> {
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = if mark {
            crate::libvirt::templates::add_template_marker(&xml)
        } else {
            crate::libvirt::templates::remove_template_marker(&xml)
        };
        self.define_domain_xml(&new_xml)
    }

    /// Domains marked as templates. Same return shape as `list_all_domains`,
    /// just filtered.
    pub fn list_templates(&self) -> Result<Vec<crate::models::vm::VmInfo>, VirtManagerError> {
        let all = self.list_all_domains()?;
        let mut out = Vec::new();
        for vm in all {
            if let Ok(xml) = self.get_domain_xml(&vm.name, true) {
                if crate::libvirt::templates::is_template(&xml) {
                    out.push(vm);
                }
            }
        }
        Ok(out)
    }

    /// Build a NoCloud cloud-init seed ISO on the hypervisor host and
    /// return its absolute path. Requires `genisoimage`, `xorrisofs`,
    /// or `mkisofs` to be on the host's PATH (one of those is on every
    /// libvirt host in practice). The ISO is written to `dest_dir`
    /// which must be a libvirt-managed pool path so the new domain
    /// can reference it.
    ///
    /// The contents (meta-data + user-data) are base64-encoded on the
    /// way over SSH so embedded newlines / quotes / metacharacters in
    /// hostnames + ssh keys can't break out of the shell context.
    pub fn build_cloud_init_iso(
        &self,
        dest_dir: &str,
        iso_filename: &str,
        meta_data: &str,
        user_data: &str,
        network_config: Option<&str>,
    ) -> Result<String, VirtManagerError> {
        use base64::Engine;
        use std::io::Write;
        use std::process::{Command, Stdio};
        use std::time::Duration;
        use wait_timeout::ChildExt;

        let uri = self.uri()?;
        let target = crate::libvirt::vnc_proxy::parse_ssh_target(&uri).ok_or_else(|| {
            VirtManagerError::OperationFailed {
                operation: "buildSeedIso".into(),
                reason: "cloud-init seed build requires a qemu+ssh URI".into(),
            }
        })?;

        let b64 = base64::engine::general_purpose::STANDARD;
        let meta_b64 = b64.encode(meta_data);
        let user_b64 = b64.encode(user_data);
        let network_b64 = network_config.map(|s| b64.encode(s));

        let iso_path = format!("{}/{}", dest_dir.trim_end_matches('/'), iso_filename);
        // Quote the path with single quotes — UUIDs and pool paths don't
        // contain quotes themselves so this is safe.
        let mut script = String::new();
        script.push_str("set -eu\n");
        script.push_str("d=$(mktemp -d /tmp/k-seed-XXXXXX)\n");
        script.push_str(&format!("echo {meta_b64} | base64 -d > $d/meta-data\n"));
        script.push_str(&format!("echo {user_b64} | base64 -d > $d/user-data\n"));
        if let Some(nw) = network_b64 {
            script.push_str(&format!("echo {nw} | base64 -d > $d/network-config\n"));
        }
        // genisoimage / xorrisofs / mkisofs — pick the first available.
        script.push_str(&format!(
            "if command -v genisoimage >/dev/null 2>&1; then iso=genisoimage; \
             elif command -v xorrisofs >/dev/null 2>&1; then iso=xorrisofs; \
             elif command -v mkisofs >/dev/null 2>&1; then iso=mkisofs; \
             else echo NO_ISO_TOOL >&2; exit 1; fi\n"
        ));
        script.push_str(&format!(
            "$iso -quiet -output '{iso_path}' -volid cidata -joliet -rock $d/meta-data $d/user-data"
        ));
        if network_config.is_some() {
            script.push_str(" $d/network-config");
        }
        script.push_str("\nrm -rf $d\n");
        script.push_str(&format!("echo OK_PATH={iso_path}\n"));

        let mut child = Command::new("ssh")
            .arg("-o").arg("BatchMode=yes")
            .arg("-o").arg("ConnectTimeout=5")
            .arg(&target)
            .arg("bash -s")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "spawnSshIso".into(),
                reason: e.to_string(),
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(script.as_bytes()).map_err(|e| {
                VirtManagerError::OperationFailed {
                    operation: "writeIsoScript".into(),
                    reason: e.to_string(),
                }
            })?;
        }
        let status = child
            .wait_timeout(Duration::from_secs(60))
            .map_err(|e| VirtManagerError::OperationFailed {
                operation: "waitIsoScript".into(),
                reason: e.to_string(),
            })?
            .ok_or_else(|| VirtManagerError::OperationFailed {
                operation: "waitIsoScript".into(),
                reason: "timed out".into(),
            })?;
        if !status.success() {
            let mut err_buf = String::new();
            if let Some(mut s) = child.stderr.take() {
                use std::io::Read;
                let _ = s.read_to_string(&mut err_buf);
            }
            return Err(VirtManagerError::OperationFailed {
                operation: "buildSeedIso".into(),
                reason: format!("ssh script failed: {err_buf}"),
            });
        }
        Ok(iso_path)
    }

    /// Initiate a live migration of `name` from this connection (the
    /// source) to `dest` (a separately-opened LibvirtConnection that
    /// points at the destination libvirtd).
    ///
    /// virDomainMigrate is synchronous in libvirt's API — this call
    /// blocks the calling thread for the entire transfer (potentially
    /// minutes for big guests). To avoid wedging the rest of the app
    /// behind the connection mutex while that runs, we clone both
    /// `Connect` handles via virConnectRef (Connect's Clone impl) and
    /// release the mutexes before issuing the migrate call. Other
    /// commands — including `migration_status` polling on the same
    /// source — can then proceed concurrently.
    pub fn migrate_to(
        &self,
        name: &str,
        dest: &LibvirtConnection,
        cfg: &crate::libvirt::migration::MigrationConfig,
    ) -> Result<(), VirtManagerError> {
        use crate::libvirt::migration::migrate_err;

        // Clone Connect refs while holding the mutexes briefly.
        let src_conn = self.with_connection(|c| Ok(c.clone()))?;
        let dst_conn = dest.with_connection(|c| Ok(c.clone()))?;

        // Now operate on the cloned handles without holding either
        // LibvirtConnection's internal mutex.
        let domain = Domain::lookup_by_name(&src_conn, name)
            .map_err(|_| VirtManagerError::DomainNotFound { name: name.into() })?;
        domain
            .migrate(&dst_conn, cfg.flags(), cfg.dest_name.as_deref(), cfg.dest_uri.as_deref(), cfg.bandwidth_mibs)
            .map(|_| ())
            .map_err(|e| migrate_err("migrate", e))
    }

    /// Read the current migration progress for a running domain. When
    /// no migration is in flight, the returned `phase` is `None`.
    pub fn migration_status(
        &self,
        name: &str,
    ) -> Result<crate::libvirt::migration::MigrationProgress, VirtManagerError> {
        use crate::libvirt::migration::MigrationProgress;
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            // 0 flags = current job (vs completed). When no job is
            // running libvirt returns OPERATION_INVALID; map that to
            // an empty progress rather than an error so polling code
            // doesn't have to special-case it.
            match domain.get_job_stats(0) {
                Ok(s) => Ok(MigrationProgress::from_job_stats(s)),
                Err(_) => Ok(MigrationProgress::default()),
            }
        })
    }

    /// Cancel the in-flight migration for `name`. Calls the raw
    /// `virDomainAbortJob` because the safe wrapper in this crate
    /// version doesn't expose it.
    pub fn cancel_migration(&self, name: &str) -> Result<(), VirtManagerError> {
        self.with_connection(|conn| {
            let domain = Self::lookup_domain(conn, name)?;
            // SAFETY: domain is a live virDomainPtr held by the safe
            // wrapper for the duration of this closure; the FFI call
            // returns a c_int which we map back to Rust's Result.
            let rc = unsafe { virt::sys::virDomainAbortJob(domain.as_ptr()) };
            if rc < 0 {
                Err(VirtManagerError::OperationFailed {
                    operation: "abortJob".into(),
                    reason: virt::error::Error::last_error().to_string(),
                })
            } else {
                Ok(())
            }
        })
    }

    /// Apply a SEV launchSecurity block (or remove the existing one).
    /// `cfg = None` strips the block; persistent only — SEV is fixed at
    /// guest launch and cannot be hot-toggled. SEV-SNP / TDX writes are
    /// rejected here because they need an operator-managed key bundle.
    pub fn set_launch_security(&self, name: &str,
        cfg: Option<&crate::libvirt::launch_security::LaunchSecurityConfig>)
        -> Result<(), VirtManagerError>
    {
        let xml = self.get_domain_xml(name, true)?;
        let new_xml = crate::libvirt::launch_security::apply_launch_security(&xml, cfg)?;
        self.define_domain_xml(&new_xml)
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


/// Extract (host, port) from a \`qemu+ssh://[user@]host[:port]/...\` URI.
/// Returns None for non-ssh URIs (e.g. \`qemu:///system\`).
fn parse_ssh_host_port(uri: &str) -> Option<(String, u16)> {
    let rest = uri.strip_prefix("qemu+ssh://")?;
    let authority = rest.split('/').next()?;
    // Strip optional \`user@\`.
    let host_part = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    // Split optional \`:port\`.
    let (host, port) = match host_part.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(22)),
        None => (host_part.to_string(), 22u16),
    };
    if host.is_empty() { None } else { Some((host, port)) }
}

/// Redact user-info from a URI before logging/reporting.
fn redact_uri(uri: &str) -> String {
    if let Some(rest) = uri.strip_prefix("qemu+ssh://") {
        if let Some(idx) = rest.find('@') {
            return format!("qemu+ssh://***@{}", &rest[idx + 1..]);
        }
    }
    uri.to_string()
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    #[test]
    fn parses_host_port_variants() {
        assert_eq!(parse_ssh_host_port("qemu+ssh://host/system"), Some(("host".into(), 22)));
        assert_eq!(parse_ssh_host_port("qemu+ssh://user@host/system"), Some(("host".into(), 22)));
        assert_eq!(parse_ssh_host_port("qemu+ssh://user@host:2222/system"), Some(("host".into(), 2222)));
        assert_eq!(parse_ssh_host_port("qemu:///system"), None);
    }
    #[test]
    fn redact_strips_userinfo() {
        assert_eq!(redact_uri("qemu+ssh://alice@host/system"), "qemu+ssh://***@host/system");
        assert_eq!(redact_uri("qemu:///system"), "qemu:///system");
    }

    #[test]
    fn first_disk_source_skips_cdrom() {
        let xml = r#"<domain><devices>
  <disk type='file' device='cdrom'><source file='/iso/inst.iso'/><readonly/></disk>
  <disk type='file' device='disk'><source file='/var/lib/libvirt/images/vm.qcow2'/></disk>
</devices></domain>"#;
        assert_eq!(first_disk_source(xml).as_deref(), Some("/var/lib/libvirt/images/vm.qcow2"));
    }

    #[test]
    fn first_free_sd_slot_picks_unused_letter() {
        let xml = "<domain><devices><disk><target dev='sda'/></disk><disk><target dev='sdb'/></disk></devices></domain>";
        assert_eq!(first_free_sd_slot(xml), "sdc");
    }

    #[test]
    fn inject_disk_lands_inside_devices_block() {
        let xml = "<domain>\n  <devices>\n    <disk/>\n  </devices>\n</domain>";
        let snippet = "<disk device='cdrom'/>";
        let out = inject_disk_before_close(xml, snippet);
        let inserted = out.find("<disk device='cdrom'").unwrap();
        let close = out.find("</devices>").unwrap();
        assert!(inserted < close);
    }
}


fn extract_attr_value(block: &str, tag: &str, attr: &str) -> Option<String> {
    let needle = format!("<{tag} ");
    let i = block.find(&needle)?;
    let rest = &block[i..];
    let close = rest.find("/>").or_else(|| rest.find('>'))?;
    let header = &rest[..close];
    for q in ['\'', '"'] {
        let an = format!("{}={}", attr, q);
        if let Some(s) = header.find(&an) {
            let after = &header[s + an.len()..];
            if let Some(e) = after.find(q) {
                return Some(after[..e].to_string());
            }
        }
    }
    None
}

/// Pull the first writable disk's source file path out of a domain XML.
/// Skips read-only / cdrom devices.
fn first_disk_source(xml: &str) -> Option<String> {
    let mut rest = xml;
    while let Some(i) = rest.find("<disk ") {
        rest = &rest[i..];
        let end = rest.find("</disk>").unwrap_or(rest.len());
        let block = &rest[..end];
        let cdrom = block.contains("device='cdrom'") || block.contains("device=\"cdrom\"");
        let readonly = block.contains("<readonly/>");
        if !cdrom && !readonly {
            if let Some(p) = extract_attr_value(block, "source", "file")
                .or_else(|| extract_attr_value(block, "source", "dev")) {
                return Some(p);
            }
        }
        rest = &rest[end..];
    }
    None
}

/// Find the first sd[a-z] target letter that's NOT already used by a
/// disk/cdrom in `xml`. The seed ISO needs a unique slot. Falls back
/// to "sdz" when somehow the entire alphabet is taken.
fn first_free_sd_slot(xml: &str) -> String {
    for c in b'a'..=b'z' {
        let dev = format!("sd{}", c as char);
        let needle1 = format!("dev='{dev}'");
        let needle2 = format!("dev=\"{dev}\"");
        if !xml.contains(&needle1) && !xml.contains(&needle2) {
            return dev;
        }
    }
    "sdz".to_string()
}

/// Splice an extra disk XML element into the `<devices>` block, just
/// before its closing tag.
fn inject_disk_before_close(xml: &str, disk_xml: &str) -> String {
    let close = match xml.find("</devices>") {
        Some(i) => i,
        None => return xml.to_string(),
    };
    let mut out = String::with_capacity(xml.len() + disk_xml.len() + 4);
    out.push_str(&xml[..close]);
    out.push_str("    ");
    out.push_str(disk_xml.trim_end_matches('\n'));
    out.push('\n');
    out.push_str(&xml[close..]);
    out
}

fn detect_volume_format(vol: &virt::storage_vol::StorageVol) -> Option<String> {
    let xml = vol.get_xml_desc(0).ok()?;
    let i = xml.find("<format type=")?;
    let rest = &xml[i + "<format type=".len()..];
    let q = rest.chars().next()?;
    if q != '"' && q != '\'' { return None; }
    let after = &rest[1..];
    let e = after.find(q)?;
    Some(after[..e].to_string())
}

