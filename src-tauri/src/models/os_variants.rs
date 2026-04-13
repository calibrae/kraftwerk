//! Per-OS device defaults used by the VM creation wizard.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsVariantDefaults {
    pub disk_bus: String,
    pub nic_model: String,
    pub video_model: String,
    pub machine_type: String,
    /// "bios" or "efi"
    pub firmware: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsVariant {
    pub id: String,
    pub label: String,
    /// "linux" | "windows" | "bsd"
    pub os_type: String,
    pub defaults: OsVariantDefaults,
}

pub fn all_variants() -> Vec<OsVariant> {
    let linux = OsVariantDefaults {
        disk_bus: "virtio".into(),
        nic_model: "virtio".into(),
        video_model: "virtio".into(),
        machine_type: "q35".into(),
        firmware: "bios".into(),
    };
    let windows_10 = OsVariantDefaults {
        disk_bus: "sata".into(),
        nic_model: "e1000e".into(),
        video_model: "qxl".into(),
        machine_type: "q35".into(),
        firmware: "efi".into(),
    };
    let windows_11 = OsVariantDefaults {
        disk_bus: "virtio".into(),
        nic_model: "e1000e".into(),
        video_model: "qxl".into(),
        machine_type: "q35".into(),
        firmware: "efi".into(),
    };
    let freebsd = OsVariantDefaults {
        disk_bus: "virtio".into(),
        nic_model: "virtio".into(),
        video_model: "virtio".into(),
        machine_type: "q35".into(),
        firmware: "bios".into(),
    };

    vec![
        OsVariant { id: "fedora".into(),         label: "Fedora".into(),                os_type: "linux".into(),   defaults: linux.clone() },
        OsVariant { id: "ubuntu".into(),         label: "Ubuntu".into(),                os_type: "linux".into(),   defaults: linux.clone() },
        OsVariant { id: "debian".into(),         label: "Debian".into(),                os_type: "linux".into(),   defaults: linux.clone() },
        OsVariant { id: "centos".into(),         label: "CentOS".into(),                os_type: "linux".into(),   defaults: linux.clone() },
        OsVariant { id: "rhel".into(),           label: "Red Hat Enterprise Linux".into(), os_type: "linux".into(), defaults: linux.clone() },
        OsVariant { id: "generic-linux".into(),  label: "Generic Linux".into(),         os_type: "linux".into(),   defaults: linux },
        OsVariant { id: "windows10".into(),      label: "Windows 10".into(),            os_type: "windows".into(), defaults: windows_10.clone() },
        OsVariant { id: "windows11".into(),      label: "Windows 11".into(),            os_type: "windows".into(), defaults: windows_11 },
        OsVariant { id: "generic-windows".into(), label: "Generic Windows".into(),      os_type: "windows".into(), defaults: windows_10 },
        OsVariant { id: "freebsd".into(),        label: "FreeBSD".into(),               os_type: "bsd".into(),     defaults: freebsd },
    ]
}

pub fn defaults_for(variant_id: &str) -> OsVariantDefaults {
    all_variants()
        .into_iter()
        .find(|v| v.id == variant_id)
        .map(|v| v.defaults)
        .unwrap_or_else(|| OsVariantDefaults {
            disk_bus: "virtio".into(),
            nic_model: "virtio".into(),
            video_model: "virtio".into(),
            machine_type: "q35".into(),
            firmware: "bios".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_has_expected_count() {
        assert_eq!(all_variants().len(), 10);
    }

    #[test]
    fn fedora_uses_virtio_bios() {
        let d = defaults_for("fedora");
        assert_eq!(d.disk_bus, "virtio");
        assert_eq!(d.firmware, "bios");
    }

    #[test]
    fn windows10_uses_sata_efi() {
        let d = defaults_for("windows10");
        assert_eq!(d.disk_bus, "sata");
        assert_eq!(d.firmware, "efi");
        assert_eq!(d.nic_model, "e1000e");
    }

    #[test]
    fn windows11_uses_virtio_efi() {
        let d = defaults_for("windows11");
        assert_eq!(d.disk_bus, "virtio");
        assert_eq!(d.firmware, "efi");
    }

    #[test]
    fn unknown_variant_returns_generic_defaults() {
        let d = defaults_for("unknown-os-99");
        assert_eq!(d.disk_bus, "virtio");
        assert_eq!(d.machine_type, "q35");
    }

    #[test]
    fn serializes_variant_to_json() {
        let v = &all_variants()[0];
        let json = serde_json::to_string(v).unwrap();
        assert!(json.contains("\"id\":\"fedora\""));
        assert!(json.contains("\"os_type\":\"linux\""));
    }

    #[test]
    fn linux_variants_all_share_defaults() {
        let variants = all_variants();
        let linux_ids = ["fedora", "ubuntu", "debian", "centos", "rhel", "generic-linux"];
        for id in &linux_ids {
            let v = variants.iter().find(|v| v.id == *id).unwrap();
            assert_eq!(v.os_type, "linux");
            assert_eq!(v.defaults.disk_bus, "virtio");
            assert_eq!(v.defaults.firmware, "bios");
        }
    }
}
