//! Build libvirt domain XML from structured creation parameters.
//!
//! Kept deliberately minimal: only what the creation wizard needs. Editing
//! an existing domain's XML round-trip is a separate concern (domain_config).

use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;

/// Where the VM's root disk comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiskSource {
    /// Create a new volume in the given pool.
    NewVolume {
        pool_name: String,
        name: String,
        /// Virtual capacity in bytes.
        capacity_bytes: u64,
        /// "qcow2" or "raw"
        format: String,
    },
    /// Attach an existing volume by path.
    ExistingPath { path: String, format: String },
}

/// How the VM connects to the network.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NetworkSource {
    /// Libvirt-managed network (virtual).
    Network { name: String },
    /// Existing host bridge (L2 direct).
    Bridge { name: String },
    /// No network.
    None,
}

/// Install media — optional ISO to boot from.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallMedia {
    /// Absolute path on the hypervisor host to the ISO file.
    pub iso_path: Option<String>,
}

/// Full parameter set for creating a new domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainBuildParams {
    pub name: String,
    pub memory_mb: u64,
    pub vcpus: u32,
    /// OS type hint: "linux" | "windows" | "bsd"
    pub os_type: String,
    /// Machine type, e.g. "q35"
    pub machine_type: String,
    /// Architecture, e.g. "x86_64"
    pub arch: String,
    /// "bios" or "efi"
    pub firmware: String,
    /// Virtio/sata/ide for the root disk
    pub disk_bus: String,
    /// NIC model: virtio/e1000e/rtl8139/etc.
    pub nic_model: String,
    /// Video model: virtio/qxl/vga/cirrus
    pub video_model: String,
    pub disk_source: DiskSource,
    pub network: NetworkSource,
    #[serde(default)]
    pub install_media: InstallMedia,
    /// Graphics: "vnc", "spice", or "none"
    #[serde(default = "default_graphics")]
    pub graphics: String,
}

fn default_graphics() -> String { "vnc".into() }

/// Build the domain XML string.
pub fn build_domain_xml(p: &DomainBuildParams) -> String {
    let mut xml = String::from("<domain type='kvm'>\n");
    xml.push_str(&format!("  <name>{}</name>\n", escape_xml(&p.name)));
    xml.push_str(&format!(
        "  <memory unit='MiB'>{}</memory>\n  <currentMemory unit='MiB'>{}</currentMemory>\n",
        p.memory_mb, p.memory_mb
    ));
    xml.push_str(&format!("  <vcpu placement='static'>{}</vcpu>\n", p.vcpus));

    // <os>
    xml.push_str("  <os>\n");
    xml.push_str(&format!(
        "    <type arch='{}' machine='{}'>hvm</type>\n",
        escape_xml(&p.arch),
        escape_xml(&p.machine_type),
    ));
    if p.firmware.eq_ignore_ascii_case("efi") {
        xml.push_str("    <loader readonly='yes' type='pflash'>/usr/share/OVMF/OVMF_CODE.fd</loader>\n");
    }
    // If an ISO is provided, boot cdrom first
    if p.install_media.iso_path.is_some() {
        xml.push_str("    <boot dev='cdrom'/>\n");
    }
    xml.push_str("    <boot dev='hd'/>\n");
    xml.push_str("  </os>\n");

    xml.push_str("  <features>\n    <acpi/>\n    <apic/>\n  </features>\n");
    xml.push_str("  <cpu mode='host-passthrough' check='none'/>\n");
    xml.push_str("  <clock offset='utc'/>\n");
    xml.push_str("  <on_poweroff>destroy</on_poweroff>\n");
    xml.push_str("  <on_reboot>restart</on_reboot>\n");
    xml.push_str("  <on_crash>destroy</on_crash>\n");

    xml.push_str("  <devices>\n");
    xml.push_str("    <emulator>/usr/bin/qemu-system-x86_64</emulator>\n");

    // Root disk
    write_root_disk(&mut xml, &p.disk_source, &p.disk_bus);

    // Install CD-ROM
    if let Some(iso) = &p.install_media.iso_path {
        xml.push_str("    <disk type='file' device='cdrom'>\n");
        xml.push_str("      <driver name='qemu' type='raw'/>\n");
        xml.push_str(&format!("      <source file='{}'/>\n", escape_xml(iso)));
        xml.push_str("      <target dev='sda' bus='sata'/>\n");
        xml.push_str("      <readonly/>\n");
        xml.push_str("    </disk>\n");
    }

    // Network
    write_network(&mut xml, &p.network, &p.nic_model);

    // Console / serial (always present — we need it for our serial console)
    xml.push_str("    <serial type='pty'>\n      <target type='isa-serial' port='0'/>\n    </serial>\n");
    xml.push_str("    <console type='pty'>\n      <target type='serial' port='0'/>\n    </console>\n");

    // Input (tablet gives absolute pointer positioning for guests)
    xml.push_str("    <input type='tablet' bus='usb'/>\n");
    xml.push_str("    <input type='keyboard' bus='ps2'/>\n");

    // Graphics
    match p.graphics.as_str() {
        "none" => {}
        "spice" => {
            xml.push_str("    <graphics type='spice' autoport='yes' listen='127.0.0.1'/>\n");
        }
        _ => {
            xml.push_str("    <graphics type='vnc' port='-1' autoport='yes' listen='127.0.0.1'/>\n");
        }
    }

    // Video
    xml.push_str(&format!(
        "    <video>\n      <model type='{}'/>\n    </video>\n",
        escape_xml(&p.video_model),
    ));

    // USB controller (virtio-based models use usb-ehci + xhci)
    xml.push_str("    <controller type='usb' model='qemu-xhci'/>\n");

    xml.push_str("  </devices>\n");
    xml.push_str("</domain>\n");
    xml
}

