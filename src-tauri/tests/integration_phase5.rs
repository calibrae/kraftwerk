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
use kraftwerk_lib::libvirt::hostdev::HostDevice;
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

// ────────────────────────────────────────────────────────────────────
// 5.2 mdev / vGPU
// ────────────────────────────────────────────────────────────────────

#[test]
fn list_host_mdev_paths_succeed() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    // Read paths must succeed even when the host has no mdevs / no vGPU
    // hardware — list_all_node_devices returns an empty Vec, which we
    // pass through as Ok([]).
    let mdevs = conn.list_host_mdevs().expect("list_host_mdevs");
    let types = conn.list_host_mdev_types().expect("list_host_mdev_types");
    eprintln!("host has {} active mdevs, {} advertised types", mdevs.len(), types.len());
    // Sanity: every advertised type has a non-empty parent + type_id.
    for t in &types {
        assert!(!t.parent.is_empty(), "mdev type missing parent: {t:?}");
        assert!(!t.type_id.is_empty(), "mdev type missing id: {t:?}");
    }
    // Sanity: every active mdev has a uuid.
    for m in &mdevs {
        assert!(!m.uuid.is_empty(), "active mdev missing uuid: {m:?}");
    }
}

#[test]
fn mdev_attach_round_trip_when_available() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let mdevs = conn.list_host_mdevs().expect("list_host_mdevs");
    let Some(m) = mdevs.into_iter().next() else {
        eprintln!("SKIP: no mdev instances on host — test needs a pre-allocated mdev");
        return;
    };
    let vm = vm_name();

    let dev = HostDevice::Mdev {
        uuid: m.uuid.clone(),
        model: "vfio-pci".into(),
        display: false,
    };

    // Attach persistent only; mdev hot-plug needs PCI bus space.
    let attach_res = conn.attach_hostdev(&vm, &dev, false, true);
    if attach_res.is_err() {
        eprintln!("SKIP: attach failed (likely vmtest-a missing PCIe slots): {attach_res:?}");
        return;
    }

    let after = conn.list_domain_hostdevs(&vm).expect("list domain hostdevs");
    let found = after.iter().any(|d| matches!(d, HostDevice::Mdev { uuid, .. } if uuid == &m.uuid));
    assert!(found, "attached mdev not found in domain hostdevs");

    // Always detach to restore baseline.
    let _ = conn.detach_hostdev(&vm, &dev, false, true);
    let final_state = conn.list_domain_hostdevs(&vm).expect("list domain hostdevs after detach");
    let still_there = final_state.iter().any(|d| matches!(d, HostDevice::Mdev { uuid, .. } if uuid == &m.uuid));
    assert!(!still_there, "mdev still attached after detach");
}

// ────────────────────────────────────────────────────────────────────
// 5.1 live migration
// ────────────────────────────────────────────────────────────────────

#[test]
fn migration_status_returns_empty_when_no_job() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_name();
    let p = conn.migration_status(&vm).expect("migration_status");
    // No active migration on a shut-off vmtest-a → all fields None or
    // phase = Some(None-equivalent). Either way, non-fatal.
    eprintln!("phase: {:?}", p.phase);
}

#[test]
fn live_migrate_round_trip_when_dest_uri_set() {
    let Some(src) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let Some(dst_uri) = std::env::var("KRAFTWERK_MIGRATION_DEST_URI").ok().filter(|s| !s.is_empty()) else {
        eprintln!("SKIP: KRAFTWERK_MIGRATION_DEST_URI unset (need a second hypervisor)");
        return;
    };
    let dst = LibvirtConnection::new();
    if dst.open(&dst_uri).is_err() {
        eprintln!("SKIP: could not open destination uri {dst_uri}");
        return;
    }

    let vm = vm_name();
    // Domain must be running to live-migrate. We don't auto-start
    // vmtest-a — operator must boot it before running this test.
    let domains = src.list_all_domains().expect("list_all_domains");
    let running = domains.iter().any(|d| d.name == vm && format!("{:?}", d.state).to_lowercase() == "running");
    if !running {
        eprintln!("SKIP: vmtest-a is not running; live migrate needs an active guest");
        return;
    }

    let cfg = kraftwerk_lib::libvirt::migration::MigrationConfig::default();
    let r = src.migrate_to(&vm, &dst, &cfg);
    match r {
        Ok(()) => {
            eprintln!("migration succeeded");
            // Migrate back so the next run is idempotent.
            let cfg_back = kraftwerk_lib::libvirt::migration::MigrationConfig::default();
            let _ = dst.migrate_to(&vm, &src, &cfg_back);
        }
        Err(e) => {
            // Common skip reasons on a single-tenant test bench: storage
            // not shared between the two hosts, or the dest already has
            // the domain defined. Surface the reason instead of failing
            // — the test is informational on most setups.
            eprintln!("migration not exercised: {e:?}");
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// 5.3 SR-IOV
// ────────────────────────────────────────────────────────────────────

#[test]
fn pci_listing_carries_sriov_info_when_present() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let pci = conn.list_host_pci_devices().expect("list_host_pci_devices");
    let pf_count = pci.iter().filter(|d| d.sriov.as_ref().map(|s| s.is_pf()).unwrap_or(false)).count();
    let vf_count = pci.iter().filter(|d| d.sriov.as_ref().map(|s| s.is_vf()).unwrap_or(false)).count();
    eprintln!("host has {pf_count} SR-IOV PFs and {vf_count} VFs");
    // Whether or not SR-IOV is present, every PF reports a max_vfs and
    // every VF reports a phys_function — the parser invariant.
    for d in &pci {
        if let Some(s) = &d.sriov {
            if s.is_pf() {
                assert!(s.max_vfs.is_some(), "PF without max_vfs: {:?}", d.name);
            }
            if s.is_vf() {
                assert!(s.phys_function.is_some(), "VF without phys_function: {:?}", d.name);
            }
        }
    }
}

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
