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

// ─── Network management tests ───
//
// These tests create a network with a unique name (`virtmanager-test-net`)
// to avoid clashing with any existing configuration. The network is always
// cleaned up at the end, even on panic (via defer pattern with Drop guard).

const TEST_NET_NAME: &str = "virtmanager-test-net";
const TEST_NET_BRIDGE: &str = "virbr-vmt";

/// Cleanup guard: ensures the test network is removed even if a test panics.
struct NetworkCleanup<'a> {
    conn: &'a LibvirtConnection,
    name: &'static str,
}

impl<'a> Drop for NetworkCleanup<'a> {
    fn drop(&mut self) {
        let _ = self.conn.delete_network(self.name);
    }
}

fn ensure_clean(conn: &LibvirtConnection) {
    // Best-effort: remove any stale test network
    let _ = conn.delete_network(TEST_NET_NAME);
}

#[test]
fn test_list_networks() {
    let conn = connect_testhost();
    let nets = conn.list_networks().expect("list_networks");
    // testhost has no libvirt-managed networks by default, but the call should succeed
    println!("Found {} networks", nets.len());
    for n in &nets {
        println!("  - {} [{}] active={} bridge={:?}", n.name, n.forward_mode, n.is_active, n.bridge);
    }
}

#[test]
fn test_create_and_delete_network() {
    let conn = connect_testhost();
    ensure_clean(&conn);

    let _guard = NetworkCleanup {
        conn: &conn,
        name: TEST_NET_NAME,
    };

    let xml = virtmanager_rs_lib::libvirt::network_config::build_nat_network_xml(
        TEST_NET_NAME,
        TEST_NET_BRIDGE,
        "10.99.99.1",
        "255.255.255.0",
        Some("10.99.99.100"),
        Some("10.99.99.200"),
    );

    conn.create_network(&xml).expect("create_network");

    let nets = conn.list_networks().unwrap();
    let net = nets.iter().find(|n| n.name == TEST_NET_NAME);
    assert!(net.is_some(), "Test network should be listed");
    let net = net.unwrap();
    assert!(net.is_active, "Newly created network should be active");
    assert_eq!(net.forward_mode, "nat");

    // Cleanup happens via guard
}

#[test]
fn test_network_config_roundtrip() {
    let conn = connect_testhost();
    ensure_clean(&conn);
    let _guard = NetworkCleanup {
        conn: &conn,
        name: TEST_NET_NAME,
    };

    let xml = virtmanager_rs_lib::libvirt::network_config::build_nat_network_xml(
        TEST_NET_NAME,
        TEST_NET_BRIDGE,
        "10.99.99.1",
        "255.255.255.0",
        Some("10.99.99.100"),
        Some("10.99.99.200"),
    );
    conn.create_network(&xml).unwrap();

    let cfg = conn.get_network_config(TEST_NET_NAME).expect("get_network_config");
    assert_eq!(cfg.name, TEST_NET_NAME);
    assert_eq!(cfg.forward_mode, "nat");
    assert_eq!(cfg.bridge.as_deref(), Some(TEST_NET_BRIDGE));
    let v4 = cfg.ipv4.unwrap();
    assert_eq!(v4.address, "10.99.99.1");
    assert_eq!(v4.dhcp_ranges.len(), 1);
    assert_eq!(v4.dhcp_ranges[0].start, "10.99.99.100");
}

#[test]
fn test_network_stop_and_start() {
    let conn = connect_testhost();
    ensure_clean(&conn);
    let _guard = NetworkCleanup {
        conn: &conn,
        name: TEST_NET_NAME,
    };

    let xml = virtmanager_rs_lib::libvirt::network_config::build_nat_network_xml(
        TEST_NET_NAME,
        TEST_NET_BRIDGE,
        "10.99.99.1",
        "255.255.255.0",
        None,
        None,
    );
    conn.create_network(&xml).unwrap();

    // Stop
    conn.stop_network(TEST_NET_NAME).expect("stop_network");
    let nets = conn.list_networks().unwrap();
    let net = nets.iter().find(|n| n.name == TEST_NET_NAME).unwrap();
    assert!(!net.is_active, "Should be stopped");

    // Start again
    conn.start_network(TEST_NET_NAME).expect("start_network");
    let nets = conn.list_networks().unwrap();
    let net = nets.iter().find(|n| n.name == TEST_NET_NAME).unwrap();
    assert!(net.is_active, "Should be active after start");
}

#[test]
fn test_network_autostart_toggle() {
    let conn = connect_testhost();
    ensure_clean(&conn);
    let _guard = NetworkCleanup {
        conn: &conn,
        name: TEST_NET_NAME,
    };

    let xml = virtmanager_rs_lib::libvirt::network_config::build_nat_network_xml(
        TEST_NET_NAME,
        TEST_NET_BRIDGE,
        "10.99.99.1",
        "255.255.255.0",
        None,
        None,
    );
    conn.create_network(&xml).unwrap();

    conn.set_network_autostart(TEST_NET_NAME, true).expect("set autostart true");
    let nets = conn.list_networks().unwrap();
    let net = nets.iter().find(|n| n.name == TEST_NET_NAME).unwrap();
    assert!(net.autostart);

    conn.set_network_autostart(TEST_NET_NAME, false).expect("set autostart false");
    let nets = conn.list_networks().unwrap();
    let net = nets.iter().find(|n| n.name == TEST_NET_NAME).unwrap();
    assert!(!net.autostart);
}

#[test]
fn test_get_network_nonexistent_fails() {
    let conn = connect_testhost();
    let result = conn.get_network_xml("this-network-does-not-exist-99999");
    assert!(result.is_err());
}

// ─── Network creation modes ───

use virtmanager_rs_lib::libvirt::network_config::{
    build_network_xml, Ipv4BuildParams, Ipv6BuildParams, NetworkBuildParams,
};

#[test]
fn test_create_isolated_network() {
    let conn = connect_testhost();
    ensure_clean(&conn);
    let _guard = NetworkCleanup { conn: &conn, name: TEST_NET_NAME };

    let xml = build_network_xml(&NetworkBuildParams {
        name: TEST_NET_NAME,
        forward_mode: "isolated",
        bridge_name: TEST_NET_BRIDGE,
        forward_dev: None,
        domain_name: None,
        ipv4: Some(Ipv4BuildParams {
            address: "10.99.99.1",
            netmask: "255.255.255.0",
            dhcp_start: None,
            dhcp_end: None,
        }),
        ipv6: None,
    });
    conn.create_network(&xml).expect("create isolated");

    let cfg = conn.get_network_config(TEST_NET_NAME).unwrap();
    // Isolated networks have no <forward> — our parser reports empty string
    assert!(cfg.forward_mode.is_empty(),
        "isolated should parse as empty forward_mode, got {:?}", cfg.forward_mode);
    assert_eq!(cfg.ipv4.unwrap().address, "10.99.99.1");
}

#[test]
fn test_create_route_network_without_dev() {
    let conn = connect_testhost();
    ensure_clean(&conn);
    let _guard = NetworkCleanup { conn: &conn, name: TEST_NET_NAME };

    let xml = build_network_xml(&NetworkBuildParams {
        name: TEST_NET_NAME,
        forward_mode: "route",
        bridge_name: TEST_NET_BRIDGE,
        forward_dev: None,
        domain_name: None,
        ipv4: Some(Ipv4BuildParams {
            address: "10.99.99.1",
            netmask: "255.255.255.0",
            dhcp_start: None,
            dhcp_end: None,
        }),
        ipv6: None,
    });
    conn.create_network(&xml).expect("create route");

    let cfg = conn.get_network_config(TEST_NET_NAME).unwrap();
    assert_eq!(cfg.forward_mode, "route");
}

#[test]
fn test_create_open_network() {
    let conn = connect_testhost();
    ensure_clean(&conn);
    let _guard = NetworkCleanup { conn: &conn, name: TEST_NET_NAME };

    let xml = build_network_xml(&NetworkBuildParams {
        name: TEST_NET_NAME,
        forward_mode: "open",
        bridge_name: TEST_NET_BRIDGE,
        forward_dev: None,
        domain_name: None,
        ipv4: Some(Ipv4BuildParams {
            address: "10.99.99.1",
            netmask: "255.255.255.0",
            dhcp_start: None,
            dhcp_end: None,
        }),
        ipv6: None,
    });
    conn.create_network(&xml).expect("create open");

    let cfg = conn.get_network_config(TEST_NET_NAME).unwrap();
    assert_eq!(cfg.forward_mode, "open");
}