fn write_root_disk(xml: &mut String, source: &DiskSource, bus: &str) {
    let dev = match bus {
        "virtio" => "vda",
        "sata" | "scsi" => "sda",
        "ide" => "hda",
        _ => "vda",
    };

    match source {
        DiskSource::NewVolume { .. } | DiskSource::ExistingPath { .. } => {
            let (path, format) = match source {
                DiskSource::NewVolume { pool_name: _, name: _, format, .. } => {
                    // For new volumes, the path is filled in after volume creation by caller.
                    // Callers should replace DiskSource::NewVolume with ExistingPath after
                    // calling create_volume. But to keep build_domain_xml pure, emit a
                    // placeholder that will fail validation if used as-is.
                    //
                    // In the wizard flow: create volume first, then substitute path here.
                    return write_new_disk_placeholder(xml, dev, bus, format);
                }
                DiskSource::ExistingPath { path, format } => (path.as_str(), format.as_str()),
            };
            xml.push_str("    <disk type='file' device='disk'>\n");
            xml.push_str(&format!("      <driver name='qemu' type='{}'/>\n", escape_xml(format)));
            xml.push_str(&format!("      <source file='{}'/>\n", escape_xml(path)));
            xml.push_str(&format!(
                "      <target dev='{}' bus='{}'/>\n",
                escape_xml(dev),
                escape_xml(bus),
            ));
            xml.push_str("    </disk>\n");
        }
    }
}

fn write_new_disk_placeholder(xml: &mut String, dev: &str, bus: &str, format: &str) {
    // Emit a disk with no source — caller must resolve it before defining.
    xml.push_str("    <disk type='file' device='disk'>\n");
    xml.push_str(&format!("      <driver name='qemu' type='{}'/>\n", escape_xml(format)));
    xml.push_str("      <source file='__PENDING__'/>\n");
    xml.push_str(&format!(
        "      <target dev='{}' bus='{}'/>\n",
        escape_xml(dev),
        escape_xml(bus),
    ));
    xml.push_str("    </disk>\n");
}

