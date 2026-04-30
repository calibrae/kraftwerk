//! Integration tests for phase 4 features (networking depth).
//!
//! 4.1 DHCP reservation + DNS hostname override (virNetworkUpdate)
//! 4.2 static route (define-XML round-trip)
//! 4.3 nwfilter listing (read-only)
//!
//! All write paths use clearly-fake values (52:54:00:99:99:99 MAC,
//! TEST-NET-2 routes, kraftwerk-it-test hostname) and clean up via
//! RAII guards. Skips when KRAFTWERK_TEST_URI is unset.
//!
//! Picks the first active virtual network with an IPv4 DHCP range —
//! usually `default` on most setups. If no such network exists the
//! relevant tests skip with a printed reason.

use kraftwerk_lib::libvirt::connection::LibvirtConnection;
use kraftwerk_lib::libvirt::network_config::NetworkRoute;
use std::env;

fn test_uri() -> Option<String> {
    env::var("KRAFTWERK_TEST_URI").ok().filter(|s| !s.is_empty())
}

fn connect() -> Option<LibvirtConnection> {
    let uri = test_uri()?;
    let conn = LibvirtConnection::new();
    conn.open(&uri).expect("connection.open");
    Some(conn)
}

/// Find an active virtual network with at least one IPv4 DHCP range.
/// Returns (network_name, ipv4_subnet_octets) so tests can pick valid
/// in-range IPs without relying on `default` existing.
fn pick_test_network(conn: &LibvirtConnection) -> Option<(String, [u8; 4], u32)> {
    let nets = conn.list_networks().ok()?;
    for n in nets {
        if !n.is_active { continue; }
        let cfg = match conn.get_network_config(&n.name) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let ipv4 = cfg.ipv4?;
        if ipv4.dhcp_ranges.is_empty() { continue; }
        let octets: Vec<u8> = ipv4.address.split('.').filter_map(|s| s.parse().ok()).collect();
        if octets.len() != 4 { continue; }
        let prefix = ipv4.prefix.unwrap_or(24);
        return Some((n.name, [octets[0], octets[1], octets[2], octets[3]], prefix));
    }
    None
}

const FAKE_MAC: &str = "52:54:00:99:99:99";
const FAKE_HOSTNAME: &str = "kraftwerk-it-test";

struct DhcpCleanup<'a> {
    conn: &'a LibvirtConnection,
    network: String,
    mac: String,
    name: Option<String>,
    ip: String,
}
impl<'a> Drop for DhcpCleanup<'a> {
    fn drop(&mut self) {
        let snippet = kraftwerk_lib::libvirt::network_config::build_dhcp_host_xml(
            Some(&self.mac),
            self.name.as_deref(),
            &self.ip,
        );
        let _ = self.conn.network_update_section(&self.network, 2, 4, &snippet);
    }
}

struct DnsCleanup<'a> {
    conn: &'a LibvirtConnection,
    network: String,
    ip: String,
    hostnames: Vec<String>,
}
impl<'a> Drop for DnsCleanup<'a> {
    fn drop(&mut self) {
        let snippet = kraftwerk_lib::libvirt::network_config::build_dns_host_xml(
            &self.ip,
            &self.hostnames,
        );
        let _ = self.conn.network_update_section(&self.network, 2, 10, &snippet);
    }
}

struct RouteCleanup<'a> {
    conn: &'a LibvirtConnection,
    network: String,
    route: NetworkRoute,
}
impl<'a> Drop for RouteCleanup<'a> {
    fn drop(&mut self) {
        let _ = self.conn.remove_network_route(&self.network, &self.route);
    }
}

#[test]
fn test_dhcp_host_add_list_remove() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let Some((net, [a, b, c, _], _)) = pick_test_network(&conn) else {
        eprintln!("SKIP: no active virtual network with an IPv4 DHCP range");
        return;
    };
    // .250 is high in the /24 — unlikely to collide with active leases.
    let ip = format!("{a}.{b}.{c}.250");
    let snippet = kraftwerk_lib::libvirt::network_config::build_dhcp_host_xml(
        Some(FAKE_MAC), Some(FAKE_HOSTNAME), &ip,
    );

    // Pre-clean in case a previous run crashed.
    let _ = conn.network_update_section(&net, 2, 4, &snippet);

    let _g = DhcpCleanup {
        conn: &conn,
        network: net.clone(),
        mac: FAKE_MAC.into(),
        name: Some(FAKE_HOSTNAME.into()),
        ip: ip.clone(),
    };

    // ADD_LAST = 3, SECTION_IP_DHCP_HOST = 4
    conn.network_update_section(&net, 3, 4, &snippet)
        .expect("add dhcp host");

    let cfg = conn.get_network_config(&net).expect("get_network_config");
    let v4 = cfg.ipv4.expect("ipv4 config");
    let mine = v4
        .dhcp_hosts
        .iter()
        .find(|h| h.mac.as_deref() == Some(FAKE_MAC))
        .expect("our DHCP host should be listed");
    assert_eq!(mine.ip, ip);
    assert_eq!(mine.name.as_deref(), Some(FAKE_HOSTNAME));
}

