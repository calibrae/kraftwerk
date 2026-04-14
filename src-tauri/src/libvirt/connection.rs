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