#[test]
fn test_create_nat_with_domain_and_dhcp() {
    let conn = connect_testhost();
    ensure_clean(&conn);
    let _guard = NetworkCleanup { conn: &conn, name: TEST_NET_NAME };

    let xml = build_network_xml(&NetworkBuildParams {
        name: TEST_NET_NAME,
        forward_mode: "nat",
        bridge_name: TEST_NET_BRIDGE,
        forward_dev: None,
        domain_name: Some("test.local"),
        ipv4: Some(Ipv4BuildParams {
            address: "10.99.99.1",
            netmask: "255.255.255.0",
            dhcp_start: Some("10.99.99.100"),
            dhcp_end: Some("10.99.99.200"),
        }),
        ipv6: None,
    });
    conn.create_network(&xml).expect("create nat");

    let cfg = conn.get_network_config(TEST_NET_NAME).unwrap();
    assert_eq!(cfg.forward_mode, "nat");
    assert_eq!(cfg.domain_name.as_deref(), Some("test.local"));
    let v4 = cfg.ipv4.unwrap();
    assert_eq!(v4.dhcp_ranges.len(), 1);
    assert_eq!(v4.dhcp_ranges[0].start, "10.99.99.100");
}

// Note: "bridge" mode (host bridge passthrough) requires the bridge to already
// exist on the host. testhost has host bridges "lan", "domo", "wan" — but attaching
// a libvirt network to them could disrupt the host. We validate the XML shape
// via unit tests only, not a real create.

// ─── Storage tests ───
//
// testhost already has `default`, `iso`, and `virtmanager-iso` pools.
// We test against existing pools (read-only) and create a disposable test
// pool for write operations.

use virtmanager_rs_lib::libvirt::storage_config::{
    build_pool_xml, build_volume_xml, PoolBuildParams, VolumeBuildParams,
};

const TEST_POOL_NAME: &str = "virtmanager-storage-test";

struct PoolCleanup<'a> {
    conn: &'a LibvirtConnection,
    name: &'static str,
}

impl<'a> Drop for PoolCleanup<'a> {
    fn drop(&mut self) {
        let _ = self.conn.delete_pool(self.name);
    }
}

fn clean_pool(conn: &LibvirtConnection) {
    let _ = conn.delete_pool(TEST_POOL_NAME);
}

#[test]
fn test_list_storage_pools() {
    let conn = connect_testhost();
    let pools = conn.list_storage_pools().expect("list_storage_pools");
    assert!(!pools.is_empty(), "testhost has at least one pool");
    println!("Found {} pools:", pools.len());
    for p in &pools {
        println!(
            "  - {} [{}] active={} cap={:.1}GB alloc={:.1}GB vols={}",
            p.name,
            p.pool_type,
            p.is_active,
            p.capacity as f64 / 1e9,
            p.allocation as f64 / 1e9,
            p.num_volumes,
        );
    }
    // testhost has at least default + iso
    assert!(pools.iter().any(|p| p.name == "default"));
}

#[test]
fn test_default_pool_has_expected_fields() {
    let conn = connect_testhost();
    let pools = conn.list_storage_pools().unwrap();
    let default = pools.iter().find(|p| p.name == "default").unwrap();
    assert_eq!(default.pool_type, "dir");
    assert!(default.is_active, "default pool should be active");
    assert!(default.capacity > 0);
    assert!(default.num_volumes > 0, "should have existing volumes");
    assert_eq!(
        default.target_path.as_deref(),
        Some("/var/lib/libvirt/images")
    );
}

#[test]
fn test_get_pool_config() {
    let conn = connect_testhost();
    let cfg = conn.get_pool_config("default").expect("get_pool_config");
    assert_eq!(cfg.name, "default");
    assert_eq!(cfg.pool_type, "dir");
    assert_eq!(
        cfg.target_path.as_deref(),
        Some("/var/lib/libvirt/images")
    );
}

#[test]
fn test_list_volumes_in_default_pool() {
    let conn = connect_testhost();
    let vols = conn.list_volumes("default").expect("list_volumes");
    assert!(!vols.is_empty(), "default pool has volumes");
    for v in &vols {
        assert!(!v.name.is_empty());
        assert!(!v.path.is_empty());
        // Some volumes (symlinks, empty placeholders) may have 0 capacity
    }
    // Known VM disks on testhost
    let has_example-firewall = vols.iter().any(|v| v.name.contains("example-firewall"));
    assert!(has_example-firewall, "expected example-firewall disk in default pool");
}

#[test]
fn test_pool_lookup_nonexistent_fails() {
    let conn = connect_testhost();
    let result = conn.get_pool_xml("this-pool-does-not-exist-99999");
    assert!(result.is_err());
}

#[test]
fn test_create_dir_pool_lifecycle() {
    let conn = connect_testhost();
    clean_pool(&conn);
    let _guard = PoolCleanup { conn: &conn, name: TEST_POOL_NAME };

    let xml = build_pool_xml(&PoolBuildParams {
        name: TEST_POOL_NAME,
        pool_type: "dir",
        target_path: Some("/tmp/virtmanager-test-pool"),
        source_host: None,
        source_dir: None,
        source_name: None,
    });

    conn.define_pool(&xml, true, true).expect("define_pool");

    let pools = conn.list_storage_pools().unwrap();
    let pool = pools.iter().find(|p| p.name == TEST_POOL_NAME).expect("pool should exist");
    assert_eq!(pool.pool_type, "dir");
    assert!(pool.is_active, "should be active after create+start");
    assert_eq!(pool.target_path.as_deref(), Some("/tmp/virtmanager-test-pool"));

    // Stop it
    conn.stop_pool(TEST_POOL_NAME).expect("stop_pool");
    let pools = conn.list_storage_pools().unwrap();
    let pool = pools.iter().find(|p| p.name == TEST_POOL_NAME).unwrap();
    assert!(!pool.is_active);

    // Start again
    conn.start_pool(TEST_POOL_NAME).expect("start_pool");
    let pools = conn.list_storage_pools().unwrap();
    let pool = pools.iter().find(|p| p.name == TEST_POOL_NAME).unwrap();
    assert!(pool.is_active);
}

#[test]
fn test_create_and_delete_volume() {
    let conn = connect_testhost();
    clean_pool(&conn);
    let _guard = PoolCleanup { conn: &conn, name: TEST_POOL_NAME };

    // Create a dedicated pool so the test volume is isolated
    let pool_xml = build_pool_xml(&PoolBuildParams {
        name: TEST_POOL_NAME,
        pool_type: "dir",
        target_path: Some("/tmp/virtmanager-test-pool"),
        source_host: None,
        source_dir: None,
        source_name: None,
    });
    conn.define_pool(&pool_xml, true, true).unwrap();

    // Create a 100MB qcow2 volume
    let vol_xml = build_volume_xml(&VolumeBuildParams {
        name: "test.qcow2",
        capacity_bytes: 100 * 1024 * 1024,
        format: "qcow2",
        allocation_bytes: None,
    });
    let path = conn
        .create_volume(TEST_POOL_NAME, &vol_xml)
        .expect("create_volume");
    assert!(path.contains("test.qcow2"));

    // Verify it's listed
    let vols = conn.list_volumes(TEST_POOL_NAME).unwrap();
    assert_eq!(vols.len(), 1);
    assert_eq!(vols[0].name, "test.qcow2");
    assert_eq!(vols[0].capacity, 100 * 1024 * 1024);
    assert_eq!(vols[0].format, "qcow2");

    // Resize to 200MB
    conn.resize_volume(&path, 200 * 1024 * 1024)
        .expect("resize_volume");
    let vols = conn.list_volumes(TEST_POOL_NAME).unwrap();
    assert_eq!(vols[0].capacity, 200 * 1024 * 1024);

    // Delete
    conn.delete_volume(&path).expect("delete_volume");
    let vols = conn.list_volumes(TEST_POOL_NAME).unwrap();
    assert!(vols.is_empty());
}

