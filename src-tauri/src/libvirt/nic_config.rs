//! Network interface (NIC) configuration — parse, build, and patch
//! `<interface>` entries under `<devices>`.
//!
//! Mirrors the structure of `boot_config.rs` / `hostdev.rs`: we keep the
//! raw domain XML intact and only mutate the specific `<interface>`
//! elements that the user touched, so unknown / unsupported child
//! elements (e.g. SR-IOV virtualport, nwfilter parameters we have not
//! modelled) round-trip byte-for-byte through add / update / remove.
//!
//! vhost-net note: using `model=virtio` with `driver_queues > 1` requires
//! `vhost-net` support in the host kernel. Libvirt will still accept the
//! XML if vhost-net is missing; QEMU will fall back to `tap` at startup
//! and throughput drops dramatically. We do not error on this — callers
//! who want stricter validation should probe capabilities.

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

// ──────────────────────────────────────────────────────────────────────
// Model
// ──────────────────────────────────────────────────────────────────────

/// Where the guest NIC is connected on the host side.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NicSource {
    /// `<interface type='network'><source network='NAME'/>` — a libvirt
    /// virtual network.
    Network { name: String },
    /// `<interface type='bridge'><source bridge='NAME'/>` — a host
    /// Linux bridge managed outside libvirt.
    Bridge { name: String },
    /// `<interface type='direct'><source dev='eth0' mode='vepa'/>` —
    /// macvtap onto a physical interface.
    Direct { dev: String, mode: String },
    /// `<interface type='user'>` — SLIRP userspace networking.
    User,
    /// `<interface type='hostdev'><source><address .../></source>` —
    /// PCI or USB NIC passed through.
    Hostdev { addr: HostdevAddress },
    /// `<interface type='vhostuser'><source type='unix' path='...' mode='client|server'/>`
    Vhostuser { socket_path: String, mode: String },
}

/// Address family for a hostdev NIC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "addr_type", rename_all = "snake_case")]
pub enum HostdevAddress {
    Pci {
        domain: u16,
        bus: u8,
        slot: u8,
        function: u8,
    },
    Usb {
        bus: u8,
        device: u8,
    },
}

/// Bandwidth limits (`<bandwidth>`). Units are KiB/s and KiB.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BandwidthLimits {
    pub average: Option<u64>,
    pub peak: Option<u64>,
    pub burst: Option<u64>,
}

impl BandwidthLimits {
    fn is_empty(&self) -> bool {
        self.average.is_none() && self.peak.is_none() && self.burst.is_none()
    }
}

/// A single `<interface>` device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NicConfig {
    pub source: NicSource,
    pub model: Option<String>,
    /// MAC address. `None` = libvirt auto-generates (52:54:00:*).
    pub mac: Option<String>,
    /// Guest-side interface name (e.g. `vnet3`). Usually blank;
    /// libvirt assigns on start.
    pub target_dev: Option<String>,
    /// `<link state='up|down'/>`. Unset = defaults to up.
    pub link_state: Option<String>,
    pub mtu: Option<u32>,
    pub boot_order: Option<u32>,
    pub bandwidth_inbound: BandwidthLimits,
    pub bandwidth_outbound: BandwidthLimits,
    pub driver_queues: Option<u32>,
    pub driver_txmode: Option<String>,
    pub filterref: Option<String>,
    pub vlan_tag: Option<u16>,
    pub port_isolated: bool,
    /// Emit `<virtualport type='openvswitch'/>` so libvirt plugs the
    /// vNIC into an OVS bridge instead of a Linux bridge. Required when
    /// the host bridge is OVS-managed.
    pub is_openvswitch: bool,
}

impl Default for NicConfig {
    fn default() -> Self {
        Self {
            source: NicSource::User,
            model: None,
            mac: None,
            target_dev: None,
            link_state: None,
            mtu: None,
            boot_order: None,
            bandwidth_inbound: BandwidthLimits::default(),
            bandwidth_outbound: BandwidthLimits::default(),
            driver_queues: None,
            driver_txmode: None,
            filterref: None,
            vlan_tag: None,
            port_isolated: false,
            is_openvswitch: false,
        }
    }
}

pub const DIRECT_MODES: &[&str] = &["bridge", "vepa", "private", "passthrough"];
pub const NIC_MODELS: &[&str] = &[
    "virtio", "virtio-transitional", "e1000", "e1000e", "rtl8139", "pcnet", "ne2k_pci",
];

// ──────────────────────────────────────────────────────────────────────
// Validation
// ──────────────────────────────────────────────────────────────────────

pub fn validate_mac(mac: &str) -> Result<(), VirtManagerError> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return Err(VirtManagerError::OperationFailed { operation: "nic_validate".into(),
            reason: format!("MAC '{mac}' must have 6 colon-separated octets"),
        });
    }
    for p in parts {
        if p.len() != 2 || !p.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(VirtManagerError::OperationFailed { operation: "nic_validate".into(),
                reason: format!("MAC '{mac}' has invalid octet '{p}'"),
            });
        }
    }
    Ok(())
}

fn normalise_mac(mac: &str) -> String {
    mac.to_ascii_lowercase()
}

