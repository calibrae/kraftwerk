//! Integration tests against the testhost hypervisor.
//!
//! These tests require SSH access to testhost (testuser@testhost) with key-based auth.
//! Run with: cargo test --test integration_testhost
//!
//! SAFETY: Only fedora-workstation is used for lifecycle tests.
//! Production VMs are NEVER modified.

use virtmanager_rs_lib::libvirt::connection::LibvirtConnection;
use virtmanager_rs_lib::models::vm::{GraphicsType, VmState};

const JOLYNE_URI: &str = "qemu+ssh://testuser@testhost/system";

/// Known production VMs that must NEVER be modified.
const PROD_VMS: &[&str] = &[
    "example-broker",
    "example-firewall",
    "example-controller",
    "example-serial",
];

const TEST_VM: &str = "fedora-workstation";

fn connect_testhost() -> LibvirtConnection {
    let conn = LibvirtConnection::new();
    conn.open(JOLYNE_URI).expect("Failed to connect to testhost");
    assert!(conn.is_connected());
    conn
}

// ─── Connection tests ───

#[test]
fn test_connect_and_disconnect() {
    let conn = connect_testhost();
    assert!(conn.is_connected());
    conn.close();
    assert!(!conn.is_connected());
}

#[test]
fn test_connect_invalid_uri_fails() {
    let conn = LibvirtConnection::new();
    let result = conn.open("qemu+ssh://nonexistent-host-12345/system");
    assert!(result.is_err());
}

#[test]
fn test_hostname() {
    let conn = connect_testhost();
    let hostname = conn.hostname().expect("Failed to get hostname");
    assert!(!hostname.is_empty(), "Hostname should not be empty");
    println!("Hypervisor hostname: {hostname}");
}

// ─── Domain listing tests ───

#[test]
fn test_list_all_domains() {
    let conn = connect_testhost();
    let domains = conn.list_all_domains().expect("Failed to list domains");

    assert!(!domains.is_empty(), "Should have at least one VM");
    println!("Found {} domains:", domains.len());
    for vm in &domains {
        println!(
            "  {} [{}] - {} vCPUs, {} MB, gfx={:?}, serial={}",
            vm.name,
            vm.state.display_name(),
            vm.vcpus,
            vm.memory_mb,
            vm.graphics_type,
            vm.has_serial
        );
    }
}

#[test]
fn test_known_vms_present() {
    let conn = connect_testhost();
    let domains = conn.list_all_domains().expect("Failed to list domains");
    let names: Vec<&str> = domains.iter().map(|d| d.name.as_str()).collect();

    for expected in PROD_VMS {
        assert!(
            names.contains(expected),
            "Expected VM '{expected}' not found in domain list"
        );
    }
    assert!(
        names.contains(&TEST_VM),
        "Test VM '{TEST_VM}' not found"
    );
}

#[test]
fn test_vm_info_fields_populated() {
    let conn = connect_testhost();
    let domains = conn.list_all_domains().expect("Failed to list domains");

    for vm in &domains {
        assert!(!vm.name.is_empty(), "VM name should not be empty");
        assert!(!vm.uuid.is_empty(), "VM UUID should not be empty");
        assert!(vm.vcpus > 0, "VM {} should have at least 1 vCPU", vm.name);
        assert!(vm.memory_mb > 0, "VM {} should have memory", vm.name);
    }
}

#[test]
fn test_prod_brokers_has_spice() {
    let conn = connect_testhost();
    let domains = conn.list_all_domains().expect("Failed to list domains");
    let broker = domains.iter().find(|d| d.name == "example-broker").unwrap();
    assert_eq!(broker.graphics_type, Some(GraphicsType::Spice));
    assert_eq!(broker.state, VmState::Running);
}

#[test]
fn test_example-firewall_has_vnc() {
    let conn = connect_testhost();
    let domains = conn.list_all_domains().expect("Failed to list domains");
    let example-firewall = domains.iter().find(|d| d.name == "example-firewall").unwrap();
    assert_eq!(example-firewall.graphics_type, Some(GraphicsType::Vnc));
    assert_eq!(example-firewall.state, VmState::Running);
}

#[test]
fn test_hass_has_serial() {
    let conn = connect_testhost();
    let domains = conn.list_all_domains().expect("Failed to list domains");
    let hass = domains.iter().find(|d| d.name == "example-serial").unwrap();
    assert!(hass.has_serial, "hass should have serial console");
}

// ─── Domain XML tests ───

#[test]
fn test_get_domain_xml() {
    let conn = connect_testhost();
    let xml = conn.get_domain_xml(TEST_VM, false).expect("Failed to get XML");
    assert!(xml.contains("<domain"), "XML should contain <domain");
    assert!(xml.contains(TEST_VM), "XML should contain VM name");
    println!("Domain XML length: {} bytes", xml.len());
}