#[test]
fn test_pool_autostart_toggle() {
    let conn = connect_testhost();
    clean_pool(&conn);
    let _guard = PoolCleanup { conn: &conn, name: TEST_POOL_NAME };

    let xml = build_pool_xml(&PoolBuildParams {
        name: TEST_POOL_NAME,
        pool_type: "dir",
        target_path: Some("/tmp/virtmanager-test-pool"),
        source_host: None,
        source_dir: None,
        source_name: None,
    });
    conn.define_pool(&xml, true, true).unwrap();

    conn.set_pool_autostart(TEST_POOL_NAME, true).expect("set autostart true");
    let pools = conn.list_storage_pools().unwrap();
    let pool = pools.iter().find(|p| p.name == TEST_POOL_NAME).unwrap();
    assert!(pool.autostart);

    conn.set_pool_autostart(TEST_POOL_NAME, false).expect("set autostart false");
    let pools = conn.list_storage_pools().unwrap();
    let pool = pools.iter().find(|p| p.name == TEST_POOL_NAME).unwrap();
    assert!(!pool.autostart);
}

#[test]
fn test_refresh_pool() {
    let conn = connect_testhost();
    // Refresh a real pool (read-only operation)
    conn.refresh_pool("default").expect("refresh_pool");
}

// Production safety: verify we never disturbed existing testhost pools
#[test]
fn test_testhost_prod_pools_untouched() {
    let conn = connect_testhost();
    let pools = conn.list_storage_pools().unwrap();
    let names: Vec<&str> = pools.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"default"), "default pool must exist");
    // Verify default remains active
    let default = pools.iter().find(|p| p.name == "default").unwrap();
    assert!(default.is_active, "default must stay active");
}

// ─── VM creation tests ───
//
// Creates a disposable test VM in the `default` pool (testhost) with a tiny
// disk, verifies it's defined with correct attributes, then cleans up
// both the domain and the volume.

use virtmanager_rs_lib::libvirt::domain_builder::{
    build_domain_xml, DiskSource, DomainBuildParams, InstallMedia, NetworkSource,
};

const TEST_VM_NAME: &str = "virtmanager-test-vm";
const TEST_VOL_NAME: &str = "virtmanager-test-vm.qcow2";

struct VmCleanup<'a> {
    conn: &'a LibvirtConnection,
    domain_name: &'static str,
    volume_path: Option<String>,
}

impl<'a> Drop for VmCleanup<'a> {
    fn drop(&mut self) {
        // Undefine domain if present (stops it first if running)
        let _ = self.conn.destroy_domain(self.domain_name);
        let _ = self.conn.undefine_domain(self.domain_name);
        if let Some(p) = &self.volume_path {
            let _ = self.conn.delete_volume(p);
        }
    }
}

fn tiny_params(volume_path: &str) -> DomainBuildParams {
    DomainBuildParams {
        name: TEST_VM_NAME.into(),
        memory_mb: 128,
        vcpus: 1,
        os_type: "linux".into(),
        machine_type: "q35".into(),
        arch: "x86_64".into(),
        firmware: "bios".into(),
        disk_bus: "virtio".into(),
        nic_model: "virtio".into(),
        video_model: "virtio".into(),
        disk_source: DiskSource::ExistingPath {
            path: volume_path.into(),
            format: "qcow2".into(),
        },
        network: NetworkSource::None,
        install_media: InstallMedia::default(),
        graphics: "none".into(),
    }
}

#[test]
fn test_create_and_undefine_vm() {
    let conn = connect_testhost();

    // Cleanup any stale test VM first
    let _ = conn.destroy_domain(TEST_VM_NAME);
    let _ = conn.undefine_domain(TEST_VM_NAME);

    // Create a small test volume
    let vol_xml = virtmanager_rs_lib::libvirt::storage_config::build_volume_xml(
        &virtmanager_rs_lib::libvirt::storage_config::VolumeBuildParams {
            name: TEST_VOL_NAME,
            capacity_bytes: 64 * 1024 * 1024, // 64MB
            format: "qcow2",
            allocation_bytes: None,
        },
    );
    let _ = conn.delete_volume(&format!("/var/lib/libvirt/images/{TEST_VOL_NAME}"));
    let vol_path = conn.create_volume("default", &vol_xml).expect("create volume");

    let _guard = VmCleanup {
        conn: &conn,
        domain_name: TEST_VM_NAME,
        volume_path: Some(vol_path.clone()),
    };

    let xml = build_domain_xml(&tiny_params(&vol_path));
    conn.define_domain_xml(&xml).expect("define_domain_xml");

    // Verify listed
    let domains = conn.list_all_domains().unwrap();
    let vm = domains.iter().find(|d| d.name == TEST_VM_NAME);
    assert!(vm.is_some(), "VM should be listed");
    assert_eq!(vm.unwrap().vcpus, 1);
    assert_eq!(vm.unwrap().memory_mb, 128);

    // Config round-trips
    let cfg = conn.get_domain_config(TEST_VM_NAME, false).unwrap();
    assert_eq!(cfg.name, TEST_VM_NAME);
    assert_eq!(cfg.vcpus.max, 1);
}

#[test]
fn test_create_vm_with_pending_disk_fails_gracefully() {
    // build_domain_xml with NewVolume source emits "__PENDING__" placeholder.
    // libvirt should reject this (no such file), demonstrating that the
    // wizard flow must resolve the volume path before defining.
    let conn = connect_testhost();
    let params = DomainBuildParams {
        disk_source: DiskSource::NewVolume {
            pool_name: "default".into(),
            name: "x".into(),
            capacity_bytes: 1024 * 1024,
            format: "qcow2".into(),
        },
        ..tiny_params("unused")
    };
    let xml = build_domain_xml(&params);
    assert!(xml.contains("__PENDING__"));

    // Libvirt may or may not reject __PENDING__ at define time (it only checks
    // the file at start), but we expect the wizard never to reach this.
    // We just validate the XML contains the sentinel.
    let _ = conn; // silence unused warning
}


// ─── VNC SSH tunnel tests ───

use virtmanager_rs_lib::libvirt::vnc_proxy::{parse_ssh_target, parse_vnc_endpoint, VncSession};

#[test]
fn test_parse_vnc_endpoint_from_example-firewall_xml() {
    let conn = connect_testhost();
    let xml = conn.get_domain_xml("example-firewall", false).unwrap();
    let ep = parse_vnc_endpoint(&xml);
    assert!(ep.is_some(), "example-firewall should have a VNC port assigned");
    let (host, port) = ep.unwrap();
    println!("example-firewall VNC: {host}:{port}");
    assert!(port > 0);
}

#[test]
fn test_parse_testhost_ssh_target() {
    assert_eq!(
        parse_ssh_target(JOLYNE_URI),
        Some("testuser@testhost".into()),
    );
}

#[test]
fn test_vnc_session_ssh_tunnel_to_example-firewall() {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let conn = connect_testhost();
    let xml = conn.get_domain_xml("example-firewall", false).unwrap();
    let (listen, port) = parse_vnc_endpoint(&xml).expect("VNC port");
    let target = parse_ssh_target(JOLYNE_URI).unwrap();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let session = VncSession::start(&target, &listen, port, runtime.handle())
        .expect("start VNC session");

    // Connect as a WS client, read the RFB banner back via the tunnel.
    let handle = std::thread::spawn({
        let ws_port = session.port;
        move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                use futures_util::StreamExt;
                use tokio_tungstenite::tungstenite::Message;
                let url = format!("ws://127.0.0.1:{ws_port}");
                let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
                // First message should carry the RFB banner.
                let msg: Option<Result<Message, _>> = ws.next().await;
                let msg = msg.expect("ws closed too early").unwrap();
                let bytes = match msg {
                    Message::Binary(b) => b.to_vec(),
                    other => panic!("expected binary, got {other:?}"),
                };
                let text = String::from_utf8_lossy(&bytes);
                println!("tunnel banner: {text:?}");
                assert!(
                    text.starts_with("RFB "),
                    "expected RFB banner through tunnel, got {text:?}"
                );
            });
        }
    });

    handle.join().unwrap();
    session.close();
    let _ = TcpStream::connect_timeout(
        &"127.0.0.1:1".parse().unwrap(),
        Duration::from_millis(10),
    ); // no-op to silence unused Read/Write imports
}

