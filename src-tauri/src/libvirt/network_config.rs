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
        _ => {}
    }
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
/// Used by the creation wizard.
pub fn build_nat_network_xml(
    name: &str,
    bridge: &str,
    ipv4_address: &str,
    ipv4_netmask: &str,
    dhcp_start: Option<&str>,
    dhcp_end: Option<&str>,
) -> String {
    use crate::libvirt::xml_helpers::escape_xml;

    let mut xml = format!(
        "<network>\n  <name>{}</name>\n  <forward mode='nat'/>\n  <bridge name='{}'/>\n  <ip address='{}' netmask='{}'>\n",
        escape_xml(name),
        escape_xml(bridge),
        escape_xml(ipv4_address),
        escape_xml(ipv4_netmask),
    );

    if let (Some(s), Some(e)) = (dhcp_start, dhcp_end) {
        xml.push_str(&format!(
            "    <dhcp>\n      <range start='{}' end='{}'/>\n    </dhcp>\n",
            escape_xml(s),
            escape_xml(e),
        ));
    }

    xml.push_str("  </ip>\n</network>\n");
    xml
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
}
