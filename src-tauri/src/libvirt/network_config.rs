//! Parse libvirt network XML into a structured configuration.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::models::error::VirtManagerError;
use crate::models::network::ForwardMode;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    pub name: String,
    pub uuid: String,
    pub bridge: Option<String>,
    pub forward_mode: String,
    pub forward_dev: Option<String>,
    pub domain_name: Option<String>,
    pub ipv4: Option<IpConfig>,
    pub ipv6: Option<IpConfig>,
    /// `<dns><host>` entries — local-resolver overrides for the network's
    /// dnsmasq.
    pub dns_hosts: Vec<DnsHost>,
    /// Static routes pushed into the host's routing table for this
    /// network (libvirt rewrites the host's iptables to honour them).
    pub routes: Vec<NetworkRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRoute {
    /// "ipv4" or "ipv6". Defaults to ipv4 when libvirt omits the attr.
    pub family: String,
    pub address: String,
    pub prefix: u32,
    pub gateway: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsHost {
    pub ip: String,
    pub hostnames: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IpConfig {
    pub address: String,
    pub netmask: Option<String>,
    pub prefix: Option<u32>,
    pub dhcp_ranges: Vec<DhcpRange>,
    pub dhcp_hosts: Vec<DhcpHost>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhcpRange {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhcpHost {
    pub mac: Option<String>,
    pub name: Option<String>,
    pub ip: String,
}

/// Parse a libvirt network XML string into a NetworkConfig.
pub fn parse(xml: &str) -> Result<NetworkConfig, VirtManagerError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut cfg = NetworkConfig::default();
    let mut path: Vec<String> = Vec::new();
    let mut buf = Vec::new();

    // IP parsing state: track which <ip> we're inside (v4 or v6)
    let mut current_ip_family: Option<IpFamily> = None;
    let mut ipv4 = IpConfig::default();
    let mut ipv6 = IpConfig::default();
    let mut has_ipv4 = false;
    let mut has_ipv6 = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: format!("at pos {}: {}", reader.buffer_position(), e),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let attrs = collect_attrs(&e);
                handle_start(
                    &mut cfg,
                    &path,
                    &name,
                    &attrs,
                    &mut current_ip_family,
                    &mut ipv4,
                    &mut ipv6,
                    &mut has_ipv4,
                    &mut has_ipv6,
                );
                path.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let attrs = collect_attrs(&e);
                handle_start(
                    &mut cfg,
                    &path,
                    &name,
                    &attrs,
                    &mut current_ip_family,
                    &mut ipv4,
                    &mut ipv6,
                    &mut has_ipv4,
                    &mut has_ipv6,
                );
            }
            Ok(Event::End(e)) => {
                let end_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if end_name == "ip" {
                    current_ip_family = None;
                }
                path.pop();
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                handle_text(&mut cfg, &path, &text);
            }
            _ => {}
        }
        buf.clear();
    }

    if has_ipv4 {
        cfg.ipv4 = Some(ipv4);
    }
    if has_ipv6 {
        cfg.ipv6 = Some(ipv6);
    }

    Ok(cfg)
}

#[derive(Debug, Clone, Copy)]
enum IpFamily {
    V4,
    V6,
}