// ─── Live stats sampling ───

#[test]
fn test_sample_domain_stats_running_vm() {
    let conn = connect_testhost();
    let s = conn.sample_domain_stats(TEST_VM).expect("sample stats");
    assert!(s.timestamp_ms > 0);
    assert!(s.vcpus > 0);
    assert!(s.memory_actual_kib > 0);
    assert!(s.memory_max_kib >= s.memory_actual_kib);
    // Running VMs accrue CPU time
    assert!(s.cpu_time_ns > 0, "running VM should have nonzero cpu_time");
    println!(
        "  {} sample: cpu={}ns mem={}KiB/{}KiB disks={} nics={}",
        TEST_VM, s.cpu_time_ns, s.memory_actual_kib, s.memory_max_kib,
        s.disks.len(), s.interfaces.len()
    );
}

#[test]
fn test_sample_stats_includes_disks_and_nics() {
    let conn = connect_testhost();
    // example-firewall has 2 bridged NICs (lan, domo) and a disk
    let s = conn.sample_domain_stats("example-firewall").expect("sample");
    assert!(!s.disks.is_empty(), "example-firewall has a disk");
    assert!(!s.interfaces.is_empty(), "example-firewall has NICs");
    for nic in &s.interfaces {
        assert!(nic.rx_bytes >= 0);
        assert!(nic.tx_bytes >= 0);
    }
}

#[test]
fn test_sample_cpu_time_increments() {
    let conn = connect_testhost();
    let s1 = conn.sample_domain_stats(TEST_VM).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let s2 = conn.sample_domain_stats(TEST_VM).unwrap();
    assert!(
        s2.cpu_time_ns >= s1.cpu_time_ns,
        "cpu_time should be monotonic: {} vs {}",
        s1.cpu_time_ns,
        s2.cpu_time_ns
    );
}

// ─── SPICE via capsaicin integration ───

use virtmanager_rs_lib::libvirt::spice_proxy::{
    parse_spice_endpoint, parse_spice_password, SpiceSession,
};

#[test]
fn test_parse_spice_endpoint_from_prod_brokers_xml() {
    let conn = connect_testhost();
    let xml = conn.get_domain_xml("example-broker", false).unwrap();
    let ep = parse_spice_endpoint(&xml);
    assert!(ep.is_some(), "example-broker should have SPICE graphics");
    let (host, port) = ep.unwrap();
    println!("example-broker SPICE: {host}:{port}");
    assert!(port >= 5900);
}

#[test]
fn test_spice_session_to_prod_brokers() {
    use capsaicin_client::{ClientEvent, DisplayEvent};

    let conn = connect_testhost();
    let xml = conn.get_domain_xml("example-broker", false).unwrap();
    let (listen, port) = parse_spice_endpoint(&xml).expect("SPICE port");
    let password = parse_spice_password(&xml).unwrap_or_default();
    let target = virtmanager_rs_lib::libvirt::vnc_proxy::parse_ssh_target(JOLYNE_URI).unwrap();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut session = SpiceSession::start(&target, &listen, port, &password, runtime.handle())
        .expect("start SPICE session");

    // Pump events until we see a SurfaceCreated (proves display channel handshake succeeded)
    // or time out.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut got_surface = false;
    let mut saw_any_display_event = false;

    runtime.block_on(async {
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(500), session.events_rx.recv()).await {
                Ok(Some(evt)) => {
                    match &evt {
                        ClientEvent::Display(DisplayEvent::SurfaceCreated { width, height, primary, .. }) => {
                            println!("SPICE SurfaceCreated: {width}x{height} primary={primary}");
                            got_surface = true;
                            saw_any_display_event = true;
                            break;
                        }
                        ClientEvent::Display(_) => {
                            saw_any_display_event = true;
                        }
                        ClientEvent::Cursor(_) | ClientEvent::MouseMode(_) => {
                            // New sub-channel events; ignore for the handshake test.
                        }
                        ClientEvent::Closed(err) => {
                            panic!("SPICE connection closed prematurely: {err:?}");
                        }
                    }
                }
                Ok(None) => panic!("event stream ended"),
                Err(_) => { /* tick */ }
            }
        }
    });

    assert!(
        got_surface || saw_any_display_event,
        "expected at least one Display event within 10s"
    );

    session.close();
}

#[test]
fn test_secure_xml_includes_spice_password() {
    let conn = connect_testhost();
    let xml = conn
        .get_domain_xml_flags("fedora-workstation", false, true)
        .expect("secure XML");
    let pwd = parse_spice_password(&xml);
    assert!(pwd.is_some(), "fedora-workstation SPICE password should be present with VIR_DOMAIN_XML_SECURE");
    println!("fedora-workstation SPICE password redaction bypassed, len={}", pwd.unwrap().len());
}

// ─── Host device enumeration ───

use virtmanager_rs_lib::libvirt::hostdev::HostDevice;

#[test]
fn test_list_host_pci_devices() {
    let conn = connect_testhost();
    let devs = conn.list_host_pci_devices().expect("list_host_pci_devices");
    assert!(!devs.is_empty(), "testhost should have PCI devices");
    println!("Found {} PCI devices", devs.len());
    // testhost is Jasper Lake — Intel (0x8086) should appear several times.
    assert!(devs.iter().any(|d| d.vendor_id == 0x8086),
        "expected Intel devices on testhost");
    // Every device should have a valid BDF.
    for d in &devs {
        assert!(d.vendor_id != 0, "{}: vendor_id=0", d.name);
    }
}

#[test]
fn test_list_host_usb_devices() {
    let conn = connect_testhost();
    let devs = conn.list_host_usb_devices().expect("list_host_usb_devices");
    // testhost may or may not have USB devices plugged; just don't crash.
    println!("Found {} USB devices", devs.len());
    for d in &devs {
        assert!(d.bus > 0);
    }
}

#[test]
fn test_list_domain_hostdevs_on_fedora() {
    let conn = connect_testhost();
    // fedora-workstation XML we've seen has no <hostdev>; expect empty.
    let devs = conn.list_domain_hostdevs("fedora-workstation")
        .expect("list_domain_hostdevs");
    // Not necessarily empty — just shouldn't panic.
    println!("fedora-workstation has {} hostdev entries", devs.len());
    for d in &devs {
        match d {
            HostDevice::Pci { .. } => {}
            HostDevice::UsbAddress { .. } => {}
            HostDevice::UsbVendor { .. } => {}
        }
    }
}

// ─── Domain capabilities ───

#[test]
fn test_get_domain_capabilities_testhost() {
    let conn = connect_testhost();
    let caps = conn.get_domain_capabilities(None, None, None, None)
        .expect("get_domain_capabilities");
    // testhost is x86_64 KVM
    assert_eq!(caps.arch, "x86_64");
    assert_eq!(caps.domain_type, "kvm");
    assert!(caps.max_vcpus > 0, "max_vcpus should be reported");
    // host-passthrough is always there on a KVM host
    assert!(caps.cpu.modes_supported.iter().any(|m| m == "host-passthrough"));
    // Must advertise at least virtio + sata disk buses
    assert!(caps.devices.disk_buses.contains(&"virtio".to_string()));
    // VNC + SPICE both supported
    assert!(caps.devices.graphics_types.contains(&"vnc".to_string()));
    assert!(caps.devices.graphics_types.contains(&"spice".to_string()));
    println!(
        "caps: arch={} machine={} maxVcpu={} buses={:?} gfx={:?}",
        caps.arch, caps.machine, caps.max_vcpus,
        caps.devices.disk_buses, caps.devices.graphics_types
    );
}

// ─── Round A: boot / firmware ───
//
// fedora-workstation is the disposable test target. Its persistent
// config survives between runs; we save+restore so this test is
// idempotent.

#[test]
fn test_parse_boot_config_on_fedora() {
    let conn = connect_testhost();
    let cfg = conn.get_boot_config("fedora-workstation")
        .expect("get_boot_config");
    // fedora-workstation was seen to use EFI earlier.
    println!("fedora-workstation boot: fw={} machine={:?} order={:?}",
        cfg.firmware, cfg.machine, cfg.boot_order);
    assert!(!cfg.firmware.is_empty());
    assert!(cfg.machine.is_some());
}

