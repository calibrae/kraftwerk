//! Integration tests for phase 5 features.
//!
//! 5.4 nested virtualization: read state, toggle, restore.
//! 5.5 launch security: parse current block; round-trip apply when the
//!     host advertises SEV (else skipped — most CI/test boxes don't).
//! 5.6 vTPM info: get_vtpm_info wires through and computes the path.
//!
//! Configure via env vars:
//! - `KRAFTWERK_RAM_TEST_URI` — libvirt URI of a hypervisor with a
//!   disposable shut-off VM. Reused from the memory tests.
//! - `KRAFTWERK_RAM_TEST_VM_A` (default `vmtest-a`) — the test VM.
//!
//! All write paths use RAII guards so a failed assertion still rolls
//! the domain XML back to its starting state.

use kraftwerk_lib::libvirt::connection::LibvirtConnection;
use kraftwerk_lib::libvirt::launch_security::{
    LaunchSecurityConfig, LaunchSecurityKind, LaunchSecurityKindWrap,
};
use std::env;

fn test_uri() -> Option<String> {
    env::var("KRAFTWERK_RAM_TEST_URI").ok().filter(|s| !s.is_empty())
}

fn vm_name() -> String {
    env::var("KRAFTWERK_RAM_TEST_VM_A").unwrap_or_else(|_| "vmtest-a".into())
}

fn connect() -> Option<LibvirtConnection> {
    let uri = test_uri()?;
    let conn = LibvirtConnection::new();
    conn.open(&uri).expect("connection.open");
    Some(conn)
}

// ────────────────────────────────────────────────────────────────────
// 5.4 nested virtualization
// ────────────────────────────────────────────────────────────────────

struct NestedGuard<'a> {
    conn: &'a LibvirtConnection,
    vm: String,
    original: bool,
    cpu_mode: String,
}
impl Drop for NestedGuard<'_> {
    fn drop(&mut self) {
        if self.cpu_mode == "host-passthrough" { return; }
        let _ = self.conn.set_nested_virt(&self.vm, self.original);
    }
}

#[test]
fn nested_virt_state_round_trip() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_name();

    let state = conn.get_nested_virt_state(&vm).expect("get_nested_virt_state");
    // Vendor must be one of known values.
    let vendor_attr = serde_json::to_string(&state.vendor).unwrap();
    assert!(["\"intel\"", "\"amd\"", "\"unknown\""].contains(&vendor_attr.as_str()));

    if matches!(state.vendor, kraftwerk_lib::libvirt::nested_virt::CpuVendor::Unknown) {
        eprintln!("SKIP: host vendor unknown — toggle requires intel/amd");
        return;
    }
    if state.cpu_mode == "host-passthrough" {
        eprintln!("SKIP: vmtest-a uses host-passthrough — nested inherits, no domain toggle");
        return;
    }

    let _guard = NestedGuard {
        conn: &conn,
        vm: vm.clone(),
        original: state.enabled_in_domain,
        cpu_mode: state.cpu_mode.clone(),
    };

    // Flip and verify.
    conn.set_nested_virt(&vm, !state.enabled_in_domain).expect("set_nested_virt flip");
    let flipped = conn.get_nested_virt_state(&vm).expect("get after flip");
    assert_eq!(flipped.enabled_in_domain, !state.enabled_in_domain);

    // Flip back (also verified by guard, but explicit assertion is louder).
    conn.set_nested_virt(&vm, state.enabled_in_domain).expect("set_nested_virt restore");
    let back = conn.get_nested_virt_state(&vm).expect("get after restore");
    assert_eq!(back.enabled_in_domain, state.enabled_in_domain);
}

// ────────────────────────────────────────────────────────────────────
// 5.5 launch security
// ────────────────────────────────────────────────────────────────────

struct LaunchSecGuard<'a> {
    conn: &'a LibvirtConnection,
    vm: String,
    original: Option<LaunchSecurityConfig>,
}
impl Drop for LaunchSecGuard<'_> {
    fn drop(&mut self) {
        let _ = self.conn.set_launch_security(&self.vm, self.original.as_ref());
    }
}

#[test]
fn launch_security_read_passes_when_absent() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_name();
    // Just exercise the read path. We don't assert None because the
    // operator may have left a launchSecurity block in the test VM —
    // just that the call succeeds and produces a parseable Option.
    let _ = conn.get_launch_security(&vm).expect("get_launch_security");
}

#[test]
fn launch_security_sev_round_trip_when_supported() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_name();

    // Pull host capabilities to decide whether SEV is even possible here.
    let caps_xml = conn.get_host_capabilities_xml().expect("host caps");
    // Quick-and-dirty: domain caps gives us the structured cbitpos.
    // get_domain_capabilities is the structured path.
    let dc = conn.get_domain_capabilities(None, None, None, None);
    let Ok(dc) = dc else {
        eprintln!("SKIP: domain capabilities unavailable: {dc:?}");
        return;
    };
    if !dc.features.sev_supported || dc.features.sev_cbitpos.is_none() {
        eprintln!("SKIP: host doesn't advertise SEV (cbitpos absent). caps len {}", caps_xml.len());
        return;
    }
    let cbit = dc.features.sev_cbitpos.unwrap();
    let rpb = dc.features.sev_reduced_phys_bits.unwrap_or(1);

    let original = conn.get_launch_security(&vm).expect("read original");
    let _guard = LaunchSecGuard {
        conn: &conn,
        vm: vm.clone(),
        original,
    };

    // Apply a SEV block.
    let cfg = LaunchSecurityConfig {
        kind: Some(LaunchSecurityKindWrap(LaunchSecurityKind::Sev)),
        cbitpos: Some(cbit),
        reduced_phys_bits: Some(rpb),
        policy: Some("0x0003".into()),
        ..Default::default()
    };
    conn.set_launch_security(&vm, Some(&cfg)).expect("set SEV");
    let after = conn.get_launch_security(&vm).expect("get after set").expect("block present");
    assert_eq!(after.kind.unwrap().0, LaunchSecurityKind::Sev);
    assert_eq!(after.cbitpos, Some(cbit));
    assert_eq!(after.policy.as_deref(), Some("0x0003"));

    // Strip and verify.
    conn.set_launch_security(&vm, None).expect("strip SEV");
    let stripped = conn.get_launch_security(&vm).expect("get after strip");
    assert!(stripped.is_none(), "block should be gone, was {stripped:?}");
}

// ────────────────────────────────────────────────────────────────────
// 5.6 vTPM info
// ────────────────────────────────────────────────────────────────────

#[test]
fn vtpm_info_returns_uuid_and_path_when_emulator() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_name();

    let info = conn.get_vtpm_info(&vm).expect("get_vtpm_info");
    // UUID should be non-empty hex+hyphens.
    assert!(!info.uuid.is_empty());
    assert!(info.uuid.contains('-'));

    match &info.tpm {
        Some(t) if t.backend_model == "emulator" => {
            let path = info.state_path.as_deref().expect("emulator → path");
            assert!(path.starts_with("/var/lib/libvirt/swtpm/"));
            assert!(path.contains(&info.uuid));
            // state_path_exists may be Some(true), Some(false), or None
            // depending on whether the VM has booted with TPM enabled.
            // All three are valid — just exercising the probe.
        }
        Some(t) => {
            eprintln!("vmtest-a TPM backend is {} — no persistent state expected", t.backend_model);
            assert!(info.state_path.is_none());
        }
        None => {
            eprintln!("vmtest-a has no TPM — state_path must be None");
            assert!(info.state_path.is_none());
        }
    }
}