fn collect_attrs(e: &quick_xml::events::BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .filter_map(|a| a.ok())
        .map(|a| {
            (
                String::from_utf8_lossy(a.key.as_ref()).to_string(),
                a.unescape_value().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn get_attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[allow(clippy::too_many_arguments)]
fn handle_start(
    cfg: &mut NetworkConfig,
    path: &[String],
    name: &str,
    attrs: &[(String, String)],
    current_ip_family: &mut Option<IpFamily>,
    ipv4: &mut IpConfig,
    ipv6: &mut IpConfig,
    has_ipv4: &mut bool,
    has_ipv6: &mut bool,
) {
    let parent_is = |p: &str| path.last().map(String::as_str) == Some(p);
    let grandparent_is = |p: &str| {
        path.len() >= 2 && path[path.len() - 2] == p
    };

    match name {
        "bridge" if parent_is("network") => {
            cfg.bridge = get_attr(attrs, "name").map(String::from);
        }
        "forward" if parent_is("network") => {
            cfg.forward_mode = get_attr(attrs, "mode").unwrap_or("nat").to_string();
            cfg.forward_dev = get_attr(attrs, "dev").map(String::from);
        }
        "domain" if parent_is("network") => {
            cfg.domain_name = get_attr(attrs, "name").map(String::from);
        }
        "ip" if parent_is("network") => {
            let family = get_attr(attrs, "family").unwrap_or("ipv4");
            let is_v6 = family == "ipv6";
            let target = if is_v6 {
                *has_ipv6 = true;
                *current_ip_family = Some(IpFamily::V6);
                ipv6
            } else {
                *has_ipv4 = true;
                *current_ip_family = Some(IpFamily::V4);
                ipv4
            };
            if let Some(addr) = get_attr(attrs, "address") {
                target.address = addr.to_string();
            }
            target.netmask = get_attr(attrs, "netmask").map(String::from);
            target.prefix = get_attr(attrs, "prefix").and_then(|s| s.parse().ok());
        }
        "range" if parent_is("dhcp") && grandparent_is("ip") => {
            let target = match current_ip_family {
                Some(IpFamily::V6) => ipv6,
                _ => ipv4,
            };
            if let (Some(start), Some(end)) = (get_attr(attrs, "start"), get_attr(attrs, "end")) {
                target.dhcp_ranges.push(DhcpRange {
                    start: start.to_string(),
                    end: end.to_string(),
                });
            }
        }
        "host" if parent_is("dhcp") && grandparent_is("ip") => {
            let target = match current_ip_family {
                Some(IpFamily::V6) => ipv6,
                _ => ipv4,
            };
            if let Some(ip) = get_attr(attrs, "ip") {
                target.dhcp_hosts.push(DhcpHost {
                    mac: get_attr(attrs, "mac").map(String::from),
                    name: get_attr(attrs, "name").map(String::from),
                    ip: ip.to_string(),
                });
            }
        }
        "route" if parent_is("network") => {
            if let (Some(addr), Some(gw)) = (get_attr(attrs, "address"), get_attr(attrs, "gateway")) {
                cfg.routes.push(NetworkRoute {
                    family: get_attr(attrs, "family").unwrap_or("ipv4").to_string(),
                    address: addr.to_string(),
                    prefix: get_attr(attrs, "prefix")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    gateway: gw.to_string(),
                });
            }
        }
        "host" if parent_is("dns") && grandparent_is("network") => {
            // Started a <dns><host ip='...'> block; collect hostnames in
            // the corresponding handle_text entry.
            if let Some(ip) = get_attr(attrs, "ip") {
                cfg.dns_hosts.push(DnsHost {
                    ip: ip.to_string(),
                    hostnames: Vec::new(),
                });
            }
        }
        _ => {}
    }
}

fn handle_text(cfg: &mut NetworkConfig, path: &[String], text: &str) {
    let last = path.last().map(String::as_str);
    let parent = if path.len() >= 2 {
        Some(path[path.len() - 2].as_str())
    } else {
        None
    };

    match (last, parent) {
        (Some("name"), Some("network")) => cfg.name = text.to_string(),
        (Some("uuid"), Some("network")) => cfg.uuid = text.to_string(),
        (Some("hostname"), Some("host")) => {
            // <dns><host ip='...'><hostname>foo</hostname></host></dns>
            // Append to the last DNS host we started in handle_start.
            if let Some(last_dns) = cfg.dns_hosts.last_mut() {
                if !text.trim().is_empty() {
                    last_dns.hostnames.push(text.to_string());
                }
            }
        }
        _ => {}
    }
}

/// Build a `<host mac='...' name='...' ip='...'/>` snippet — the XML
/// virNetworkUpdate(SECTION_IP_DHCP_HOST, ADD_LAST/DELETE) expects.
pub fn build_dhcp_host_xml(mac: Option<&str>, name: Option<&str>, ip: &str) -> String {
    use crate::libvirt::xml_helpers::escape_xml;
    let mut s = String::from("<host");
    if let Some(m) = mac {
        if !m.is_empty() {
            s.push_str(&format!(" mac='{}'", escape_xml(m)));
        }
    }
    if let Some(n) = name {
        if !n.is_empty() {
            s.push_str(&format!(" name='{}'", escape_xml(n)));
        }
    }
    s.push_str(&format!(" ip='{}'/>", escape_xml(ip)));
    s
}

/// Build a `<route ...>` element for a network static route.
pub fn build_route_xml(r: &NetworkRoute) -> String {
    use crate::libvirt::xml_helpers::escape_xml;
    format!(
        "<route family='{}' address='{}' prefix='{}' gateway='{}'/>",
        escape_xml(&r.family),
        escape_xml(&r.address),
        r.prefix,
        escape_xml(&r.gateway),
    )
}

/// Add a `<route .../>` element to a network XML, before `</network>`.
/// libvirt has no virNetworkUpdate section for routes, so we rewrite
/// and redefine.
pub fn add_route_to_network_xml(xml: &str, route: &NetworkRoute) -> String {
    let snippet = build_route_xml(route);
    if let Some(idx) = xml.rfind("</network>") {
        // Indent like the rest of the file's children — find the
        // leading whitespace of the line containing </network>.
        let line_start = xml[..idx].rfind('\n').map(|n| n + 1).unwrap_or(0);
        let indent = &xml[line_start..idx];
        format!("{}{}{}\n{}{}", &xml[..line_start], indent, snippet, indent, &xml[idx..])
    } else {
        // Defensive fallback — should never happen on a real network XML.
        format!("{xml}\n{snippet}")
    }
}

/// Remove the first matching `<route>` element. Match is on
/// (family, address, prefix, gateway) — close enough for human-managed
/// routes; if duplicates exist we drop the first.
pub fn remove_route_from_network_xml(xml: &str, route: &NetworkRoute) -> String {
    // The single-line route element makes substring matching reliable.
    let needle = build_route_xml(route);
    if let Some(idx) = xml.find(&needle) {
        // Trim any leading whitespace + trailing newline so we don't
        // leave a blank gap.
        let line_start = xml[..idx].rfind('\n').map(|n| n + 1).unwrap_or(0);
        let trail = idx + needle.len();
        let trail_end = if xml[trail..].starts_with('\n') { trail + 1 } else { trail };
        // Only swallow leading whitespace if the entire prefix on this
        // line was just whitespace (i.e. the route is on its own line).
        let line_prefix = &xml[line_start..idx];
        let cut_start = if line_prefix.chars().all(|c| c.is_whitespace()) {
            line_start
        } else {
            idx
        };
        return format!("{}{}", &xml[..cut_start], &xml[trail_end..]);
    }
    xml.to_string()
}

/// Build a `<host ip='...'><hostname>...</hostname></host>` snippet for
/// SECTION_DNS_HOST. Multiple hostnames per entry are folded into one
/// element since libvirt's update API takes one `<host>` at a time.
pub fn build_dns_host_xml(ip: &str, hostnames: &[String]) -> String {
    use crate::libvirt::xml_helpers::escape_xml;
    let mut s = format!("<host ip='{}'>", escape_xml(ip));
    for h in hostnames {
        if !h.is_empty() {
            s.push_str(&format!("<hostname>{}</hostname>", escape_xml(h)));
        }
    }
    s.push_str("</host>");
    s
}

/// Build a summary string like "192.168.100.1/24" for display.
pub fn ip_summary(ip: &IpConfig) -> String {
    let prefix = ip.prefix.unwrap_or_else(|| netmask_to_prefix(ip.netmask.as_deref()));
    format!("{}/{}", ip.address, prefix)
}

fn netmask_to_prefix(netmask: Option<&str>) -> u32 {
    match netmask {
        Some(nm) => nm
            .split('.')
            .filter_map(|s| s.parse::<u8>().ok())
            .map(|b| b.count_ones())
            .sum(),
        None => 24,
    }
}

/// Build a minimal network XML for creating a simple NAT network.
/// Parameters for creating a new network.
///
/// Different forward modes require different subsets of fields:
/// - **nat / route / open / isolated**: `bridge_name` is a new bridge libvirt will create,
///   plus typically `ipv4` and/or `ipv6`. DHCP is optional per family.
///   For `route`, `forward_dev` optionally pins the physical uplink interface.
///   For `isolated`, pass "isolated" or empty string — no `<forward>` element is emitted.
/// - **bridge**: `bridge_name` is the name of a *pre-existing* host bridge (e.g. `br0`).
///   IPv4/IPv6 config is ignored since the host manages addressing.
#[derive(Debug, Clone, Default)]
pub struct NetworkBuildParams<'a> {
    pub name: &'a str,
    /// "nat" | "route" | "open" | "bridge" | "isolated" (empty = isolated)
    pub forward_mode: &'a str,
    pub bridge_name: &'a str,
    /// For `route` / `bridge` modes, pins a specific host interface.
    pub forward_dev: Option<&'a str>,
    /// Optional internal DNS domain (e.g. "example.local").
    pub domain_name: Option<&'a str>,
    pub ipv4: Option<Ipv4BuildParams<'a>>,
    pub ipv6: Option<Ipv6BuildParams<'a>>,
}

#[derive(Debug, Clone)]
pub struct Ipv4BuildParams<'a> {
    pub address: &'a str,
    pub netmask: &'a str,
    pub dhcp_start: Option<&'a str>,
    pub dhcp_end: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct Ipv6BuildParams<'a> {
    pub address: &'a str,
    pub prefix: u32,
    pub dhcp_start: Option<&'a str>,
    pub dhcp_end: Option<&'a str>,
}

/// Build a libvirt network XML from structured parameters.
/// Handles all supported forward modes.
pub fn build_network_xml(p: &NetworkBuildParams) -> String {
    use crate::libvirt::xml_helpers::escape_xml;

    let mut xml = String::from("<network>\n");
    xml.push_str(&format!("  <name>{}</name>\n", escape_xml(p.name)));

    let mode_lower = p.forward_mode.trim().to_lowercase();
    let include_forward = !mode_lower.is_empty() && mode_lower != "isolated";
    if include_forward {
        match p.forward_dev {
            Some(dev) if !dev.is_empty() => xml.push_str(&format!(
                "  <forward mode='{}' dev='{}'/>\n",
                escape_xml(&mode_lower),
                escape_xml(dev),
            )),
            _ => xml.push_str(&format!("  <forward mode='{}'/>\n", escape_xml(&mode_lower))),
        }
    }

    if !p.bridge_name.is_empty() {
        xml.push_str(&format!("  <bridge name='{}'/>\n", escape_xml(p.bridge_name)));
    }

    if let Some(domain) = p.domain_name {
        if !domain.is_empty() {
            xml.push_str(&format!("  <domain name='{}'/>\n", escape_xml(domain)));
        }
    }

    // bridge mode: host-managed, skip all <ip> config.
    if mode_lower != "bridge" {
        if let Some(v4) = &p.ipv4 {
            xml.push_str(&format!(
                "  <ip address='{}' netmask='{}'>\n",
                escape_xml(v4.address),
                escape_xml(v4.netmask),
            ));
            if let (Some(s), Some(e)) = (v4.dhcp_start, v4.dhcp_end) {
                if !s.is_empty() && !e.is_empty() {
                    xml.push_str(&format!(
                        "    <dhcp>\n      <range start='{}' end='{}'/>\n    </dhcp>\n",
                        escape_xml(s),
                        escape_xml(e),
                    ));
                }
            }
            xml.push_str("  </ip>\n");
        }

        if let Some(v6) = &p.ipv6 {
            xml.push_str(&format!(
                "  <ip family='ipv6' address='{}' prefix='{}'>\n",
                escape_xml(v6.address),
                v6.prefix,
            ));
            if let (Some(s), Some(e)) = (v6.dhcp_start, v6.dhcp_end) {
                if !s.is_empty() && !e.is_empty() {
                    xml.push_str(&format!(
                        "    <dhcp>\n      <range start='{}' end='{}'/>\n    </dhcp>\n",
                        escape_xml(s),
                        escape_xml(e),
                    ));
                }
            }
            xml.push_str("  </ip>\n");
        }
    }

    xml.push_str("</network>\n");
    xml
}

/// Thin wrapper for backward-compat NAT-only creation.
pub fn build_nat_network_xml(
    name: &str,
    bridge: &str,
    ipv4_address: &str,
    ipv4_netmask: &str,
    dhcp_start: Option<&str>,
    dhcp_end: Option<&str>,
) -> String {
    build_network_xml(&NetworkBuildParams {
        name,
        forward_mode: "nat",
        bridge_name: bridge,
        forward_dev: None,
        domain_name: None,
        ipv4: Some(Ipv4BuildParams {
            address: ipv4_address,
            netmask: ipv4_netmask,
            dhcp_start,
            dhcp_end,
        }),
        ipv6: None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_NAT_XML: &str = r#"<network>
  <name>default</name>
  <uuid>abc-123</uuid>
  <forward mode='nat'/>
  <bridge name='virbr0' stp='on' delay='0'/>
  <domain name='example.local'/>
  <ip address='192.168.122.1' netmask='255.255.255.0'>
    <dhcp>
      <range start='192.168.122.2' end='192.168.122.254'/>
      <host mac='52:54:00:aa:bb:cc' name='vm1' ip='192.168.122.10'/>
    </dhcp>
  </ip>
</network>
"#;

    #[test]
    fn parses_name_and_uuid() {
        let cfg = parse(DEFAULT_NAT_XML).unwrap();
        assert_eq!(cfg.name, "default");
        assert_eq!(cfg.uuid, "abc-123");
    }

    #[test]
    fn parses_bridge() {
        let cfg = parse(DEFAULT_NAT_XML).unwrap();
        assert_eq!(cfg.bridge, Some("virbr0".into()));
    }

    #[test]
    fn parses_forward_mode() {
        let cfg = parse(DEFAULT_NAT_XML).unwrap();
        assert_eq!(cfg.forward_mode, "nat");
    }

    #[test]
    fn parses_domain_name() {
        let cfg = parse(DEFAULT_NAT_XML).unwrap();
        assert_eq!(cfg.domain_name, Some("example.local".into()));
    }

    #[test]
    fn parses_ipv4_address() {
        let cfg = parse(DEFAULT_NAT_XML).unwrap();
        let v4 = cfg.ipv4.unwrap();
        assert_eq!(v4.address, "192.168.122.1");
        assert_eq!(v4.netmask, Some("255.255.255.0".into()));
    }

    #[test]
    fn parses_dhcp_range() {
        let cfg = parse(DEFAULT_NAT_XML).unwrap();
        let v4 = cfg.ipv4.unwrap();
        assert_eq!(v4.dhcp_ranges.len(), 1);
        assert_eq!(v4.dhcp_ranges[0].start, "192.168.122.2");
        assert_eq!(v4.dhcp_ranges[0].end, "192.168.122.254");
    }

    #[test]
    fn parses_dhcp_static_host() {
        let cfg = parse(DEFAULT_NAT_XML).unwrap();
        let v4 = cfg.ipv4.unwrap();
        assert_eq!(v4.dhcp_hosts.len(), 1);
        assert_eq!(v4.dhcp_hosts[0].mac.as_deref(), Some("52:54:00:aa:bb:cc"));
        assert_eq!(v4.dhcp_hosts[0].ip, "192.168.122.10");
    }

    #[test]
    fn parses_isolated_network() {
        let xml = r#"<network>
  <name>iso</name>
  <uuid>x</uuid>
  <bridge name='virbr1'/>
  <ip address='10.0.0.1' netmask='255.255.255.0'/>
</network>
"#;
        let cfg = parse(xml).unwrap();
        // No <forward> element means isolated; default forward_mode stays empty
        assert_eq!(cfg.forward_mode, "");
        let v4 = cfg.ipv4.unwrap();
        assert_eq!(v4.address, "10.0.0.1");
    }

    #[test]
    fn parses_ipv6_network() {
        let xml = r#"<network>
  <name>v6net</name>
  <uuid>x</uuid>
  <forward mode='nat'/>
  <bridge name='virbr2'/>
  <ip family='ipv6' address='2001:db8::1' prefix='64'>
    <dhcp>
      <range start='2001:db8::100' end='2001:db8::200'/>
    </dhcp>
  </ip>
</network>
"#;
        let cfg = parse(xml).unwrap();
        assert!(cfg.ipv6.is_some());
        let v6 = cfg.ipv6.unwrap();
        assert_eq!(v6.address, "2001:db8::1");
        assert_eq!(v6.prefix, Some(64));
        assert_eq!(v6.dhcp_ranges.len(), 1);
    }

    #[test]
    fn parses_both_ipv4_and_ipv6() {
        let xml = r#"<network>
  <name>dual</name>
  <uuid>x</uuid>
  <forward mode='nat'/>
  <bridge name='virbr3'/>
  <ip address='10.0.0.1' netmask='255.255.255.0'/>
  <ip family='ipv6' address='fd00::1' prefix='64'/>
</network>
"#;
        let cfg = parse(xml).unwrap();
        assert!(cfg.ipv4.is_some());
        assert!(cfg.ipv6.is_some());
    }

    #[test]
    fn ip_summary_with_prefix() {
        let ip = IpConfig {
            address: "10.0.0.1".into(),
            prefix: Some(24),
            ..Default::default()
        };
        assert_eq!(ip_summary(&ip), "10.0.0.1/24");
    }

    #[test]
    fn ip_summary_from_netmask() {
        let ip = IpConfig {
            address: "192.168.1.1".into(),
            netmask: Some("255.255.255.0".into()),
            ..Default::default()
        };
        assert_eq!(ip_summary(&ip), "192.168.1.1/24");
    }

    #[test]
    fn netmask_to_prefix_common_masks() {
        assert_eq!(netmask_to_prefix(Some("255.255.255.0")), 24);
        assert_eq!(netmask_to_prefix(Some("255.255.0.0")), 16);
        assert_eq!(netmask_to_prefix(Some("255.0.0.0")), 8);
        assert_eq!(netmask_to_prefix(None), 24);
    }

    #[test]
    fn invalid_xml_returns_error() {
        let result = parse("not xml");
        assert!(result.is_err() || result.as_ref().unwrap().name.is_empty());
    }

    #[test]
    fn build_nat_xml_minimal() {
        let xml = build_nat_network_xml("test", "virbr99", "10.99.0.1", "255.255.255.0", None, None);
        assert!(xml.contains("<name>test</name>"));
        assert!(xml.contains("virbr99"));
        assert!(xml.contains("10.99.0.1"));
        assert!(!xml.contains("<dhcp>"));
    }

    #[test]
    fn build_nat_xml_with_dhcp() {
        let xml = build_nat_network_xml(
            "test",
            "virbr99",
            "10.99.0.1",
            "255.255.255.0",
            Some("10.99.0.100"),
            Some("10.99.0.200"),
        );
        assert!(xml.contains("<dhcp>"));
        assert!(xml.contains("start='10.99.0.100'"));
        assert!(xml.contains("end='10.99.0.200'"));
    }

    #[test]
    fn build_nat_xml_escapes_injection() {
        let xml = build_nat_network_xml(
            "a'><evil/>x",
            "virbr0",
            "1.2.3.4",
            "255.255.255.0",
            None,
            None,
        );
        assert!(!xml.contains("<evil/>"));
        assert!(xml.contains("&apos;") || xml.contains("&quot;"));
    }

    #[test]
    fn parse_build_roundtrip() {
        let xml = build_nat_network_xml(
            "rt",
            "virbrRT",
            "172.16.0.1",
            "255.255.0.0",
            Some("172.16.0.100"),
            Some("172.16.255.254"),
        );
        let cfg = parse(&xml).unwrap();
        assert_eq!(cfg.name, "rt");
        assert_eq!(cfg.bridge, Some("virbrRT".into()));
        assert_eq!(cfg.forward_mode, "nat");
        let v4 = cfg.ipv4.unwrap();
        assert_eq!(v4.address, "172.16.0.1");
        assert_eq!(v4.dhcp_ranges.len(), 1);
    }

    fn nat_v4(name: &str) -> NetworkBuildParams {
        NetworkBuildParams {
            name,
            forward_mode: "nat",
            bridge_name: "virbr10",
            forward_dev: None,
            domain_name: None,
            ipv4: Some(Ipv4BuildParams {
                address: "192.0.2.1",
                netmask: "255.255.255.0",
                dhcp_start: Some("192.0.2.100"),
                dhcp_end: Some("192.0.2.200"),
            }),
            ipv6: None,
        }
    }

    #[test]
    fn builder_nat_matches_wrapper() {
        let a = build_network_xml(&nat_v4("n"));
        let b = build_nat_network_xml("n", "virbr10", "192.0.2.1", "255.255.255.0",
            Some("192.0.2.100"), Some("192.0.2.200"));
        assert_eq!(a, b);
    }

    #[test]
    fn builder_isolated_omits_forward() {
        let mut p = nat_v4("iso");
        p.forward_mode = "isolated";
        let xml = build_network_xml(&p);
        assert!(!xml.contains("<forward"), "isolated should have no <forward> element");
        assert!(xml.contains("<bridge name='virbr10'/>"));
        assert!(xml.contains("<ip address='192.0.2.1'"));
    }

    #[test]
    fn builder_empty_mode_is_isolated() {
        let mut p = nat_v4("iso2");
        p.forward_mode = "";
        let xml = build_network_xml(&p);
        assert!(!xml.contains("<forward"));
    }

    #[test]
    fn builder_route_mode() {
        let mut p = nat_v4("r");
        p.forward_mode = "route";
        let xml = build_network_xml(&p);
        assert!(xml.contains("<forward mode='route'/>"));
        assert!(xml.contains("<ip address='192.0.2.1'"));
    }

    #[test]
    fn builder_route_with_uplink_dev() {
        let mut p = nat_v4("r2");
        p.forward_mode = "route";
        p.forward_dev = Some("eth0");
        let xml = build_network_xml(&p);
        assert!(xml.contains("<forward mode='route' dev='eth0'/>"));
    }

    #[test]
    fn builder_open_mode() {
        let mut p = nat_v4("o");
        p.forward_mode = "open";
        let xml = build_network_xml(&p);
        assert!(xml.contains("<forward mode='open'/>"));
        assert!(xml.contains("<ip address='192.0.2.1'"));
    }

    #[test]
    fn builder_bridge_mode_omits_ip_config() {
        let p = NetworkBuildParams {
            name: "host-br",
            forward_mode: "bridge",
            bridge_name: "br0",
            forward_dev: None,
            domain_name: None,
            ipv4: Some(Ipv4BuildParams {
                address: "1.2.3.4",
                netmask: "255.255.255.0",
                dhcp_start: None,
                dhcp_end: None,
            }),
            ipv6: None,
        };
        let xml = build_network_xml(&p);
        assert!(xml.contains("<forward mode='bridge'/>"));
        assert!(xml.contains("<bridge name='br0'/>"));
        assert!(!xml.contains("<ip "), "bridge mode must not emit <ip> config");
        assert!(!xml.contains("<dhcp"));
    }

    #[test]
    fn builder_includes_domain_name() {
        let mut p = nat_v4("d");
        p.domain_name = Some("lab.local");
        let xml = build_network_xml(&p);
        assert!(xml.contains("<domain name='lab.local'/>"));
    }

    #[test]
    fn builder_omits_empty_domain_name() {
        let mut p = nat_v4("d");
        p.domain_name = Some("");
        let xml = build_network_xml(&p);
        assert!(!xml.contains("<domain"));
    }

    #[test]
    fn builder_ipv6_only() {
        let p = NetworkBuildParams {
            name: "v6",
            forward_mode: "nat",
            bridge_name: "virbr6",
            forward_dev: None,
            domain_name: None,
            ipv4: None,
            ipv6: Some(Ipv6BuildParams {
                address: "fd00::1",
                prefix: 64,
                dhcp_start: Some("fd00::100"),
                dhcp_end: Some("fd00::200"),
            }),
        };
        let xml = build_network_xml(&p);
        assert!(xml.contains("<ip family='ipv6' address='fd00::1' prefix='64'>"));
        assert!(xml.contains("start='fd00::100'"));
        assert!(!xml.contains("family='ipv4'"));
    }

    #[test]
    fn builder_dual_stack() {
        let p = NetworkBuildParams {
            name: "dual",
            forward_mode: "nat",
            bridge_name: "virbrDual",
            forward_dev: None,
            domain_name: None,
            ipv4: Some(Ipv4BuildParams {
                address: "10.0.0.1",
                netmask: "255.255.255.0",
                dhcp_start: None,
                dhcp_end: None,
            }),
            ipv6: Some(Ipv6BuildParams {
                address: "fd00::1",
                prefix: 64,
                dhcp_start: None,
                dhcp_end: None,
            }),
        };
        let xml = build_network_xml(&p);
        let cfg = parse(&xml).unwrap();
        assert!(cfg.ipv4.is_some());
        assert!(cfg.ipv6.is_some());
    }

    #[test]
    fn builder_skips_dhcp_when_partial() {
        let mut p = nat_v4("n");
        if let Some(ref mut v4) = p.ipv4 {
            v4.dhcp_start = Some("");
            v4.dhcp_end = Some("x");
        }
        let xml = build_network_xml(&p);
        assert!(!xml.contains("<dhcp>"));
    }

    #[test]
    fn builder_escapes_all_user_input() {
        let p = NetworkBuildParams {
            name: "a'<inject>",
            forward_mode: "nat",
            bridge_name: "br'<",
            forward_dev: Some("e'<"),
            domain_name: Some("d'<"),
            ipv4: Some(Ipv4BuildParams {
                address: "1'<",
                netmask: "2'<",
                dhcp_start: Some("3'<"),
                dhcp_end: Some("4'<"),
            }),
            ipv6: None,
        };
        let xml = build_network_xml(&p);
        // No unescaped angle brackets inside attributes / text
        assert!(!xml.contains("<inject>"));
        // Actual escape sequences present
        assert!(xml.contains("&lt;"));
    }

    #[test]
    fn builder_route_roundtrip_parses() {
        let mut p = nat_v4("route-rt");
        p.forward_mode = "route";
        p.forward_dev = Some("eth1");
        let xml = build_network_xml(&p);
        let cfg = parse(&xml).unwrap();
        assert_eq!(cfg.forward_mode, "route");
        assert_eq!(cfg.forward_dev.as_deref(), Some("eth1"));
    }

    // ── DNS host parsing + dhcp/dns host snippet builders ──

    #[test]
    fn parses_dns_hosts_with_multiple_hostnames() {
        let xml = r#"<network>
  <name>n1</name>
  <forward mode='nat'/>
  <dns>
    <host ip='192.168.122.10'>
      <hostname>foo</hostname>
      <hostname>foo.lan</hostname>
    </host>
    <host ip='192.168.122.11'><hostname>bar</hostname></host>
  </dns>
  <ip address='192.168.122.1' netmask='255.255.255.0'/>
</network>"#;
        let cfg = parse(xml).unwrap();
        assert_eq!(cfg.dns_hosts.len(), 2);
        assert_eq!(cfg.dns_hosts[0].ip, "192.168.122.10");
        assert_eq!(cfg.dns_hosts[0].hostnames, vec!["foo", "foo.lan"]);
        assert_eq!(cfg.dns_hosts[1].hostnames, vec!["bar"]);
    }

    #[test]
    fn dhcp_host_xml_omits_optional_attrs() {
        let only_ip = build_dhcp_host_xml(None, None, "192.168.122.50");
        assert_eq!(only_ip, "<host ip='192.168.122.50'/>");
        let with_mac = build_dhcp_host_xml(Some("52:54:00:aa:bb:cc"), Some("ws"), "192.168.122.50");
        assert_eq!(
            with_mac,
            "<host mac='52:54:00:aa:bb:cc' name='ws' ip='192.168.122.50'/>"
        );
    }

    #[test]
    fn dns_host_xml_with_hostnames() {
        let xml = build_dns_host_xml("10.0.0.5", &["primary".into(), "primary.lan".into()]);
        assert_eq!(
            xml,
            "<host ip='10.0.0.5'><hostname>primary</hostname><hostname>primary.lan</hostname></host>"
        );
    }

    // ── Static routes ──

    #[test]
    fn parses_static_routes_under_network() {
        let xml = r#"<network>
  <name>n</name>
  <forward mode='route'/>
  <ip address='192.168.222.1' netmask='255.255.255.0'/>
  <route family='ipv4' address='10.0.0.0' prefix='8' gateway='192.168.222.5'/>
  <route address='192.168.99.0' prefix='24' gateway='192.168.222.6'/>
</network>"#;
        let cfg = parse(xml).unwrap();
        assert_eq!(cfg.routes.len(), 2);
        assert_eq!(cfg.routes[0].family, "ipv4");
        assert_eq!(cfg.routes[0].address, "10.0.0.0");
        assert_eq!(cfg.routes[0].prefix, 8);
        assert_eq!(cfg.routes[0].gateway, "192.168.222.5");
        assert_eq!(cfg.routes[1].family, "ipv4"); // default
    }

    #[test]
    fn add_route_inserts_before_close_tag() {
        let xml = "<network>\n  <name>n</name>\n</network>\n";
        let r = NetworkRoute {
            family: "ipv4".into(),
            address: "10.0.0.0".into(),
            prefix: 8,
            gateway: "192.168.122.1".into(),
        };
        let out = add_route_to_network_xml(xml, &r);
        assert!(out.contains("<route family='ipv4' address='10.0.0.0' prefix='8' gateway='192.168.122.1'/>"));
        // Order: name first, route second, close tag last.
        let name_at = out.find("<name>").unwrap();
        let route_at = out.find("<route").unwrap();
        let close_at = out.find("</network>").unwrap();
        assert!(name_at < route_at);
        assert!(route_at < close_at);
    }

    #[test]
    fn remove_route_removes_only_match() {
        let xml = "<network>\n  <name>n</name>\n  <route family='ipv4' address='10.0.0.0' prefix='8' gateway='192.168.122.1'/>\n  <route family='ipv4' address='10.1.0.0' prefix='16' gateway='192.168.122.2'/>\n</network>\n";
        let kill = NetworkRoute {
            family: "ipv4".into(),
            address: "10.0.0.0".into(),
            prefix: 8,
            gateway: "192.168.122.1".into(),
        };
        let out = remove_route_from_network_xml(xml, &kill);
        assert!(!out.contains("10.0.0.0"));
        assert!(out.contains("10.1.0.0"));
    }

    #[test]
    fn dhcp_host_xml_escapes_xml() {
        let s = build_dhcp_host_xml(None, Some("<script>"), "1.2.3.4");
        assert!(s.contains("&lt;script&gt;"));
    }
}