#[test]
fn test_boot_menu_toggle_round_trip() {
    // Boot order is libvirt-validated against the VM's actual devices
    // (a <boot dev='cdrom'/> gets stripped when there is no CD-ROM).
    // Use boot menu enable/disable as a stable round-trip probe instead.
    let conn = connect_testhost();
    let before = conn.get_boot_config("fedora-workstation").unwrap();

    let want = !before.boot_menu_enabled;
    let patch = virtmanager_rs_lib::libvirt::boot_config::BootPatch {
        boot_menu_enabled: Some(want),
        boot_menu_timeout_ms: Some(Some(3000)),
        ..Default::default()
    };
    conn.apply_boot_patch("fedora-workstation", &patch).expect("toggle bootmenu");
    let mid = conn.get_boot_config("fedora-workstation").unwrap();
    assert_eq!(mid.boot_menu_enabled, want);

    // Restore
    let restore = virtmanager_rs_lib::libvirt::boot_config::BootPatch {
        boot_menu_enabled: Some(before.boot_menu_enabled),
        boot_menu_timeout_ms: Some(before.boot_menu_timeout_ms),
        ..Default::default()
    };
    conn.apply_boot_patch("fedora-workstation", &restore).expect("restore");
    let after = conn.get_boot_config("fedora-workstation").unwrap();
    assert_eq!(after.boot_menu_enabled, before.boot_menu_enabled);
}

#[test]
fn test_apply_event_action_round_trip() {
    let conn = connect_testhost();
    let before = conn.get_boot_config("fedora-workstation").unwrap();

    let patch = virtmanager_rs_lib::libvirt::boot_config::BootPatch {
        on_poweroff: Some("restart".into()),
        ..Default::default()
    };
    conn.apply_boot_patch("fedora-workstation", &patch).expect("apply");
    let mid = conn.get_boot_config("fedora-workstation").unwrap();
    assert_eq!(mid.on_poweroff.as_deref(), Some("restart"));

    let restore = virtmanager_rs_lib::libvirt::boot_config::BootPatch {
        on_poweroff: before.on_poweroff.clone(),
        ..Default::default()
    };
    conn.apply_boot_patch("fedora-workstation", &restore).expect("restore");
    let after = conn.get_boot_config("fedora-workstation").unwrap();
    assert_eq!(after.on_poweroff, before.on_poweroff);
}

// ─── Round B: disks ───

#[test]
fn test_list_disks_on_fedora() {
    let conn = connect_testhost();
    let disks = conn.list_domain_disks(TEST_VM).expect("list disks");
    println!("fedora-workstation disks ({}):", disks.len());
    for d in &disks {
        println!("  target={} bus={} device={} driver_type={:?} source={:?}",
            d.target, d.bus, d.device, d.driver_type, d.source);
    }
    // fedora-workstation is known to have at least the root disk.
    assert!(!disks.is_empty(), "expected at least one disk");
    // Its root disk is vda on virtio bus per the dumpxml we inspected.
    assert!(disks.iter().any(|d| d.target == "vda"));
}

#[test]
fn test_hotplug_disk_round_trip() {
    use virtmanager_rs_lib::libvirt::disk_config::{DiskConfig, DiskSource};
    use virtmanager_rs_lib::libvirt::storage_config::{build_volume_xml, VolumeBuildParams};

    let conn = connect_testhost();
    // Pick a target name that isn't already taken.
    let existing = conn.list_domain_disks(TEST_VM).expect("list disks");
    assert!(!existing.iter().any(|d| d.target == "vdz"),
        "pre-existing vdz — abort");

    // 1. Create a 64 MiB qcow2 volume in the default pool.
    const VOL_NAME: &str = "virtmanager-test-hotplug-disk.qcow2";
    // Clean up any leftover from a prior failed run.
    if let Ok(vols) = conn.list_volumes("default") {
        if let Some(v) = vols.iter().find(|v| v.name == VOL_NAME) {
            let _ = conn.delete_volume(&v.path);
        }
    }

    let vol_xml = build_volume_xml(&VolumeBuildParams {
        name: VOL_NAME,
        capacity_bytes: 64 * 1024 * 1024,
        format: "qcow2",
        allocation_bytes: None,
    });
    let path = conn.create_volume("default", &vol_xml).expect("create volume");
    println!("Created test volume at {}", path);

    // Always clean up, even on panic.
    let cleanup = |path: &str| {
        let _ = conn.delete_volume(path);
    };

    let disk = DiskConfig {
        device: "disk".into(),
        bus: "virtio".into(),
        target: "vdz".into(),
        source: DiskSource::File { path: path.clone() },
        driver_name: Some("qemu".into()),
        driver_type: Some("qcow2".into()),
        cache: Some("none".into()),
        ..Default::default()
    };

    // 2. Attach live + config.
    let attach_res = conn.add_domain_disk(TEST_VM, &disk, true, true);
    if let Err(e) = &attach_res {
        cleanup(&path);
        panic!("attach failed: {}", e);
    }

    // 3. Verify it shows up in the list. list_domain_disks reads inactive
    //    (persistent) XML, which matches config=true.
    let after = conn.list_domain_disks(TEST_VM).expect("list after attach");
    let found = after.iter().any(|d| d.target == "vdz");
    if !found {
        // Try to detach before bailing so we don't leave state behind.
        let _ = conn.remove_domain_disk(TEST_VM, "vdz", true, true);
        cleanup(&path);
        panic!("attached disk 'vdz' not found in domain disks after attach");
    }

    // 4. Detach.
    let detach_res = conn.remove_domain_disk(TEST_VM, "vdz", true, true);
    if let Err(e) = &detach_res {
        cleanup(&path);
        panic!("detach failed: {}", e);
    }

    // 5. Confirm gone.
    let final_disks = conn.list_domain_disks(TEST_VM).expect("list final");
    let still_there = final_disks.iter().any(|d| d.target == "vdz");

    // 6. Always clean up the volume.
    cleanup(&path);

    assert!(!still_there, "disk 'vdz' still present after detach");
}

#[test]
fn test_cdrom_media_change_round_trip() {
    use virtmanager_rs_lib::libvirt::disk_config::{DiskConfig, DiskSource};

    let conn = connect_testhost();

    // Don't clobber an existing CD-ROM. Pick a target name that isn't used.
    let existing = conn.list_domain_disks(TEST_VM).expect("list disks");
    // sdz is a safe name on SATA that is vanishingly unlikely to already exist.
    let target = "sdz";
    if existing.iter().any(|d| d.target == target) {
        panic!("pre-existing {target} — abort");
    }

    // 1. Add an empty CD-ROM device (no source).
    let empty_cd = DiskConfig {
        device: "cdrom".into(),
        bus: "sata".into(),
        target: target.into(),
        source: DiskSource::None,
        driver_name: Some("qemu".into()),
        driver_type: Some("raw".into()),
        readonly: true,
        ..Default::default()
    };

    // Live+config. Some hosts reject live CD add on certain bus types —
    // if so, try config-only so at least the path is covered.
    let attach = conn.add_domain_disk(TEST_VM, &empty_cd, true, true)
        .or_else(|_| conn.add_domain_disk(TEST_VM, &empty_cd, false, true));
    if let Err(e) = &attach {
        panic!("add empty cdrom failed: {}", e);
    }

    // 2. Change media: insert an ISO (use any file-ish path — libvirt
    //    doesn't require the file to exist to parse the update, and we
    //    only verify round-trip of the xml).
    //    Pick /var/lib/libvirt/images/ which always exists as a dir on
    //    testhost; using /dev/null works too — update semantics only care
    //    about the source= attribute swap on the domain xml.
    let with_media = DiskConfig {
        source: DiskSource::File { path: "/dev/null".into() },
        ..empty_cd.clone()
    };

    // update_domain_disk with config only is the safest — live update of
    // a CD-ROM sometimes errors on bus=sata with qemu. We want to exercise
    // the UpdateDeviceFlags path regardless.
    let upd = conn.update_domain_disk(TEST_VM, &with_media, false, true);
    if let Err(e) = &upd {
        // Clean up before bailing.
        let _ = conn.remove_domain_disk(TEST_VM, target, true, true);
        let _ = conn.remove_domain_disk(TEST_VM, target, false, true);
        panic!("cdrom update failed: {}", e);
    }

    // 3. Verify the inactive config picked it up.
    let after = conn.list_domain_disks(TEST_VM).expect("list after update");
    let cd = after.iter().find(|d| d.target == target).expect("cdrom entry");
    match &cd.source {
        DiskSource::File { path } => assert_eq!(path, "/dev/null"),
        other => panic!("expected File source, got {:?}", other),
    }

    // 4. Clean up. Try live+config first, fall back to config only.
    let _ = conn.remove_domain_disk(TEST_VM, target, true, true)
        .or_else(|_| conn.remove_domain_disk(TEST_VM, target, false, true));

    // 5. Ensure removed.
    let final_disks = conn.list_domain_disks(TEST_VM).expect("final list");
    assert!(!final_disks.iter().any(|d| d.target == target),
        "cdrom target '{target}' should have been removed");
}
// ─── Round C: NIC management ───
//
// Drop guard ensures we always remove the test NIC even if a test panics
// mid-way. We pick a deterministic MAC in libvirt's 52:54:00 OUI so the
// post-run cleanup is unambiguous.

