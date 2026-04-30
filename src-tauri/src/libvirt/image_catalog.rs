//! Cloud image catalog — a curated set of distro cloud images that
//! can be downloaded once per pool and used as the source for new VMs.
//!
//! The catalog is embedded in the binary (no manifest fetch). Each
//! entry points at the upstream "current" image URL — distro vendors
//! redirect those to the latest minor version, so we don't have to
//! ship a new release every time Fedora 41 → 42 happens. Trade-off:
//! sha256 verification is optional and the recorded size is a hint,
//! not authoritative.
//!
//! Download path is SSH + curl on the hypervisor host into a chosen
//! pool's directory. The file is then visible to libvirt after a
//! storage pool refresh.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageArch {
    X86_64,
    Aarch64,
}

impl ImageArch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogImage {
    /// Stable id used in commands. Format: `<distro>-<version>-<arch>`.
    pub id: String,
    /// Display label, e.g. "Fedora Cloud 41".
    pub label: String,
    pub distro: String,
    pub version: String,
    pub arch: ImageArch,
    /// Direct URL to the image file (qcow2 / raw).
    pub url: String,
    /// Suggested filename to write into the pool directory.
    pub filename: String,
    /// Image format ("qcow2" or "raw") — passed to libvirt + qemu-img.
    pub format: String,
    /// Approximate compressed size in bytes (UI hint, not enforced).
    pub size_hint_bytes: u64,
    /// Notes shown to the operator (cloud-init compatibility,
    /// minimum disk size to expand to, etc).
    pub notes: String,
}

/// The curated catalog. Sized small on purpose — we ship the four
/// distros that every libvirt user reaches for. Operators can drop
/// extra qcow2s into a pool by hand and they'll show up in the
/// existing volume list.
pub fn builtin_catalog() -> Vec<CatalogImage> {
    vec![
        CatalogImage {
            id: "fedora-current-x86_64".into(),
            label: "Fedora Cloud (current)".into(),
            distro: "fedora".into(),
            version: "current".into(),
            arch: ImageArch::X86_64,
            url: "https://download.fedoraproject.org/pub/fedora/linux/releases/41/Cloud/x86_64/images/Fedora-Cloud-Base-Generic-41-1.4.x86_64.qcow2".into(),
            filename: "Fedora-Cloud-Base-41.qcow2".into(),
            format: "qcow2".into(),
            size_hint_bytes: 460 * 1024 * 1024,
            notes: "cloud-init enabled. Default user `fedora`.".into(),
        },
        CatalogImage {
            id: "debian-12-x86_64".into(),
            label: "Debian 12 (Bookworm) genericcloud".into(),
            distro: "debian".into(),
            version: "12".into(),
            arch: ImageArch::X86_64,
            url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-amd64.qcow2".into(),
            filename: "debian-12-genericcloud-amd64.qcow2".into(),
            format: "qcow2".into(),
            size_hint_bytes: 320 * 1024 * 1024,
            notes: "cloud-init enabled. Default user `debian`.".into(),
        },
        CatalogImage {
            id: "ubuntu-2404-x86_64".into(),
            label: "Ubuntu 24.04 LTS (Noble) cloudimg".into(),
            distro: "ubuntu".into(),
            version: "24.04".into(),
            arch: ImageArch::X86_64,
            url: "https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img".into(),
            filename: "ubuntu-24.04-cloudimg.img".into(),
            format: "qcow2".into(),
            size_hint_bytes: 600 * 1024 * 1024,
            notes: "cloud-init enabled. Default user `ubuntu`.".into(),
        },
        CatalogImage {
            id: "alpine-edge-x86_64".into(),
            label: "Alpine (cloud, edge)".into(),
            distro: "alpine".into(),
            version: "edge".into(),
            arch: ImageArch::X86_64,
            url: "https://dl-cdn.alpinelinux.org/alpine/edge/releases/cloud/nocloud_alpine-edge-x86_64-bios-cloudinit-r0.qcow2".into(),
            filename: "alpine-edge-cloudinit.qcow2".into(),
            format: "qcow2".into(),
            size_hint_bytes: 90 * 1024 * 1024,
            notes: "cloud-init via tiny-cloud-init. Default user `alpine`.".into(),
        },
        CatalogImage {
            id: "fedora-current-aarch64".into(),
            label: "Fedora Cloud aarch64 (current)".into(),
            distro: "fedora".into(),
            version: "current".into(),
            arch: ImageArch::Aarch64,
            url: "https://download.fedoraproject.org/pub/fedora/linux/releases/41/Cloud/aarch64/images/Fedora-Cloud-Base-Generic-41-1.4.aarch64.qcow2".into(),
            filename: "Fedora-Cloud-Base-41-aarch64.qcow2".into(),
            format: "qcow2".into(),
            size_hint_bytes: 480 * 1024 * 1024,
            notes: "Apple Silicon hosts via `qemu-system-aarch64`.".into(),
        },
    ]
}

/// Look up a catalog entry by id.
pub fn find_image(id: &str) -> Option<CatalogImage> {
    builtin_catalog().into_iter().find(|i| i.id == id)
}

/// Enriched view of a catalog image relative to a specific storage
/// pool: have we downloaded it already, and at what path?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogImageStatus {
    pub image: CatalogImage,
    /// `Some(path)` when the image's filename is already in the pool.
    pub local_path: Option<String>,
    pub local_size_bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_distros() {
        let c = builtin_catalog();
        let distros: Vec<_> = c.iter().map(|i| i.distro.as_str()).collect();
        for d in &["fedora", "debian", "ubuntu", "alpine"] {
            assert!(distros.contains(d), "expected {d} in catalog");
        }
    }

    #[test]
    fn ids_are_unique() {
        let c = builtin_catalog();
        let mut seen = std::collections::HashSet::new();
        for img in &c {
            assert!(seen.insert(&img.id), "duplicate id {}", img.id);
        }
    }

    #[test]
    fn find_image_by_id() {
        assert!(find_image("fedora-current-x86_64").is_some());
        assert!(find_image("nope").is_none());
    }

    #[test]
    fn urls_are_https() {
        for img in builtin_catalog() {
            assert!(img.url.starts_with("https://"), "{} URL is not https", img.id);
        }
    }
}
