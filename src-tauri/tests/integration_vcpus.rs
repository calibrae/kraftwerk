//! Integration tests for vCPU editing (current + max).
//!
//! Configure via env vars:
//! - `KRAFTWERK_RAM_TEST_URI` — libvirt URI of a hypervisor with disposable
//!   shut-off VMs to mutate (e.g. `qemu+ssh://user@ramhost/system`).
//! - `KRAFTWERK_RAM_TEST_VM_A` (default: `vmtest-a`) and
//!   `KRAFTWERK_RAM_TEST_VM_B` (default: `vmtest-b`) — names of the two
//!   test VMs. Both must be shut off; max-vcpu edits require shutdown.
//!
//! Tests skip gracefully when the env var is unset.
//!
//! Each test resets the VM's vcpus (max + current) to a known baseline
//! on drop via a RAII guard so tests don't bleed into each other.

use kraftwerk_lib::libvirt::connection::LibvirtConnection;
use std::env;

fn ram_test_uri() -> Option<String> {
    env::var("KRAFTWERK_RAM_TEST_URI").ok().filter(|s| !s.is_empty())
}

fn vm_a() -> String {
    env::var("KRAFTWERK_RAM_TEST_VM_A").unwrap_or_else(|_| "vmtest-a".into())
}

fn vm_b() -> String {
    env::var("KRAFTWERK_RAM_TEST_VM_B").unwrap_or_else(|_| "vmtest-b".into())
}

fn connect_ram_host() -> Option<LibvirtConnection> {
    let uri = ram_test_uri()?;
    let conn = LibvirtConnection::new();
    conn.open(&uri).expect("Failed to connect to vCPU test host");
    Some(conn)
}

/// RAII guard that snapshots vcpus.max + vcpus.current at construction and
/// restores them on drop.
struct VcpuGuard<'a> {
    conn: &'a LibvirtConnection,
    vm: String,
    orig_max: u32,
    orig_cur: u32,
}

impl<'a> VcpuGuard<'a> {
    fn new(conn: &'a LibvirtConnection, vm: &str) -> Self {
        let cfg = conn
            .get_domain_config(vm, true)
            .expect("snapshot domain config");
        Self {
            conn,
            vm: vm.to_string(),
            orig_max: cfg.vcpus.max,
            orig_cur: cfg.vcpus.current,
        }
    }
}

impl Drop for VcpuGuard<'_> {
    fn drop(&mut self) {
        // Bump max first, then current, to preserve the libvirt invariant
        // that current <= max at all times.
        let _ = self.conn.set_max_vcpus(&self.vm, self.orig_max);
        let _ = self
            .conn
            .set_vcpus(&self.vm, self.orig_cur, false, true);
    }
}

#[test]
fn test_set_max_vcpus_bumps_max_on_wg_test_a() {
    let Some(conn) = connect_ram_host() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_a();
    let _guard = VcpuGuard::new(&conn, &vm);

    let orig = conn.get_domain_config(&vm, true).unwrap();
    let new_max = orig.vcpus.max * 2;

    conn.set_max_vcpus(&vm, new_max).expect("set_max_vcpus");

    let after = conn.get_domain_config(&vm, true).unwrap();
    assert_eq!(after.vcpus.max, new_max, "vcpus.max should reflect new max");
}

#[test]
fn test_set_current_vcpus_below_max_on_wg_test_b() {
    let Some(conn) = connect_ram_host() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_b();
    let _guard = VcpuGuard::new(&conn, &vm);

    let orig = conn.get_domain_config(&vm, true).unwrap();
    let target_cur = if orig.vcpus.max >= 2 { 1 } else { orig.vcpus.max };

    conn.set_vcpus(&vm, target_cur, false, true)
        .expect("set_vcpus");

    let after = conn.get_domain_config(&vm, true).unwrap();
    assert_eq!(after.vcpus.current, target_cur);
    assert_eq!(after.vcpus.max, orig.vcpus.max);
}

#[test]
fn test_bump_max_then_set_current_above_old_max() {
    let Some(conn) = connect_ram_host() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_a();
    let _guard = VcpuGuard::new(&conn, &vm);

    let orig = conn.get_domain_config(&vm, true).unwrap();
    let old_max = orig.vcpus.max;
    let new_max = old_max * 3;
    let new_cur = old_max * 2;

    conn.set_max_vcpus(&vm, new_max).expect("set_max_vcpus");
    conn.set_vcpus(&vm, new_cur, false, true).expect("set_vcpus");

    let after = conn.get_domain_config(&vm, true).unwrap();
    assert_eq!(after.vcpus.max, new_max);
    assert_eq!(after.vcpus.current, new_cur);
}

#[test]
fn test_set_current_above_max_without_bump_fails() {
    let Some(conn) = connect_ram_host() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_b();
    let _guard = VcpuGuard::new(&conn, &vm);

    let orig = conn.get_domain_config(&vm, true).unwrap();
    let too_big = orig.vcpus.max * 4;

    let r = conn.set_vcpus(&vm, too_big, false, true);
    assert!(
        r.is_err(),
        "set_vcpus above max should fail without a prior set_max_vcpus"
    );
}