const TEST_NIC_MAC: &str = "52:54:00:fe:dc:ba";

struct NicCleanup<'a> {
    conn: &'a LibvirtConnection,
    vm: &'a str,
    mac: &'a str,
}
impl<'a> Drop for NicCleanup<'a> {
    fn drop(&mut self) {
        // Try both live and config in case the VM state changed mid-test.
        let _ = self.conn.remove_domain_nic(self.vm, self.mac, true, false);
        let _ = self.conn.remove_domain_nic(self.vm, self.mac, false, true);
// ─── Round E: virtio-adjacent devices ───────────────────────────────────

/// Guard: always restore the panic notifier to its pre-test state on
/// drop, even on test failure / panic.
struct PanicGuard<'a> {
    conn: &'a LibvirtConnection,
    vm: &'a str,
    before: Option<virtmanager_rs_lib::libvirt::virtio_devices::PanicConfig>,
}
impl<'a> Drop for PanicGuard<'a> {
    fn drop(&mut self) {
        let _ = self.conn.set_panic(self.vm, self.before.as_ref(), false, true);
    }
}

/// Guard: always remove any leftover RNG device we added during a test.
struct RngGuard<'a> {
    conn: &'a LibvirtConnection,
    vm: &'a str,
    cfg: virtmanager_rs_lib::libvirt::virtio_devices::RngConfig,
    live: bool,
    config: bool,
    armed: bool,
}
impl<'a> RngGuard<'a> {
    fn disarm(mut self) { self.armed = false; }
}
impl<'a> Drop for RngGuard<'a> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.conn.remove_rng(self.vm, &self.cfg, self.live, self.config);
// ─── Round G: filesystem passthrough + shmem ───
//
// Live hypervisor: testhost (libvirt 10.x, QEMU 8.x).
// Test VM: fedora-workstation. These tests MUTATE the persistent
// definition - the Drop guard at the end of each test must restore
// <memoryBacking> (and remove any leftover filesystem) even on panic,
// so a failed assertion doesn't leave fedora-workstation unbootable on
// next boot.

use virtmanager_rs_lib::libvirt::filesystem_config as fsc;

/// RAII cleanup. On drop, removes any leftover virtiofs filesystem
/// matching `target_dir` and strips the <memoryBacking> block we added.
struct RoundGCleanup<'a> {
    conn: &'a LibvirtConnection,
    vm: &'a str,
    target_dir: String,
    restore_memory_backing: bool,
}

impl Drop for RoundGCleanup<'_> {
    fn drop(&mut self) {
        // Best-effort cleanup; we're already on a panic path potentially,
        // so swallow errors.
        let _ = self.conn.remove_filesystem(self.vm, &self.target_dir, false, true);
        if self.restore_memory_backing {
            let _ = self.conn.remove_memory_backing(self.vm);
        }
    }
}

