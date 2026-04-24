//! Integration tests for memory editing (current + max RAM).
//!
//! Configure via env vars:
//! - `KRAFTWERK_RAM_TEST_URI` — libvirt URI of a hypervisor with disposable
//!   shut-off VMs to mutate (e.g. `qemu+ssh://user@polnareff/system`).
//! - `KRAFTWERK_RAM_TEST_VM_A` (default: `wg-test-a`) and
//!   `KRAFTWERK_RAM_TEST_VM_B` (default: `wg-test-b`) — names of the two
//!   test VMs. Both must be shut off; max-memory edits require shutdown.
//!
//! Tests skip gracefully when the env var is unset or the VMs aren't
//! present in the expected state.
//!
//! Each test resets the VM's memory + currentMemory to a known baseline
//! on drop via a RAII guard so tests don't bleed into each other.

use kraftwerk_lib::libvirt::connection::LibvirtConnection;
use std::env;

fn ram_test_uri() -> Option<String> {
    env::var("KRAFTWERK_RAM_TEST_URI").ok().filter(|s| !s.is_empty())
}

fn vm_a() -> String {
    env::var("KRAFTWERK_RAM_TEST_VM_A").unwrap_or_else(|_| "wg-test-a".into())
}

fn vm_b() -> String {
    env::var("KRAFTWERK_RAM_TEST_VM_B").unwrap_or_else(|_| "wg-test-b".into())
}

fn connect_ram_host() -> Option<LibvirtConnection> {
    let uri = ram_test_uri()?;
    let conn = LibvirtConnection::new();
    conn.open(&uri).expect("Failed to connect to RAM test host");
    Some(conn)
}

/// RAII guard that snapshots memory + currentMemory at construction and
/// restores them on drop.
struct MemGuard<'a> {
    conn: &'a LibvirtConnection,
    vm: String,
    orig_max_kib: u64,
    orig_cur_kib: u64,
}

impl<'a> MemGuard<'a> {
    fn new(conn: &'a LibvirtConnection, vm: &str) -> Self {
        let cfg = conn
            .get_domain_config(vm, true)
            .expect("snapshot domain config");
        Self {
            conn,
            vm: vm.to_string(),
            orig_max_kib: cfg.memory.kib,
            orig_cur_kib: cfg.current_memory.kib,
        }
    }
}

impl Drop for MemGuard<'_> {
    fn drop(&mut self) {
        // Always restore max first (may be raising above current on drop, or
        // lowering if we raised during the test — either way, bump first
        // before current to stay within libvirt's invariants).
        let _ = self.conn.set_max_memory(&self.vm, self.orig_max_kib);
        let _ = self
            .conn
            .set_memory(&self.vm, self.orig_cur_kib, false, true);
    }
}

#[test]
fn test_set_max_memory_bumps_max_on_wg_test_a() {
    let Some(conn) = connect_ram_host() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_a();
    let _guard = MemGuard::new(&conn, &vm);

    let orig = conn.get_domain_config(&vm, true).unwrap();
    let new_max_kib = orig.memory.kib * 2;

    conn.set_max_memory(&vm, new_max_kib)
        .expect("set_max_memory");

    let after = conn.get_domain_config(&vm, true).unwrap();
    assert_eq!(after.memory.kib, new_max_kib, "memory should reflect new max");
}

#[test]
fn test_set_current_memory_below_max_on_wg_test_b() {
    let Some(conn) = connect_ram_host() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_b();
    let _guard = MemGuard::new(&conn, &vm);

    let orig = conn.get_domain_config(&vm, true).unwrap();
    // Drop current to half the max (rounded to 1 KiB multiple; wg-test-b
    // starts at 512 MiB = 524288 KiB, half is 262144).
    let target_cur_kib = orig.memory.kib / 2;

    conn.set_memory(&vm, target_cur_kib, false, true)
        .expect("set_memory");

    let after = conn.get_domain_config(&vm, true).unwrap();
    assert_eq!(after.current_memory.kib, target_cur_kib);
    // Max should remain unchanged.
    assert_eq!(after.memory.kib, orig.memory.kib);
}

#[test]
fn test_bump_max_then_set_current_above_old_max() {
    let Some(conn) = connect_ram_host() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_a();
    let _guard = MemGuard::new(&conn, &vm);

    let orig = conn.get_domain_config(&vm, true).unwrap();
    let old_max_kib = orig.memory.kib;
    let new_max_kib = old_max_kib * 3;
    let new_cur_kib = old_max_kib * 2; // above old max, below new max

    // Must bump max first, else set_memory would be rejected.
    conn.set_max_memory(&vm, new_max_kib)
        .expect("set_max_memory");
    conn.set_memory(&vm, new_cur_kib, false, true)
        .expect("set_memory");

    let after = conn.get_domain_config(&vm, true).unwrap();
    assert_eq!(after.memory.kib, new_max_kib);
    assert_eq!(after.current_memory.kib, new_cur_kib);
}

#[test]
fn test_set_current_above_max_without_bump_fails() {
    let Some(conn) = connect_ram_host() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_b();
    let _guard = MemGuard::new(&conn, &vm);

    let orig = conn.get_domain_config(&vm, true).unwrap();
    let too_big_kib = orig.memory.kib * 4;

    // libvirt should reject this: currentMemory cannot exceed memory.
    let r = conn.set_memory(&vm, too_big_kib, false, true);
    assert!(
        r.is_err(),
        "set_memory above max should fail without a prior set_max_memory"
    );
}
