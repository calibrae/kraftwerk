//! vTPM persistent NVRAM helpers.
//!
//! The emulator (swtpm) backend keeps per-domain state under
//! `/var/lib/libvirt/swtpm/<domain-uuid>/tpm<version>/`. That directory
//! survives across guest reboots and holds the TPM's NVRAM blobs (EK,
//! SRK, sealed keys, PCRs that persist through resets, BitLocker keys,
//! LUKS unlock secrets, etc.).
//!
//! Operating on those files (backup / restore / reset) requires root —
//! they are owned by the `tss` / `swtpm` user and unreadable to the
//! libvirt SSH user. So this module's job is informational: surface
//! the path, expose ready-to-paste command snippets, and let the
//! operator run them out-of-band. We do not auto-sudo.

use serde::{Deserialize, Serialize};

use crate::libvirt::virtio_devices::TpmConfig;

/// Snapshot of vTPM state-on-disk for one domain. Computed, not stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VtpmInfo {
    /// Domain UUID (lowercase, hyphenated).
    pub uuid: String,
    /// `Some` when there's an emulator-backed TPM; the TPM config.
    pub tpm: Option<TpmConfig>,
    /// Path to the persistent state dir on the hypervisor host. `None`
    /// when there's no TPM, or the backend is not `emulator` (e.g.
    /// `passthrough` uses `/dev/tpm0` directly with no per-domain dir).
    pub state_path: Option<String>,
    /// Whether the directory exists on the host (probed via SSH `test
    /// -d`). `None` when we couldn't probe (no SSH, timeout, local
    /// driver). The directory only appears the first time the guest
    /// boots with the TPM defined, so absence on a never-started VM
    /// is normal.
    pub state_path_exists: Option<bool>,
}

/// Build the swtpm state directory path. libvirt's hardcoded layout
/// since 5.6.0 — see `src/qemu/qemu_tpm.c:qemuTPMEmulatorStorageBuildPath`.
///
/// `version`: the TpmConfig.backend_version string ("1.2" or "2.0"),
/// or `None` for the default ("2.0" → `tpm2`). "1.2" → `tpm1.2`.
pub fn swtpm_state_path(uuid: &str, version: Option<&str>) -> String {
    let v = match version.unwrap_or("2.0") {
        "1.2" => "tpm1.2",
        _ => "tpm2",
    };
    format!("/var/lib/libvirt/swtpm/{uuid}/{v}")
}

/// True when this TPM config has a persistent state directory worth
/// surfacing — i.e. the emulator backend.
pub fn has_persistent_state(tpm: &TpmConfig) -> bool {
    tpm.backend_model == "emulator"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tpm2_path_is_default() {
        let p = swtpm_state_path("d8a8d8e0-1234-5678-9abc-def012345678", None);
        assert_eq!(p, "/var/lib/libvirt/swtpm/d8a8d8e0-1234-5678-9abc-def012345678/tpm2");
        let p2 = swtpm_state_path("d8a8d8e0-1234-5678-9abc-def012345678", Some("2.0"));
        assert_eq!(p, p2);
    }

    #[test]
    fn tpm12_path_uses_tpm1_2_dir() {
        let p = swtpm_state_path("aaaa-bbbb", Some("1.2"));
        assert_eq!(p, "/var/lib/libvirt/swtpm/aaaa-bbbb/tpm1.2");
    }

    #[test]
    fn unknown_version_defaults_to_tpm2() {
        let p = swtpm_state_path("u", Some("9.9"));
        assert!(p.ends_with("/tpm2"));
    }

    #[test]
    fn only_emulator_has_persistent_state() {
        let emu = TpmConfig {
            model: "tpm-crb".into(),
            backend_model: "emulator".into(),
            backend_version: Some("2.0".into()),
            source_path: None,
        };
        assert!(has_persistent_state(&emu));

        let pass = TpmConfig {
            model: "tpm-tis".into(),
            backend_model: "passthrough".into(),
            backend_version: None,
            source_path: Some("/dev/tpm0".into()),
        };
        assert!(!has_persistent_state(&pass));
    }
}