pub fn validate(nic: &NicConfig) -> Result<(), VirtManagerError> {
    if let Some(ref m) = nic.mac {
        validate_mac(m)?;
    }
    if let NicSource::Direct { mode, .. } = &nic.source {
        if !DIRECT_MODES.contains(&mode.as_str()) {
            return Err(VirtManagerError::OperationFailed { operation: "nic_validate".into(),
                reason: format!("direct mode '{mode}' not in {DIRECT_MODES:?}"),
            });
        }
    }
    if let NicSource::Vhostuser { mode, .. } = &nic.source {
        if mode != "client" && mode != "server" {
            return Err(VirtManagerError::OperationFailed { operation: "nic_validate".into(),
                reason: format!("vhostuser mode '{mode}' must be client or server"),
            });
        }
        if nic.bandwidth_inbound.average.is_some() || nic.bandwidth_outbound.average.is_some() {
            return Err(VirtManagerError::OperationFailed { operation: "nic_validate".into(),
                reason: "vhostuser does not support <bandwidth>".into(),
            });
        }
        if nic.filterref.is_some() {
            return Err(VirtManagerError::OperationFailed { operation: "nic_validate".into(),
                reason: "vhostuser does not support <filterref>".into(),
            });
        }
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────
// Parse
// ──────────────────────────────────────────────────────────────────────

pub fn parse_nics(xml: &str) -> Result<Vec<NicConfig>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut out: Vec<NicConfig> = Vec::new();
    let mut cur: Option<NicState> = None;
    let mut path: Vec<String> = Vec::new();

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(VirtManagerError::XmlParsingFailed {
                reason: format!("nic parse at {}: {}", r.buffer_position(), e),
            }),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if n == "interface" {
                    cur = Some(NicState::new(get_attr(&a, "type").unwrap_or_default()));
                } else if let Some(ref mut s) = cur {
                    handle_child(n.as_str(), &a, &path, s);
                }
                path.push(n);
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if let Some(ref mut s) = cur {
                    handle_child(n.as_str(), &a, &path, s);
                }
            }
            Ok(Event::End(e)) => {
                let n = utf8_name_end(&e);
                path.pop();
                if n == "interface" {
                    if let Some(s) = cur.take() {
                        if let Some(nic) = s.finish() {
                            out.push(nic);
                        }
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

struct NicState {
    iface_type: String,
    src_network: Option<String>,
    src_bridge: Option<String>,
    src_dev: Option<String>,
    src_mode: Option<String>,
    src_path: Option<String>,
    src_sock_mode: Option<String>,
    src_pci_domain: Option<u16>,
    src_pci_bus: Option<u8>,
    src_pci_slot: Option<u8>,
    src_pci_func: Option<u8>,
    src_usb_bus: Option<u8>,
    src_usb_device: Option<u8>,
    mac: Option<String>,
    model: Option<String>,
    target_dev: Option<String>,
    link_state: Option<String>,
    mtu: Option<u32>,
    boot_order: Option<u32>,
    bw_in: BandwidthLimits,
    bw_out: BandwidthLimits,
    driver_queues: Option<u32>,
    driver_txmode: Option<String>,
    filterref: Option<String>,
    vlan_tag: Option<u16>,
    port_isolated: bool,
    is_openvswitch: bool,
}

impl NicState {
    fn new(iface_type: String) -> Self {
        Self {
            iface_type,
            src_network: None, src_bridge: None, src_dev: None, src_mode: None,
            src_path: None, src_sock_mode: None,
            src_pci_domain: None, src_pci_bus: None, src_pci_slot: None, src_pci_func: None,
            src_usb_bus: None, src_usb_device: None,
            mac: None, model: None, target_dev: None, link_state: None,
            mtu: None, boot_order: None,
            bw_in: BandwidthLimits::default(), bw_out: BandwidthLimits::default(),
            driver_queues: None, driver_txmode: None, filterref: None,
            vlan_tag: None, port_isolated: false,
            is_openvswitch: false,
        }
    }

    fn finish(self) -> Option<NicConfig> {
        let source = match self.iface_type.as_str() {
            "network" => NicSource::Network { name: self.src_network.unwrap_or_default() },
            "bridge" => NicSource::Bridge { name: self.src_bridge.unwrap_or_default() },
            "direct" => NicSource::Direct {
                dev: self.src_dev.unwrap_or_default(),
                mode: self.src_mode.unwrap_or_else(|| "bridge".into()),
            },
            "user" => NicSource::User,
            "hostdev" => {
                if let (Some(d), Some(b), Some(s), Some(f)) = (self.src_pci_domain, self.src_pci_bus, self.src_pci_slot, self.src_pci_func) {
                    NicSource::Hostdev { addr: HostdevAddress::Pci { domain: d, bus: b, slot: s, function: f } }
                } else if let (Some(b), Some(d)) = (self.src_usb_bus, self.src_usb_device) {
                    NicSource::Hostdev { addr: HostdevAddress::Usb { bus: b, device: d } }
                } else {
                    return None;
                }
            }
            "vhostuser" => NicSource::Vhostuser {
                socket_path: self.src_path.unwrap_or_default(),
                mode: self.src_sock_mode.unwrap_or_else(|| "client".into()),
            },
            _ => return None,
        };
        Some(NicConfig {
            source, model: self.model, mac: self.mac, target_dev: self.target_dev,
            link_state: self.link_state, mtu: self.mtu, boot_order: self.boot_order,
            bandwidth_inbound: self.bw_in, bandwidth_outbound: self.bw_out,
            driver_queues: self.driver_queues, driver_txmode: self.driver_txmode,
            filterref: self.filterref, vlan_tag: self.vlan_tag,
            port_isolated: self.port_isolated,
            is_openvswitch: self.is_openvswitch,
        })
    }
}

fn handle_child(n: &str, a: &[(String, String)], path: &[String], s: &mut NicState) {
    let parent = path.last().map(String::as_str);
    match (parent, n) {
        (Some("interface"), "mac") => s.mac = get_attr(a, "address").map(|m| normalise_mac(&m)),
        (Some("interface"), "model") => s.model = get_attr(a, "type"),
        (Some("interface"), "target") => s.target_dev = get_attr(a, "dev"),
        (Some("interface"), "link") => s.link_state = get_attr(a, "state"),
        (Some("interface"), "mtu") => s.mtu = get_attr(a, "size").and_then(|v| v.parse().ok()),
        (Some("interface"), "boot") => s.boot_order = get_attr(a, "order").and_then(|v| v.parse().ok()),
        (Some("interface"), "filterref") => s.filterref = get_attr(a, "filter"),
        (Some("interface"), "port") => s.port_isolated = get_attr(a, "isolated").as_deref() == Some("yes"),
        (Some("interface"), "driver") => {
            s.driver_queues = get_attr(a, "queues").and_then(|v| v.parse().ok());
            s.driver_txmode = get_attr(a, "txmode");
        }
        (Some("interface"), "source") => match s.iface_type.as_str() {
            "network" => s.src_network = get_attr(a, "network"),
            "bridge" => s.src_bridge = get_attr(a, "bridge"),
            "direct" => {
                s.src_dev = get_attr(a, "dev");
                s.src_mode = get_attr(a, "mode");
            }
            "vhostuser" => {
                s.src_path = get_attr(a, "path");
                s.src_sock_mode = get_attr(a, "mode");
            }
            _ => {}
        },
        (Some("source"), "address") => {
            // Only meaningful inside <interface type='hostdev'>
            if s.iface_type == "hostdev" {
                if let Some(dom) = get_attr(a, "domain") {
                    s.src_pci_domain = parse_hex_u16(&dom);
                    s.src_pci_bus = get_attr(a, "bus").and_then(|v| parse_hex_u8(&v));
                    s.src_pci_slot = get_attr(a, "slot").and_then(|v| parse_hex_u8(&v));
                    s.src_pci_func = get_attr(a, "function").and_then(|v| parse_hex_u8(&v));
                } else {
                    s.src_usb_bus = get_attr(a, "bus").and_then(|v| v.parse().ok());
                    s.src_usb_device = get_attr(a, "device").and_then(|v| v.parse().ok());
                }
            }
        }
        (Some("vlan"), "tag") => {
            s.vlan_tag = get_attr(a, "id").and_then(|v| v.parse().ok());
        }
        (Some("interface"), "virtualport") => {
            // <virtualport type='openvswitch'/>
            if get_attr(a, "type").as_deref() == Some("openvswitch") {
                s.is_openvswitch = true;
            }
        }
        (Some("bandwidth"), "inbound") => {
            s.bw_in.average = get_attr(a, "average").and_then(|v| v.parse().ok());
            s.bw_in.peak = get_attr(a, "peak").and_then(|v| v.parse().ok());
            s.bw_in.burst = get_attr(a, "burst").and_then(|v| v.parse().ok());
        }
        (Some("bandwidth"), "outbound") => {
            s.bw_out.average = get_attr(a, "average").and_then(|v| v.parse().ok());
            s.bw_out.peak = get_attr(a, "peak").and_then(|v| v.parse().ok());
            s.bw_out.burst = get_attr(a, "burst").and_then(|v| v.parse().ok());
        }
        _ => {}
    }
    let _ = path; // silence unused if we ever drop path below
}

// ──────────────────────────────────────────────────────────────────────
// Build
// ──────────────────────────────────────────────────────────────────────

pub fn build_nic_xml(nic: &NicConfig) -> String {
    let iface_type = match &nic.source {
        NicSource::Network { .. } => "network",
        NicSource::Bridge { .. } => "bridge",
        NicSource::Direct { .. } => "direct",
        NicSource::User => "user",
        NicSource::Hostdev { .. } => "hostdev",
        NicSource::Vhostuser { .. } => "vhostuser",
    };

    let mut s = String::new();
    s.push_str(&format!("<interface type='{iface_type}'>\n"));

    if let Some(ref mac) = nic.mac {
        if validate_mac(mac).is_ok() {
            s.push_str(&format!("  <mac address='{}'/>\n", escape_xml(&normalise_mac(mac))));
        }
    }

    match &nic.source {
        NicSource::Network { name } => {
            s.push_str(&format!("  <source network='{}'/>\n", escape_xml(name)));
        }
        NicSource::Bridge { name } => {
            s.push_str(&format!("  <source bridge='{}'/>\n", escape_xml(name)));
        }
        NicSource::Direct { dev, mode } => {
            s.push_str(&format!("  <source dev='{}' mode='{}'/>\n", escape_xml(dev), escape_xml(mode)));
        }
        NicSource::User => {}
        NicSource::Hostdev { addr } => match addr {
            HostdevAddress::Pci { domain, bus, slot, function } => {
                s.push_str(&format!(
                    "  <source>\n    <address type='pci' domain='0x{domain:04x}' bus='0x{bus:02x}' slot='0x{slot:02x}' function='0x{function:x}'/>\n  </source>\n"
                ));
            }
            HostdevAddress::Usb { bus, device } => {
                s.push_str(&format!(
                    "  <source>\n    <address type='usb' bus='{bus}' device='{device}'/>\n  </source>\n"
                ));
            }
        },
        NicSource::Vhostuser { socket_path, mode } => {
            s.push_str(&format!(
                "  <source type='unix' path='{}' mode='{}'/>\n",
                escape_xml(socket_path), escape_xml(mode)
            ));
        }
    }

    if let Some(ref m) = nic.model {
        s.push_str(&format!("  <model type='{}'/>\n", escape_xml(m)));
    }
    if let Some(ref td) = nic.target_dev {
        if !td.is_empty() {
            s.push_str(&format!("  <target dev='{}'/>\n", escape_xml(td)));
        }
    }
    if let Some(ref ls) = nic.link_state {
        s.push_str(&format!("  <link state='{}'/>\n", escape_xml(ls)));
    }
    if let Some(mtu) = nic.mtu {
        s.push_str(&format!("  <mtu size='{mtu}'/>\n"));
    }
    if let Some(order) = nic.boot_order {
        s.push_str(&format!("  <boot order='{order}'/>\n"));
    }
    if let Some(tag) = nic.vlan_tag {
        s.push_str(&format!("  <vlan>\n    <tag id='{tag}'/>\n  </vlan>\n"));
    }
    if nic.is_openvswitch {
        s.push_str("  <virtualport type='openvswitch'/>\n");
    }
    if nic.port_isolated {
        s.push_str("  <port isolated='yes'/>\n");
    }
    if nic.driver_queues.is_some() || nic.driver_txmode.is_some() {
        let q = match nic.driver_queues {
            Some(n) => format!(" queues='{n}'"),
            None => String::new(),
        };
        let tx = match nic.driver_txmode {
            Some(ref m) => format!(" txmode='{}'", escape_xml(m)),
            None => String::new(),
        };
        s.push_str(&format!("  <driver{q}{tx}/>\n"));
    }
    if let Some(ref f) = nic.filterref {
        s.push_str(&format!("  <filterref filter='{}'/>\n", escape_xml(f)));
    }
    if !nic.bandwidth_inbound.is_empty() || !nic.bandwidth_outbound.is_empty() {
        s.push_str("  <bandwidth>\n");
        if !nic.bandwidth_inbound.is_empty() {
            s.push_str("    <inbound");
            if let Some(a) = nic.bandwidth_inbound.average { s.push_str(&format!(" average='{a}'")); }
            if let Some(p) = nic.bandwidth_inbound.peak { s.push_str(&format!(" peak='{p}'")); }
            if let Some(b) = nic.bandwidth_inbound.burst { s.push_str(&format!(" burst='{b}'")); }
            s.push_str("/>\n");
        }
        if !nic.bandwidth_outbound.is_empty() {
            s.push_str("    <outbound");
            if let Some(a) = nic.bandwidth_outbound.average { s.push_str(&format!(" average='{a}'")); }
            if let Some(p) = nic.bandwidth_outbound.peak { s.push_str(&format!(" peak='{p}'")); }
            if let Some(b) = nic.bandwidth_outbound.burst { s.push_str(&format!(" burst='{b}'")); }
            s.push_str("/>\n");
        }
        s.push_str("  </bandwidth>\n");
    }

    s.push_str("</interface>\n");
    s
}

// ──────────────────────────────────────────────────────────────────────
// Apply (add / remove / update)
// ──────────────────────────────────────────────────────────────────────

pub fn apply_nic_add(xml: &str, nic: &NicConfig) -> Result<String, VirtManagerError> {
    validate(nic)?;
    let frag = build_nic_xml(nic);
    let indented = indent(&frag, 4);
    if let Some(idx) = xml.rfind("</devices>") {
        let mut out = String::with_capacity(xml.len() + indented.len());
        out.push_str(&xml[..idx]);
        out.push_str(&indented);
        out.push_str(&xml[idx..]);
        Ok(out)
    } else if let Some(idx) = xml.rfind("</domain>") {
        let wrapped = format!("  <devices>\n{indented}  </devices>\n");
        let mut out = String::with_capacity(xml.len() + wrapped.len());
        out.push_str(&xml[..idx]);
        out.push_str(&wrapped);
        out.push_str(&xml[idx..]);
        Ok(out)
    } else {
        Err(VirtManagerError::XmlParsingFailed {
            reason: "no </domain> found; cannot insert NIC".into(),
        })
    }
}

pub fn apply_nic_remove_by_mac(xml: &str, mac_or_target: &str) -> Result<String, VirtManagerError> {
    let needle = mac_or_target.to_ascii_lowercase();
    let (start, end) = find_interface_span(xml, &needle)?
        .ok_or_else(|| VirtManagerError::OperationFailed { operation: "nic_validate".into(),
            reason: format!("no interface matching '{mac_or_target}'"),
        })?;

    let mut s = start;
    while s > 0 && matches!(xml.as_bytes()[s - 1], b' ' | b'\t') { s -= 1; }
    let mut e = end;
    if e < xml.len() && xml.as_bytes()[e] == b'\n' { e += 1; }
    let mut out = String::with_capacity(xml.len());
    out.push_str(&xml[..s]);
    out.push_str(&xml[e..]);
    Ok(out)
}

pub fn apply_nic_update(xml: &str, mac_or_target: &str, nic: &NicConfig) -> Result<String, VirtManagerError> {
    validate(nic)?;
    let needle = mac_or_target.to_ascii_lowercase();
    let (start, end) = find_interface_span(xml, &needle)?
        .ok_or_else(|| VirtManagerError::OperationFailed { operation: "nic_validate".into(),
            reason: format!("no interface matching '{mac_or_target}'"),
        })?;
    let frag = build_nic_xml(nic);
    let mut indent_chars = String::new();
    let mut p = start;
    while p > 0 && matches!(xml.as_bytes()[p - 1], b' ' | b'\t') {
        p -= 1;
        indent_chars.insert(0, xml.as_bytes()[p] as char);
    }
    let indented = reindent(&frag, &indent_chars);
    let indented = indented.trim_end_matches('\n').to_string();
    let mut out = String::with_capacity(xml.len() + indented.len());
    out.push_str(&xml[..start]);
    out.push_str(&indented);
    out.push_str(&xml[end..]);
    Ok(out)
}

fn find_interface_span(xml: &str, needle: &str) -> Result<Option<(usize, usize)>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut iface_start: Option<usize> = None;
    let mut inside = false;
    let mut depth: i32 = 0;
    let mut cur_mac: Option<String> = None;
    let mut cur_target: Option<String> = None;

    loop {
        let before_pos = r.buffer_position() as usize;
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(VirtManagerError::XmlParsingFailed {
                reason: format!("find_interface: {e}"),
            }),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                if n == "interface" && !inside {
                    inside = true;
                    depth = 1;
                    iface_start = Some(before_pos);
                    cur_mac = None;
                    cur_target = None;
                } else if inside && n == "interface" {
                    depth += 1;
                }
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                if inside {
                    let a = attrs(&e);
                    if n == "mac" {
                        if let Some(m) = get_attr(&a, "address") {
                            cur_mac = Some(m.to_ascii_lowercase());
                        }
                    } else if n == "target" {
                        if let Some(t) = get_attr(&a, "dev") {
                            cur_target = Some(t.to_ascii_lowercase());
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let n = utf8_name_end(&e);
                if inside && n == "interface" {
                    depth -= 1;
                    if depth == 0 {
                        let after = r.buffer_position() as usize;
                        let mac_match = cur_mac.as_deref() == Some(needle);
                        let tgt_match = cur_target.as_deref() == Some(needle);
                        if mac_match || tgt_match {
                            return Ok(iface_start.map(|s| (s, after)));
                        }
                        inside = false;
                        iface_start = None;
                        cur_mac = None;
                        cur_target = None;
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(None)
}

// ──────────────────────────────────────────────────────────────────────
// helpers
// ──────────────────────────────────────────────────────────────────────

fn indent(text: &str, spaces: usize) -> String {
    let pad: String = " ".repeat(spaces);
    let mut out = String::with_capacity(text.len() + text.matches('\n').count() * spaces);
    for (i, line) in text.lines().enumerate() {
        if i > 0 { out.push('\n'); }
        if !line.is_empty() { out.push_str(&pad); out.push_str(line); }
    }
    if text.ends_with('\n') { out.push('\n'); }
    out
}

fn reindent(text: &str, pad: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.matches('\n').count() * pad.len());
    for (i, line) in text.lines().enumerate() {
        if i > 0 { out.push('\n'); }
        if !line.is_empty() { out.push_str(pad); out.push_str(line); }
    }
    if text.ends_with('\n') { out.push('\n'); }
    out
}

fn utf8_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}
fn utf8_name_end(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}
fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes().filter_map(|a| a.ok()).map(|a| (
        String::from_utf8_lossy(a.key.as_ref()).to_string(),
        a.unescape_value().unwrap_or_default().to_string(),
    )).collect()
}
fn get_attr(a: &[(String, String)], k: &str) -> Option<String> {
    a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
}
fn parse_hex_u16(s: &str) -> Option<u16> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(t, 16).ok()
}
fn parse_hex_u8(s: &str) -> Option<u8> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u8::from_str_radix(t, 16).ok()
}

// ──────────────────────────────────────────────────────────────────────
// tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn domain_with(inner: &str) -> String {
        format!("<domain type='kvm'>\n  <name>t</name>\n  <devices>\n{inner}  </devices>\n</domain>\n")
    }

    #[test]
    fn parse_network_source() {
        let xml = domain_with("    <interface type='network'>\n      <mac address='52:54:00:aa:bb:cc'/>\n      <source network='default'/>\n      <model type='virtio'/>\n    </interface>\n");
        let nics = parse_nics(&xml).unwrap();
        assert_eq!(nics.len(), 1);
        assert_eq!(nics[0].source, NicSource::Network { name: "default".into() });
        assert_eq!(nics[0].model.as_deref(), Some("virtio"));
        assert_eq!(nics[0].mac.as_deref(), Some("52:54:00:aa:bb:cc"));
    }

    #[test]
    fn parse_bridge_source() {
        let xml = domain_with("    <interface type='bridge'>\n      <mac address='52:54:00:11:22:33'/>\n      <source bridge='br0'/>\n      <target dev='vnet4'/>\n      <model type='e1000e'/>\n    </interface>\n");
        let nics = parse_nics(&xml).unwrap();
        assert_eq!(nics[0].source, NicSource::Bridge { name: "br0".into() });
        assert_eq!(nics[0].target_dev.as_deref(), Some("vnet4"));
    }

    #[test]
    fn parse_direct_source() {
        let xml = domain_with("    <interface type='direct'>\n      <source dev='eth0' mode='vepa'/>\n      <model type='virtio'/>\n    </interface>\n");
        let nics = parse_nics(&xml).unwrap();
        match &nics[0].source {
            NicSource::Direct { dev, mode } => {
                assert_eq!(dev, "eth0");
                assert_eq!(mode, "vepa");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_user_source() {
        let xml = domain_with("    <interface type='user'><mac address='52:54:00:dd:ee:ff'/><model type='virtio'/></interface>\n");
        let nics = parse_nics(&xml).unwrap();
        assert_eq!(nics[0].source, NicSource::User);
    }

    #[test]
    fn parse_hostdev_pci() {
        let xml = domain_with("    <interface type='hostdev' managed='yes'>\n      <source>\n        <address type='pci' domain='0x0000' bus='0x03' slot='0x00' function='0x0'/>\n      </source>\n    </interface>\n");
        let nics = parse_nics(&xml).unwrap();
        assert_eq!(nics[0].source, NicSource::Hostdev {
            addr: HostdevAddress::Pci { domain: 0, bus: 3, slot: 0, function: 0 }
        });
    }

    #[test]
    fn parse_vhostuser() {
        let xml = domain_with("    <interface type='vhostuser'><mac address='52:54:00:00:00:01'/><source type='unix' path='/var/run/vhost.sock' mode='client'/><model type='virtio'/></interface>\n");
        let nics = parse_nics(&xml).unwrap();
        match &nics[0].source {
            NicSource::Vhostuser { socket_path, mode } => {
                assert_eq!(socket_path, "/var/run/vhost.sock");
                assert_eq!(mode, "client");
            }
            _ => panic!(),
        }
    }

    fn roundtrip(nic: &NicConfig) -> NicConfig {
        let xml = format!("<domain><devices>\n{}\n</devices></domain>", build_nic_xml(nic));
        let mut got = parse_nics(&xml).unwrap();
        got.pop().expect("one nic")
    }

    #[test]
    fn roundtrip_network() {
        let nic = NicConfig {
            source: NicSource::Network { name: "default".into() },
            model: Some("virtio".into()),
            mac: Some("52:54:00:aa:bb:cc".into()),
            ..Default::default()
        };
        assert_eq!(roundtrip(&nic), nic);
    }

    #[test]
    fn roundtrip_bridge_all_fields() {
        let nic = NicConfig {
            source: NicSource::Bridge { name: "br0".into() },
            model: Some("virtio".into()),
            mac: Some("aa:bb:cc:dd:ee:ff".into()),
            target_dev: Some("vnet7".into()),
            link_state: Some("down".into()),
            mtu: Some(9000),
            boot_order: Some(2),
            bandwidth_inbound: BandwidthLimits { average: Some(1000), peak: Some(2000), burst: Some(1024) },
            bandwidth_outbound: BandwidthLimits { average: Some(500), peak: None, burst: None },
            driver_queues: Some(4),
            driver_txmode: Some("iothread".into()),
            filterref: Some("clean-traffic".into()),
            vlan_tag: Some(42),
            port_isolated: true,
            is_openvswitch: false,
        };
        assert_eq!(roundtrip(&nic), nic);
    }

    #[test]
    fn roundtrip_openvswitch_bridge() {
        let nic = NicConfig {
            source: NicSource::Bridge { name: "ovsbr0".into() },
            model: Some("virtio".into()),
            mac: None,
            target_dev: None,
            link_state: None,
            mtu: None,
            boot_order: None,
            bandwidth_inbound: BandwidthLimits::default(),
            bandwidth_outbound: BandwidthLimits::default(),
            driver_queues: None,
            driver_txmode: None,
            filterref: None,
            vlan_tag: Some(100),
            port_isolated: false,
            is_openvswitch: true,
        };
        let xml = build_nic_xml(&nic);
        assert!(xml.contains("<virtualport type='openvswitch'/>"));
        assert!(xml.contains("<vlan>"));
        assert_eq!(roundtrip(&nic), nic);
    }

    #[test]
    fn roundtrip_direct() {
        let nic = NicConfig {
            source: NicSource::Direct { dev: "enp1s0".into(), mode: "passthrough".into() },
            model: Some("virtio".into()),
            mac: Some("52:54:00:a1:b2:c3".into()),
            ..Default::default()
        };
        assert_eq!(roundtrip(&nic), nic);
    }

    #[test]
    fn roundtrip_hostdev_pci() {
        let nic = NicConfig {
            source: NicSource::Hostdev { addr: HostdevAddress::Pci { domain: 0, bus: 3, slot: 0x10, function: 1 } },
            ..Default::default()
        };
        assert_eq!(roundtrip(&nic), nic);
    }

    #[test]
    fn roundtrip_vhostuser() {
        let nic = NicConfig {
            source: NicSource::Vhostuser { socket_path: "/tmp/s.sock".into(), mode: "server".into() },
            model: Some("virtio".into()),
            mac: Some("52:54:00:aa:bb:cd".into()),
            ..Default::default()
        };
        assert_eq!(roundtrip(&nic), nic);
    }

    #[test]
    fn mac_auto_gen_when_none() {
        let nic = NicConfig {
            source: NicSource::Network { name: "default".into() },
            model: Some("virtio".into()),
            mac: None,
            ..Default::default()
        };
        let frag = build_nic_xml(&nic);
        assert!(!frag.contains("<mac "), "should not emit mac when None");
    }

    #[test]
    fn mac_validation_accepts_and_rejects() {
        validate_mac("52:54:00:aa:bb:cc").unwrap();
        validate_mac("AA:BB:CC:DD:EE:FF").unwrap();
        assert!(validate_mac("52:54:00:aa:bb").is_err());
        assert!(validate_mac("52-54-00-aa-bb-cc").is_err());
        assert!(validate_mac("52:54:00:aa:bb:zz").is_err());
        assert!(validate_mac("525:54:00:aa:bb:cc").is_err());
    }

    #[test]
    fn invalid_mac_rejected_in_validate() {
        let nic = NicConfig {
            source: NicSource::Network { name: "default".into() },
            mac: Some("not-a-mac".into()),
            ..Default::default()
        };
        assert!(validate(&nic).is_err());
    }

    #[test]
    fn link_state_roundtrip() {
        let nic = NicConfig {
            source: NicSource::Network { name: "default".into() },
            link_state: Some("down".into()),
            mac: Some("52:54:00:aa:bb:cc".into()),
            ..Default::default()
        };
        assert_eq!(roundtrip(&nic).link_state.as_deref(), Some("down"));
    }

    #[test]
    fn bandwidth_roundtrip() {
        let nic = NicConfig {
            source: NicSource::Bridge { name: "br0".into() },
            bandwidth_inbound: BandwidthLimits { average: Some(100_000), peak: Some(200_000), burst: Some(1024) },
            bandwidth_outbound: BandwidthLimits { average: Some(50_000), peak: None, burst: None },
            mac: Some("52:54:00:aa:bb:cc".into()),
            ..Default::default()
        };
        let got = roundtrip(&nic);
        assert_eq!(got.bandwidth_inbound.average, Some(100_000));
        assert_eq!(got.bandwidth_inbound.peak, Some(200_000));
        assert_eq!(got.bandwidth_outbound.average, Some(50_000));
        assert_eq!(got.bandwidth_outbound.peak, None);
    }

    #[test]
    fn vlan_tag_roundtrip() {
        let nic = NicConfig {
            source: NicSource::Bridge { name: "br0".into() },
            vlan_tag: Some(123),
            mac: Some("52:54:00:aa:bb:cc".into()),
            ..Default::default()
        };
        assert_eq!(roundtrip(&nic).vlan_tag, Some(123));
    }

    #[test]
    fn driver_queues_and_txmode_roundtrip() {
        let nic = NicConfig {
            source: NicSource::Network { name: "default".into() },
            model: Some("virtio".into()),
            driver_queues: Some(8),
            driver_txmode: Some("iothread".into()),
            mac: Some("52:54:00:aa:bb:cc".into()),
            ..Default::default()
        };
        let got = roundtrip(&nic);
        assert_eq!(got.driver_queues, Some(8));
        assert_eq!(got.driver_txmode.as_deref(), Some("iothread"));
    }

    #[test]
    fn filterref_roundtrip() {
        let nic = NicConfig {
            source: NicSource::Network { name: "default".into() },
            filterref: Some("clean-traffic".into()),
            mac: Some("52:54:00:aa:bb:cc".into()),
            ..Default::default()
        };
        assert_eq!(roundtrip(&nic).filterref.as_deref(), Some("clean-traffic"));
    }

    #[test]
    fn multiple_nics_preserve_order() {
        let xml = domain_with("    <interface type='network'><mac address='52:54:00:00:00:01'/><source network='default'/><model type='virtio'/></interface>\n    <interface type='bridge'><mac address='52:54:00:00:00:02'/><source bridge='br0'/><model type='e1000'/></interface>\n    <interface type='user'><mac address='52:54:00:00:00:03'/><model type='virtio'/></interface>\n");
        let nics = parse_nics(&xml).unwrap();
        assert_eq!(nics.len(), 3);
        assert_eq!(nics[0].mac.as_deref(), Some("52:54:00:00:00:01"));
        assert_eq!(nics[1].mac.as_deref(), Some("52:54:00:00:00:02"));
        assert_eq!(nics[2].mac.as_deref(), Some("52:54:00:00:00:03"));
        assert!(matches!(nics[0].source, NicSource::Network { .. }));
        assert!(matches!(nics[1].source, NicSource::Bridge { .. }));
        assert!(matches!(nics[2].source, NicSource::User));
    }

    #[test]
    fn injection_safe_escape_in_source_name() {
        let nic = NicConfig {
            source: NicSource::Network { name: "pwn'/><evil>x</evil><foo bar='".into() },
            mac: Some("52:54:00:aa:bb:cc".into()),
            ..Default::default()
        };
        let xml = build_nic_xml(&nic);
        assert!(!xml.contains("<evil>"), "unescaped injection: {xml}");
        assert!(xml.contains("&lt;evil&gt;"));
        assert!(xml.contains("&apos;"));
    }

    #[test]
    fn empty_nic_list() {
        let xml = "<domain type='kvm'><name>t</name><devices><emulator>/usr/bin/qemu</emulator></devices></domain>";
        let nics = parse_nics(xml).unwrap();
        assert!(nics.is_empty());
    }

    #[test]
    fn vhostuser_rejects_bandwidth() {
        let nic = NicConfig {
            source: NicSource::Vhostuser { socket_path: "/s".into(), mode: "client".into() },
            bandwidth_inbound: BandwidthLimits { average: Some(100), ..Default::default() },
            ..Default::default()
        };
        assert!(validate(&nic).is_err());
    }

    #[test]
    fn apply_add_injects_into_devices() {
        let xml = "<domain type='kvm'>\n  <name>t</name>\n  <devices>\n    <emulator>/usr/bin/qemu</emulator>\n  </devices>\n</domain>\n";
        let nic = NicConfig {
            source: NicSource::Network { name: "default".into() },
            model: Some("virtio".into()),
            mac: Some("52:54:00:aa:bb:cc".into()),
            ..Default::default()
        };
        let new_xml = apply_nic_add(xml, &nic).unwrap();
        assert!(new_xml.contains("<interface type='network'>"));
        assert!(new_xml.contains("<emulator>"));
        let parsed = parse_nics(&new_xml).unwrap();
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn apply_remove_by_mac_preserves_others() {
        let xml = domain_with("    <interface type='network'><mac address='52:54:00:00:00:01'/><source network='default'/><model type='virtio'/></interface>\n    <interface type='bridge'><mac address='52:54:00:00:00:02'/><source bridge='br0'/><model type='e1000'/></interface>\n");
        let new_xml = apply_nic_remove_by_mac(&xml, "52:54:00:00:00:01").unwrap();
        let nics = parse_nics(&new_xml).unwrap();
        assert_eq!(nics.len(), 1);
        assert_eq!(nics[0].mac.as_deref(), Some("52:54:00:00:00:02"));
    }

    #[test]
    fn apply_remove_by_target_dev() {
        let xml = domain_with("    <interface type='bridge'><mac address='52:54:00:00:00:02'/><source bridge='br0'/><target dev='vnet9'/><model type='virtio'/></interface>\n");
        let new_xml = apply_nic_remove_by_mac(&xml, "vnet9").unwrap();
        assert!(parse_nics(&new_xml).unwrap().is_empty());
    }

    #[test]
    fn apply_update_replaces_in_place() {
        let xml = domain_with("    <interface type='network'><mac address='52:54:00:00:00:01'/><source network='default'/><model type='virtio'/></interface>\n");
        let new_nic = NicConfig {
            source: NicSource::Bridge { name: "br1".into() },
            model: Some("e1000e".into()),
            mac: Some("52:54:00:00:00:01".into()),
            link_state: Some("down".into()),
            ..Default::default()
        };
        let new_xml = apply_nic_update(&xml, "52:54:00:00:00:01", &new_nic).unwrap();
        let nics = parse_nics(&new_xml).unwrap();
        assert_eq!(nics.len(), 1);
        assert_eq!(nics[0].source, NicSource::Bridge { name: "br1".into() });
        assert_eq!(nics[0].link_state.as_deref(), Some("down"));
    }

    #[test]
    fn apply_add_wraps_devices_when_missing() {
        let xml = "<domain type='kvm'><name>t</name></domain>";
        let nic = NicConfig { source: NicSource::User, ..Default::default() };
        let new_xml = apply_nic_add(xml, &nic).unwrap();
        assert!(new_xml.contains("<devices>"));
        assert!(new_xml.contains("</devices>"));
    }
}