#[test]
fn test_dns_host_add_list_remove() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let Some((net, [a, b, c, _], _)) = pick_test_network(&conn) else {
        eprintln!("SKIP: no usable network");
        return;
    };
    let ip = format!("{a}.{b}.{c}.251");
    let hostnames = vec![format!("{FAKE_HOSTNAME}-dns").to_string()];
    let snippet = kraftwerk_lib::libvirt::network_config::build_dns_host_xml(&ip, &hostnames);
    let _ = conn.network_update_section(&net, 2, 10, &snippet);

    let _g = DnsCleanup {
        conn: &conn,
        network: net.clone(),
        ip: ip.clone(),
        hostnames: hostnames.clone(),
    };

    // ADD_LAST = 3, SECTION_DNS_HOST = 10
    conn.network_update_section(&net, 3, 10, &snippet)
        .expect("add dns host");

    let cfg = conn.get_network_config(&net).expect("get_network_config");
    let mine = cfg
        .dns_hosts
        .iter()
        .find(|h| h.ip == ip)
        .expect("our DNS host should be listed");
    assert!(mine.hostnames.iter().any(|h| h == &hostnames[0]));
}

#[test]
fn test_static_route_add_list_remove() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let Some((net, [a, b, c, _], _)) = pick_test_network(&conn) else {
        eprintln!("SKIP: no usable network");
        return;
    };
    // TEST-NET-2 (RFC 5737) — guaranteed never to conflict with real
    // routes, libvirt happily accepts it.
    let route = NetworkRoute {
        family: "ipv4".into(),
        address: "198.51.100.0".into(),
        prefix: 24,
        gateway: format!("{a}.{b}.{c}.1"),
    };
    // Pre-clean (idempotent — succeeds even when no match).
    let _ = conn.remove_network_route(&net, &route);

    let _g = RouteCleanup {
        conn: &conn,
        network: net.clone(),
        route: route.clone(),
    };

    conn.add_network_route(&net, &route).expect("add route");

    let cfg = conn.get_network_config(&net).expect("get_network_config");
    let mine = cfg
        .routes
        .iter()
        .find(|r| r.address == route.address && r.prefix == route.prefix)
        .expect("our route should be listed");
    assert_eq!(mine.gateway, route.gateway);
    assert_eq!(mine.family, route.family);
}

#[test]
fn test_list_nw_filters_returns_builtins() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let filters = conn.list_nw_filters().expect("list_nw_filters");
    // libvirt always ships at least these built-in filter names.
    let names: std::collections::HashSet<_> = filters.iter().map(|f| f.name.as_str()).collect();
    let has_at_least_one_builtin = names.contains("clean-traffic")
        || names.contains("no-mac-spoofing")
        || names.contains("allow-arp")
        || names.contains("no-ip-spoofing");
    assert!(
        has_at_least_one_builtin,
        "expected at least one libvirt built-in filter, got {names:?}",
    );

    // Each entry has a non-empty name and UUID.
    for f in &filters {
        assert!(!f.name.is_empty(), "filter name should not be empty");
        assert!(!f.uuid.is_empty(), "filter uuid should not be empty");
    }
}

#[test]
fn test_get_nw_filter_xml_returns_filter_element() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let filters = conn.list_nw_filters().expect("list_nw_filters");
    let Some(target) = filters.iter().find(|f| f.name == "clean-traffic")
        .or_else(|| filters.first()) else {
        eprintln!("SKIP: no nwfilters defined");
        return;
    };
    let xml = conn.get_nw_filter_xml(&target.name).expect("get_nw_filter_xml");
    assert!(xml.contains("<filter"), "expected <filter> root element");
    assert!(xml.contains(&target.name), "filter XML should mention its own name");
}