#[test]
fn test_get_domain_xml_inactive() {
    let conn = connect_testhost();
    let xml = conn.get_domain_xml(TEST_VM, true).expect("Failed to get inactive XML");
    assert!(xml.contains("<domain"), "Inactive XML should contain <domain");
}

#[test]
fn test_get_domain_xml_nonexistent_fails() {
    let conn = connect_testhost();
    let result = conn.get_domain_xml("this-vm-does-not-exist-12345", false);
    assert!(result.is_err());
}

// ─── Lifecycle tests (fedora-workstation ONLY) ───

#[test]
fn test_fedora_lifecycle_suspend_resume() {
    let conn = connect_testhost();

    // Verify it's running first
    let domains = conn.list_all_domains().unwrap();
    let vm = domains.iter().find(|d| d.name == TEST_VM).unwrap();
    if vm.state != VmState::Running {
        println!("Skipping lifecycle test: {TEST_VM} is not running");
        return;
    }

    // Suspend
    conn.suspend_domain(TEST_VM).expect("Failed to suspend");
    let domains = conn.list_all_domains().unwrap();
    let vm = domains.iter().find(|d| d.name == TEST_VM).unwrap();
    assert_eq!(vm.state, VmState::Paused, "VM should be paused after suspend");

    // Resume
    conn.resume_domain(TEST_VM).expect("Failed to resume");
    let domains = conn.list_all_domains().unwrap();
    let vm = domains.iter().find(|d| d.name == TEST_VM).unwrap();
    assert_eq!(vm.state, VmState::Running, "VM should be running after resume");
}

// ─── Safety: verify we never touch prod VMs ───

#[test]
fn test_prod_vms_all_running() {
    let conn = connect_testhost();
    let domains = conn.list_all_domains().unwrap();

    for prod_name in PROD_VMS {
        let vm = domains.iter().find(|d| d.name == *prod_name);
        assert!(
            vm.is_some(),
            "Production VM '{prod_name}' should exist"
        );
        assert_eq!(
            vm.unwrap().state,
            VmState::Running,
            "Production VM '{prod_name}' should be running"
        );
    }
}

// ─── Console tests ───

#[test]
fn test_open_console_on_hass() {
    // example-serial has a serial console and is always running
    let conn = connect_testhost();

    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let received = Arc::new(Mutex::new(Vec::<u8>::new()));
    let received_clone = received.clone();

    let session = conn.with_console("example-serial", move |data| {
        received_clone.lock().unwrap().extend_from_slice(&data);
    });

    assert!(session.is_ok(), "Should open console on example-serial");
    let mut session = session.unwrap();

    assert!(session.is_active(), "Session should be active after open");

    // Send a newline to trigger a prompt or output
    let sent = session.send(b"\n");
    assert!(sent.is_ok(), "Should be able to send data");

    // Wait briefly for response
    std::thread::sleep(Duration::from_secs(2));

    let data = received.lock().unwrap();
    println!("Received {} bytes from hass console", data.len());
    if !data.is_empty() {
        let text = String::from_utf8_lossy(&data);
        println!("Console output: {:?}", &text[..text.len().min(200)]);
    }

    // Close
    session.close();
    assert!(!session.is_active(), "Session should be inactive after close");
}

#[test]
fn test_open_console_on_nonexistent_domain_fails() {
    let conn = connect_testhost();
    let result = conn.with_console("this-vm-does-not-exist-12345", |_| {});
    assert!(result.is_err());
}

#[test]
fn test_console_send_after_close_fails() {
    let conn = connect_testhost();
    let mut session = conn.with_console("example-serial", |_| {}).unwrap();
    session.close();
    let result = session.send(b"test");
    assert!(result.is_err(), "Send after close should fail");
}

// ─── Domain config parsing tests ───

#[test]
fn test_parse_fedora_workstation_config() {
    let conn = connect_testhost();
    let cfg = conn
        .get_domain_config(TEST_VM, false)
        .expect("Should parse domain config");

    assert_eq!(cfg.name, TEST_VM);
    assert!(!cfg.uuid.is_empty());
    assert!(cfg.memory.kib > 0, "Memory should be populated");
    assert!(cfg.vcpus.max > 0, "vCPUs should be populated");
    assert!(
        !cfg.os.arch.as_deref().unwrap_or("").is_empty(),
        "Architecture should be set"
    );

    println!(
        "  Parsed: name={}, mem={}MiB, vcpus={}, cpu_mode={}, machine={:?}, firmware={}",
        cfg.name,
        cfg.memory.mb(),
        cfg.vcpus.max,
        cfg.cpu.mode,
        cfg.os.machine,
        cfg.os.firmware,
    );
}

#[test]
fn test_parse_all_vms_succeeds() {
    let conn = connect_testhost();
    let domains = conn.list_all_domains().unwrap();

    for vm in &domains {
        let cfg = conn
            .get_domain_config(&vm.name, false)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", vm.name, e));
        assert_eq!(cfg.name, vm.name);
        assert!(!cfg.uuid.is_empty());
    }
}