fn write_network(xml: &mut String, source: &NetworkSource, model: &str) {
    match source {
        NetworkSource::Network { name } => {
            xml.push_str("    <interface type='network'>\n");
            xml.push_str(&format!("      <source network='{}'/>\n", escape_xml(name)));
            xml.push_str(&format!("      <model type='{}'/>\n", escape_xml(model)));
            xml.push_str("    </interface>\n");
        }
        NetworkSource::Bridge { name } => {
            xml.push_str("    <interface type='bridge'>\n");
            xml.push_str(&format!("      <source bridge='{}'/>\n", escape_xml(name)));
            xml.push_str(&format!("      <model type='{}'/>\n", escape_xml(model)));
            xml.push_str("    </interface>\n");
        }
        NetworkSource::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_params() -> DomainBuildParams {
        DomainBuildParams {
            name: "test-vm".into(),
            memory_mb: 2048,
            vcpus: 2,
            os_type: "linux".into(),
            machine_type: "q35".into(),
            arch: "x86_64".into(),
            firmware: "bios".into(),
            disk_bus: "virtio".into(),
            nic_model: "virtio".into(),
            video_model: "virtio".into(),
            disk_source: DiskSource::ExistingPath {
                path: "/var/lib/libvirt/images/test.qcow2".into(),
                format: "qcow2".into(),
            },
            network: NetworkSource::Network { name: "default".into() },
            install_media: InstallMedia::default(),
            graphics: "vnc".into(),
        }
    }

    #[test]
    fn builds_basic_linux_vm() {
        let xml = build_domain_xml(&linux_params());
        assert!(xml.contains("<domain type='kvm'>"));
        assert!(xml.contains("<name>test-vm</name>"));
        assert!(xml.contains("<vcpu placement='static'>2</vcpu>"));
        assert!(xml.contains("<memory unit='MiB'>2048</memory>"));
    }

    #[test]
    fn emits_bios_by_default() {
        let xml = build_domain_xml(&linux_params());
        assert!(!xml.contains("<loader"));
    }

    #[test]
    fn emits_efi_loader_when_firmware_efi() {
        let mut p = linux_params();
        p.firmware = "efi".into();
        let xml = build_domain_xml(&p);
        assert!(xml.contains("<loader readonly='yes' type='pflash'>"));
    }

    #[test]
    fn uses_vda_for_virtio_bus() {
        let xml = build_domain_xml(&linux_params());
        assert!(xml.contains("dev='vda'"));
        assert!(xml.contains("bus='virtio'"));
    }

    #[test]
    fn uses_sda_for_sata_bus() {
        let mut p = linux_params();
        p.disk_bus = "sata".into();
        let xml = build_domain_xml(&p);
        assert!(xml.contains("dev='sda'"));
        assert!(xml.contains("bus='sata'"));
    }

    #[test]
    fn attaches_iso_and_adds_boot_cdrom() {
        let mut p = linux_params();
        p.install_media.iso_path = Some("/iso/fedora.iso".into());
        let xml = build_domain_xml(&p);
        assert!(xml.contains("<source file='/iso/fedora.iso'/>"));
        assert!(xml.contains("<boot dev='cdrom'/>"));
        assert!(xml.contains("device='cdrom'"));
    }

    #[test]
    fn no_iso_means_no_cdrom_boot() {
        let xml = build_domain_xml(&linux_params());
        assert!(!xml.contains("<boot dev='cdrom'/>"));
        assert!(!xml.contains("device='cdrom'"));
    }

    #[test]
    fn emits_libvirt_network_source() {
        let xml = build_domain_xml(&linux_params());
        assert!(xml.contains("<interface type='network'>"));
        assert!(xml.contains("<source network='default'/>"));
    }

    #[test]
    fn emits_bridge_network_source() {
        let mut p = linux_params();
        p.network = NetworkSource::Bridge { name: "br0".into() };
        let xml = build_domain_xml(&p);
        assert!(xml.contains("<interface type='bridge'>"));
        assert!(xml.contains("<source bridge='br0'/>"));
    }

    #[test]
    fn no_network_means_no_interface() {
        let mut p = linux_params();
        p.network = NetworkSource::None;
        let xml = build_domain_xml(&p);
        assert!(!xml.contains("<interface"));
    }

    #[test]
    fn emits_serial_console() {
        let xml = build_domain_xml(&linux_params());
        assert!(xml.contains("<serial type='pty'>"));
        assert!(xml.contains("<console type='pty'>"));
    }

    #[test]
    fn graphics_vnc_by_default() {
        let xml = build_domain_xml(&linux_params());
        assert!(xml.contains("<graphics type='vnc'"));
    }

    #[test]
    fn graphics_spice_when_selected() {
        let mut p = linux_params();
        p.graphics = "spice".into();
        let xml = build_domain_xml(&p);
        assert!(xml.contains("<graphics type='spice'"));
    }

    #[test]
    fn graphics_none_emits_no_graphics() {
        let mut p = linux_params();
        p.graphics = "none".into();
        let xml = build_domain_xml(&p);
        assert!(!xml.contains("<graphics"));
    }

    #[test]
    fn new_volume_emits_pending_placeholder() {
        let mut p = linux_params();
        p.disk_source = DiskSource::NewVolume {
            pool_name: "default".into(),
            name: "new.qcow2".into(),
            capacity_bytes: 10 * 1024 * 1024 * 1024,
            format: "qcow2".into(),
        };
        let xml = build_domain_xml(&p);
        // Sentinel that callers must replace after volume creation
        assert!(xml.contains("__PENDING__"));
    }

    #[test]
    fn escapes_injection_in_name() {
        let mut p = linux_params();
        p.name = "evil'><drop/>".into();
        let xml = build_domain_xml(&p);
        assert!(!xml.contains("<drop/>"));
        assert!(xml.contains("&apos;") || xml.contains("&quot;"));
    }

    #[test]
    fn escapes_injection_in_iso_path() {
        let mut p = linux_params();
        p.install_media.iso_path = Some("/iso/a'><inject/>.iso".into());
        let xml = build_domain_xml(&p);
        assert!(!xml.contains("<inject/>"));
    }

    #[test]
    fn roundtrips_through_parser() {
        use crate::libvirt::domain_config::parse;
        let xml = build_domain_xml(&linux_params());
        let cfg = parse(&xml).unwrap();
        assert_eq!(cfg.name, "test-vm");
        assert_eq!(cfg.vcpus.max, 2);
        assert_eq!(cfg.memory.mb(), 2048);
        assert_eq!(cfg.os.machine.as_deref(), Some("q35"));
    }
}