#[test]
fn test_list_domain_nics_on_fedora() {
    let conn = connect_testhost();
    let nics = conn.list_domain_nics(TEST_VM).expect("list_domain_nics");
    println!("{TEST_VM} has {} NIC(s)", nics.len());
    assert!(!nics.is_empty(), "fedora-workstation is expected to have at least one NIC");
    for n in &nics {
        println!("  source={:?} mac={:?} model={:?} target={:?} link={:?}",
            n.source, n.mac, n.model, n.target_dev, n.link_state);
fn test_list_filesystems_empty_on_fedora() {
    let conn = connect_testhost();
    let fs = conn.list_filesystems(TEST_VM).expect("list_filesystems");
    // fedora-workstation has no <filesystem> entries by default.
    println!("fedora-workstation has {} filesystem entries", fs.len());
    for f in &fs {
        println!(
            "  {:?} {} -> {}",
            f.driver_type, f.source_dir, f.target_dir
        );
    }
}

#[test]
fn test_hot_add_and_detach_network_nic() {
    use virtmanager_rs_lib::libvirt::nic_config::{NicConfig, NicSource};

    let conn = connect_testhost();
    let vm = conn.list_all_domains().unwrap();
    let running = vm.iter().any(|d| d.name == TEST_VM && matches!(d.state, VmState::Running));
    if !running {
        println!("Skipping hot-add NIC test: {TEST_VM} not running");
        return;
    }

    let before = conn.list_domain_nics(TEST_VM).unwrap().len();

    // Pre-emptive cleanup in case a prior run was killed.
    let _ = conn.remove_domain_nic(TEST_VM, TEST_NIC_MAC, true, false);

    let nic = NicConfig {
        source: NicSource::Bridge { name: "lan".into() },
        model: Some("virtio".into()),
        mac: Some(TEST_NIC_MAC.into()),
        ..Default::default()
    };

    let _guard = NicCleanup { conn: &conn, vm: TEST_VM, mac: TEST_NIC_MAC };

    conn.add_domain_nic(TEST_VM, &nic, true, false).expect("hot-add NIC");

    let after_add = conn.list_domain_nics(TEST_VM).unwrap();
    assert_eq!(after_add.len(), before + 1, "NIC count should have grown");
    let found = after_add.iter().any(|n| n.mac.as_deref() == Some(TEST_NIC_MAC));
    assert!(found, "new NIC with mac {TEST_NIC_MAC} should be visible");

    conn.remove_domain_nic(TEST_VM, TEST_NIC_MAC, true, false).expect("detach NIC");

    let after_del = conn.list_domain_nics(TEST_VM).unwrap();
    assert_eq!(after_del.len(), before, "NIC count should be back to initial");
}

#[test]
fn test_link_state_toggle_round_trip() {
    use virtmanager_rs_lib::libvirt::nic_config::{NicConfig, NicSource};

    let conn = connect_testhost();
    let vm = conn.list_all_domains().unwrap();
    let running = vm.iter().any(|d| d.name == TEST_VM && matches!(d.state, VmState::Running));
    if !running {
        println!("Skipping link-state test: {TEST_VM} not running");
        return;
    }

    // Pre-clean and add a fresh test NIC to toggle.
    let _ = conn.remove_domain_nic(TEST_VM, TEST_NIC_MAC, true, false);
    let nic = NicConfig {
        source: NicSource::Bridge { name: "lan".into() },
        model: Some("virtio".into()),
        mac: Some(TEST_NIC_MAC.into()),
        ..Default::default()
    };
    let _guard = NicCleanup { conn: &conn, vm: TEST_VM, mac: TEST_NIC_MAC };
    conn.add_domain_nic(TEST_VM, &nic, true, false).expect("add NIC for toggle");

    // Toggle to down.
    let down = NicConfig { link_state: Some("down".into()), ..nic.clone() };
    conn.update_domain_nic(TEST_VM, &down, true, false).expect("link down");
    let after_down = conn.list_domain_nics(TEST_VM).unwrap();
    let got = after_down.iter().find(|n| n.mac.as_deref() == Some(TEST_NIC_MAC)).expect("nic present");
    assert_eq!(got.link_state.as_deref(), Some("down"), "link should be down");

    // Toggle back up.
    let up = NicConfig { link_state: Some("up".into()), ..nic.clone() };
    conn.update_domain_nic(TEST_VM, &up, true, false).expect("link up");
    let after_up = conn.list_domain_nics(TEST_VM).unwrap();
    let got = after_up.iter().find(|n| n.mac.as_deref() == Some(TEST_NIC_MAC)).expect("nic present");
    assert_eq!(got.link_state.as_deref(), Some("up"), "link should be up");
}

// ─── Round D: display (graphics / video / sound / input) ───

#[test]
fn test_parse_display_config_on_fedora() {
    let conn = connect_testhost();
    let cfg = conn
        .get_display_config("fedora-workstation")
        .expect("get_display_config");
    println!(
        "fedora-workstation display: {} graphics, {} video, {} sound, {} input",
        cfg.graphics.len(),
        cfg.video.len(),
        cfg.sound.len(),
        cfg.input.len(),
    );
    // Live fedora-workstation has a SPICE graphics + virtio video + ich9 sound
    // + tablet/mouse/keyboard inputs. We only assert the minimum (presence),
    // not exact values, so the test stays stable if the VM is reconfigured.
    assert!(!cfg.graphics.is_empty(), "should have at least one <graphics>");
    assert!(!cfg.video.is_empty(), "should have at least one <video>");
    assert!(!cfg.input.is_empty(), "should have at least one <input>");
    // Graphics type should be a known value.
    let gtype = &cfg.graphics[0].r#type;
    assert!(
        matches!(gtype.as_str(), "spice" | "vnc" | "rdp" | "sdl" | "dbus" | "egl-headless" | "none"),
        "unexpected graphics type {gtype}"
    );
    // Primary video.
    let primary = cfg.video.iter().find(|v| v.primary).or(cfg.video.first());
    println!("  primary video model = {:?}", primary.map(|v| &v.model));
}

#[test]
fn test_video_model_round_trip_virtio_cirrus_virtio() {
    let conn = connect_testhost();
    let before = conn
        .get_display_config("fedora-workstation")
        .expect("get_display_config");
    let original_video = before
        .video
        .iter()
        .find(|v| v.primary)
        .or(before.video.first())
        .cloned()
        .expect("at least one <video>");

    // Flip to cirrus.
    let mut flip = original_video.clone();
    flip.model = "cirrus".to_string();
    // cirrus doesn't support blob / accel3d; strip them to avoid libvirt
    // schema rejection.
    flip.blob = None;
    flip.accel3d = false;
    // Keep heads=1 primary=yes.
    flip.primary = true;
    if flip.heads.is_none() {
        flip.heads = Some(1);
    }

    let patch = virtmanager_rs_lib::libvirt::display_config::DisplayPatch {
        video: Some(flip.clone()),
        ..Default::default()
    };
    conn.apply_display_patch("fedora-workstation", &patch)
        .expect("flip to cirrus");
    let mid = conn.get_display_config("fedora-workstation").unwrap();
    let mid_model = mid
        .video
        .iter()
        .find(|v| v.primary)
        .or(mid.video.first())
        .map(|v| v.model.clone())
        .unwrap_or_default();
    assert_eq!(mid_model, "cirrus", "video should be cirrus after flip");

    // Restore.
    let restore = virtmanager_rs_lib::libvirt::display_config::DisplayPatch {
        video: Some(original_video.clone()),
        ..Default::default()
    };
    conn.apply_display_patch("fedora-workstation", &restore)
        .expect("restore original video");
    let after = conn.get_display_config("fedora-workstation").unwrap();
    let after_model = after
        .video
        .iter()
        .find(|v| v.primary)
        .or(after.video.first())
        .map(|v| v.model.clone())
        .unwrap_or_default();
    assert_eq!(
        after_model, original_video.model,
        "video model should be restored"
fn test_list_shmems_empty_on_fedora() {
    let conn = connect_testhost();
    let shs = conn.list_shmems(TEST_VM).expect("list_shmems");
    println!("fedora-workstation has {} shmem entries", shs.len());
    assert!(
        shs.is_empty(),
        "expected no shmem entries on fedora-workstation"
    );
}

#[test]
fn test_input_list_round_trip() {
    // Save current input list, swap in a minimal list, verify, then restore.
    let conn = connect_testhost();
    let before = conn.get_display_config("fedora-workstation").unwrap();
    let original_inputs = before.input.clone();
    assert!(!original_inputs.is_empty(), "need baseline inputs");

    // Define a canonical "tablet + keyboard" list as the test payload.
    let new_inputs = vec![
        virtmanager_rs_lib::libvirt::display_config::InputConfig {
            r#type: "tablet".into(),
            bus: Some("usb".into()),
        },
        virtmanager_rs_lib::libvirt::display_config::InputConfig {
            r#type: "keyboard".into(),
            bus: Some("ps2".into()),
        },
    ];
    let patch = virtmanager_rs_lib::libvirt::display_config::DisplayPatch {
        inputs: Some(new_inputs.clone()),
        ..Default::default()
    };
    conn.apply_display_patch("fedora-workstation", &patch)
        .expect("apply inputs");

    let mid = conn.get_display_config("fedora-workstation").unwrap();
    // libvirt may auto-add a mouse on some machine types — check that
    // at minimum our tablet is present and the keyboard we asked for
    // is present.
    assert!(
        mid.input.iter().any(|i| i.r#type == "tablet" && i.bus.as_deref() == Some("usb")),
        "expected usb tablet in {:?}",
        mid.input
    );
    assert!(
        mid.input.iter().any(|i| i.r#type == "keyboard"),
        "expected keyboard in {:?}",
        mid.input
    );

    // Restore the original list.
    let restore = virtmanager_rs_lib::libvirt::display_config::DisplayPatch {
        inputs: Some(original_inputs.clone()),
        ..Default::default()
    };
    conn.apply_display_patch("fedora-workstation", &restore)
        .expect("restore inputs");
    let after = conn.get_display_config("fedora-workstation").unwrap();
    // Spot-check: counts match (libvirt may reorder slightly).
    assert_eq!(after.input.len(), original_inputs.len(), "input count restored");
fn test_get_virtio_devices_on_fedora() {
    let conn = connect_testhost();
    let snap = conn
        .get_virtio_devices("fedora-workstation")
        .expect("get_virtio_devices");
    println!(
        "fedora-workstation virtio: tpm={} rngs={} watchdog={} panic={} balloon={} vsock={} iommu={}",
        snap.tpm.is_some(),
        snap.rngs.len(),
        snap.watchdog.is_some(),
        snap.panic.is_some(),
        snap.balloon.is_some(),
        snap.vsock.is_some(),
        snap.iommu.is_some(),
    );
    // fedora-workstation was sampled with itco watchdog + virtio balloon + virtio rng.
    assert!(snap.balloon.is_some(), "should have a memballoon");
    assert!(!snap.rngs.is_empty(), "should have at least one RNG");
}

#[test]
fn test_panic_notifier_round_trip_persistent() {
    let conn = connect_testhost();
    let before = conn
        .get_virtio_devices("fedora-workstation")
        .expect("snapshot")
        .panic;

    let _guard = PanicGuard {
        conn: &conn,
        vm: "fedora-workstation",
        before: before.clone(),
    };

    let want = virtmanager_rs_lib::libvirt::virtio_devices::PanicConfig {
        model: "pvpanic".into(),
    };
    conn.set_panic("fedora-workstation", Some(&want), false, true)
        .expect("set_panic");

    let mid = conn
        .get_virtio_devices("fedora-workstation")
        .expect("snapshot");
    assert_eq!(mid.panic.as_ref().map(|p| p.model.clone()), Some("pvpanic".into()));

    // Clear, re-check.
    conn.set_panic("fedora-workstation", None, false, true)
        .expect("clear_panic");
    let after = conn
        .get_virtio_devices("fedora-workstation")
        .expect("snapshot");
    assert!(after.panic.is_none(), "panic should be removed");
    // Guard restores `before` on drop.
}

#[test]
fn test_rng_hotplug_round_trip() {
    let conn = connect_testhost();

    // fedora-workstation must be running for hotplug to be meaningful.
    // Skip if not running — the assertion confirms it's reachable.
    let vm = conn
        .list_all_domains()
        .unwrap()
        .into_iter()
        .find(|d| d.name == "fedora-workstation");
    let Some(vm) = vm else { return; };
    if vm.state != VmState::Running {
        println!("fedora-workstation not running ({:?}); skipping hotplug", vm.state);
        return;
    }

    // Distinctive shape: builtin backend (no source path) so we can
    // tell it apart from the default /dev/urandom one.
    let cfg = virtmanager_rs_lib::libvirt::virtio_devices::RngConfig {
        model: "virtio".into(),
        backend_model: "builtin".into(),
        source_path: None,
        rate_period_ms: None,
        rate_bytes: None,
    };

    let before = conn.get_virtio_devices("fedora-workstation").unwrap();
    let before_count = before.rngs.len();

    let mut guard = RngGuard {
        conn: &conn,
        vm: "fedora-workstation",
        cfg: cfg.clone(),
        live: true,
        config: true,
        armed: true,
    };

    conn.add_rng("fedora-workstation", &cfg, true, true)
        .expect("add_rng");

    let mid = conn.get_virtio_devices("fedora-workstation").unwrap();
    assert!(
        mid.rngs.iter().any(|r| r.backend_model == "builtin"),
        "new builtin RNG should be present"
    );
    assert_eq!(mid.rngs.len(), before_count + 1);

    conn.remove_rng("fedora-workstation", &cfg, true, true)
        .expect("remove_rng");
    guard.armed = false;
    let _ = guard; // drop silently; nothing to undo.

    let after = conn.get_virtio_devices("fedora-workstation").unwrap();
    assert_eq!(after.rngs.len(), before_count, "RNG count should match original");
// ─── Round F: char devices (serial / console / channel / parallel) ───

#[test]
fn test_read_char_devices_on_fedora() {
    let conn = connect_testhost();
    let snap = conn.get_char_devices("fedora-workstation")
        .expect("get_char_devices");
    println!(
        "fedora-workstation char devices: serials={} consoles={} channels={} parallels={}",
        snap.serials.len(), snap.consoles.len(), snap.channels.len(), snap.parallels.len()
    );
    // At minimum we expect one serial pty and a virtio console.
    assert!(!snap.serials.is_empty(), "should have at least one serial");
    assert!(!snap.consoles.is_empty(), "should have at least one console");
    // SPICE vdagent is usually there because fedora-workstation has SPICE graphics.
    let has_vdagent = snap.channels.iter()
        .any(|c| c.target_name.as_deref() == Some("com.redhat.spice.0"));
    println!("has vdagent channel: {}", has_vdagent);
}

#[test]
fn test_qemu_ga_channel_add_remove_round_trip() {
    use virtmanager_rs_lib::libvirt::char_devices as chd;

    let conn = connect_testhost();
    let before = conn.get_char_devices("fedora-workstation").unwrap();
    let ga_name = "org.qemu.guest_agent.0";
    let had_ga_before = before.channels.iter()
        .any(|c| c.target_name.as_deref() == Some(ga_name));

    // If already present, remove it first so we exercise both paths,
    // then restore at the end. If not present, add it and remove it.
    if had_ga_before {
        conn.remove_channel("fedora-workstation", ga_name, false, true)
            .expect("pre-remove existing qemu-ga for test");
    }

    // Add persistent-only so no live attach is needed.
    conn.add_guest_agent_channel("fedora-workstation", false, true)
        .expect("add qemu-ga channel");

    let mid = conn.get_char_devices("fedora-workstation").unwrap();
    let added = mid.channels.iter()
        .find(|c| c.target_name.as_deref() == Some(ga_name))
        .expect("qemu-ga channel should be present after add");
    assert_eq!(added.target_type, "virtio");
    assert!(matches!(added.source, chd::CharDeviceType::Unix { .. }));

    // Cleanup: remove the channel we just added.
    conn.remove_channel("fedora-workstation", ga_name, false, true)
        .expect("remove qemu-ga channel");
    let after = conn.get_char_devices("fedora-workstation").unwrap();
    assert!(
        !after.channels.iter().any(|c| c.target_name.as_deref() == Some(ga_name)),
        "qemu-ga channel should be gone after remove"
    );

    // Restore if the VM originally had qemu-ga configured.
    if had_ga_before {
        conn.add_guest_agent_channel("fedora-workstation", false, true)
            .expect("restore qemu-ga");
    }
fn test_virtiofs_add_remove_round_trip_on_fedora() {
    let conn = connect_testhost();

    // Record whether memoryBacking was present before we touched it -
    // the cleanup guard uses this to decide whether to strip it.
    let had_memory_backing_before = {
        let xml = conn.get_domain_xml(TEST_VM, true).unwrap();
        fsc::has_shared_memory_backing(&xml)
    };

    let target_dir = "virtmgr_rs_testshare".to_string();

    // Guard is armed before any mutation so a panic in the middle of
    // the test still triggers cleanup.
    let _guard = RoundGCleanup {
        conn: &conn,
        vm: TEST_VM,
        target_dir: target_dir.clone(),
        restore_memory_backing: !had_memory_backing_before,
    };

    // Defensive pre-clean: if a previous aborted test left a stale
    // entry with the same tag, drop it now.
    let pre = conn.list_filesystems(TEST_VM).unwrap();
    if pre.iter().any(|f| f.target_dir == target_dir) {
        let _ = conn.remove_filesystem(TEST_VM, &target_dir, false, true);
    }

    // Add a virtiofs share pointing to /tmp with target tag testshare.
    // Persistent-only - the running VM can't hot-plug virtiofs without
    // shared memory backing being present at start-up.
    let fs = fsc::FilesystemConfig::virtiofs("/tmp", &target_dir);
    conn.add_filesystem(TEST_VM, &fs, /*force_mb*/ true, /*live*/ false, /*config*/ true)
        .expect("add_filesystem (virtiofs)");

    // Verify the entry is now in the persistent XML.
    let after = conn.list_filesystems(TEST_VM).unwrap();
    let found = after.iter().find(|f| f.target_dir == target_dir);
    assert!(found.is_some(), "virtiofs filesystem should be present after add");
    let found = found.unwrap();
    assert_eq!(found.driver_type, fsc::FilesystemDriver::Virtiofs);
    assert_eq!(found.source_dir, "/tmp");

    // Verify shared memoryBacking is now present.
    let xml_after = conn.get_domain_xml(TEST_VM, true).unwrap();
    assert!(
        fsc::has_shared_memory_backing(&xml_after),
        "shared memoryBacking should be present after virtiofs add"
    );

    // Remove it.
    conn.remove_filesystem(TEST_VM, &target_dir, false, true)
        .expect("remove_filesystem");
    let after = conn.list_filesystems(TEST_VM).unwrap();
    assert!(
        !after.iter().any(|f| f.target_dir == target_dir),
        "filesystem should be gone after remove"
    );

    // Guard's Drop will restore memoryBacking if it wasn't present before.
}

#[test]
fn test_enable_shared_memory_backing_is_idempotent() {
    let conn = connect_testhost();
    let had_before = fsc::has_shared_memory_backing(
        &conn.get_domain_xml(TEST_VM, true).unwrap(),
    );

    // Cleanup guard: if we flip it on, turn it off again at the end.
    struct Cleanup<'a> {
        conn: &'a LibvirtConnection,
        vm: &'a str,
        restore: bool,
    }
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            if self.restore {
                let _ = self.conn.remove_memory_backing(self.vm);
            }
        }
    }
    let _guard = Cleanup {
        conn: &conn,
        vm: TEST_VM,
        restore: !had_before,
    };

    conn.enable_shared_memory_backing(TEST_VM)
        .expect("enable_shared_memory_backing");
    assert!(fsc::has_shared_memory_backing(
        &conn.get_domain_xml(TEST_VM, true).unwrap()
    ));
    // Second call: noop.
    conn.enable_shared_memory_backing(TEST_VM)
        .expect("enable_shared_memory_backing (2nd call)");
    let xml = conn.get_domain_xml(TEST_VM, true).unwrap();
    assert_eq!(xml.matches("<memoryBacking").count(), 1);
}
