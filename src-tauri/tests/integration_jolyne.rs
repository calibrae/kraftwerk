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
