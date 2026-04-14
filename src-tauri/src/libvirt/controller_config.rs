//! Controller (bus) editor: USB / SCSI / virtio-serial / IDE / SATA / PCI / CCID / FDC.
//!
//! Controllers are the buses that attach storage and device nodes. They
//! matter for:
//!   - USB family choice (xhci for modern guests, ehci+uhci for legacy)
//!   - SCSI controller model (virtio-scsi enables discard/trim, multi-queue,
//!     iothread assignment)
//!   - virtio-serial port sizing (default 1; need more for multiple channels)
//!
//! PCI controllers form the topology and are *managed by libvirt* — our
//! editor parses them for display but we do NOT build/patch PCI controllers;
//! changing PCIe layout is a footgun and libvirt auto-assigns correctly.
//!
//! Follows the "mutate XML in place" pattern from Round A so unknown
//! sibling elements round-trip exactly.

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

// ────────── types ──────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ControllerType {
    Usb,
    Scsi,
    #[serde(rename = "virtio-serial")]
    VirtioSerial,
    Ide,
    Sata,
    Pci,
    Ccid,
    Fdc,
}

impl ControllerType {
    pub fn as_str(self) -> &'static str {
        match self {
            ControllerType::Usb => "usb",
            ControllerType::Scsi => "scsi",
            ControllerType::VirtioSerial => "virtio-serial",
            ControllerType::Ide => "ide",
            ControllerType::Sata => "sata",
            ControllerType::Pci => "pci",
            ControllerType::Ccid => "ccid",
            ControllerType::Fdc => "fdc",
        }
    }

    pub fn from_xml_str(s: &str) -> Option<Self> {
        Some(match s {
            "usb" => ControllerType::Usb,
            "scsi" => ControllerType::Scsi,
            "virtio-serial" => ControllerType::VirtioSerial,
            "ide" => ControllerType::Ide,
            "sata" => ControllerType::Sata,
            "pci" => ControllerType::Pci,
            "ccid" => ControllerType::Ccid,
            "fdc" => ControllerType::Fdc,
            _ => return None,
        })
    }
}

/// One `<controller>` entry.
///
/// `index` is the identity within a type — libvirt numbers controllers
/// starting at 0 per type. PCI-specific read-only fields come through
/// but are never written on build.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ControllerConfig {
    #[serde(rename = "type")]
    pub controller_type: String,
    pub index: u32,
    pub model: Option<String>,
    pub ports: Option<u32>,
    pub vectors: Option<u32>,
    pub queues: Option<u32>,
    pub iothread: Option<u32>,
    pub ioeventfd: Option<bool>,
    pub event_idx: Option<bool>,
    // PCI read-only passthroughs (populated on parse, ignored on build).
    pub chassis: Option<u32>,
    pub slot: Option<u32>,
    pub bus: Option<u32>,
    pub function: Option<u32>,
    /// `<target chassis='N' port='0xNN'/>` port attr for PCIe-root-port.
    pub target_port: Option<String>,
}

impl ControllerConfig {
    pub fn kind(&self) -> Option<ControllerType> {
        ControllerType::from_xml_str(&self.controller_type)
    }
}

// ────────── limits / validation ──────────

/// USB xhci (qemu-xhci / nec-xhci) ports are hard-capped at 15 by QEMU.
pub const USB_XHCI_MAX_PORTS: u32 = 15;

/// virtio-serial has a 32-port register; index 0 is reserved so the
/// user-visible max is 31.
pub const VIRTIO_SERIAL_MAX_PORTS: u32 = 31;

fn invalid(field: &str, reason: &str) -> VirtManagerError {
    VirtManagerError::OperationFailed {
        operation: format!("validate controller.{field}"),
        reason: reason.to_string(),
    }
}

/// Validate a ControllerConfig against its type's constraints.
pub fn validate(cfg: &ControllerConfig) -> Result<(), VirtManagerError> {
    let kind = cfg.kind().ok_or_else(|| {
        invalid("type", &format!("unknown controller type '{}'", cfg.controller_type))
    })?;

    match kind {
        ControllerType::Usb => {
            if let Some(p) = cfg.ports {
                if p > USB_XHCI_MAX_PORTS {
                    return Err(invalid(
                        "ports",
                        &format!("USB xhci max ports is {USB_XHCI_MAX_PORTS}, got {p}"),
                    ));
                }
            }
            if cfg.queues.is_some() || cfg.iothread.is_some() {
                return Err(invalid(
                    "queues",
                    "queues/iothread are only valid on virtio-scsi",
                ));
            }
        }
        ControllerType::VirtioSerial => {
            if let Some(p) = cfg.ports {
                if p > VIRTIO_SERIAL_MAX_PORTS {
                    return Err(invalid(
                        "ports",
                        &format!("virtio-serial max ports is {VIRTIO_SERIAL_MAX_PORTS}, got {p}"),
                    ));
                }
            }
            if cfg.queues.is_some() || cfg.iothread.is_some() {
                return Err(invalid(
                    "queues",
                    "queues/iothread are only valid on virtio-scsi",
                ));
            }
        }
        ControllerType::Scsi => {
            let is_virtio_scsi = cfg
                .model
                .as_deref()
                .map(|m| {
                    m.starts_with("virtio-scsi")
                        || m.starts_with("virtio-transitional")
                        || m.starts_with("virtio-non-transitional")
                })
                .unwrap_or(false);
            if (cfg.queues.is_some() || cfg.iothread.is_some()) && !is_virtio_scsi {
                return Err(invalid(
                    "queues",
                    &format!(
                        "queues/iothread only valid on virtio-scsi, got model={:?}",
                        cfg.model
                    ),
                ));
            }
        }
        ControllerType::Pci => {
            return Err(invalid(
                "type",
                "PCI controllers are managed by libvirt and cannot be edited here",
            ));
        }
        ControllerType::Ide | ControllerType::Sata | ControllerType::Ccid | ControllerType::Fdc => {
            if cfg.queues.is_some() || cfg.iothread.is_some() {
                return Err(invalid(
                    "queues",
                    "queues/iothread are only valid on virtio-scsi",
                ));
            }
        }
    }
    Ok(())
}

// ────────── parse ──────────

/// Parse all `<controller>` entries out of a domain XML.
pub fn parse_controllers(xml: &str) -> Result<Vec<ControllerConfig>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out: Vec<ControllerConfig> = Vec::new();
    let mut buf = Vec::new();
    let mut current: Option<ControllerConfig> = None;

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: format!("at {}: {}", r.buffer_position(), e),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = name(&e);
                if n == "controller" {
                    current = Some(parse_controller_attrs(&e));
                } else if let Some(ref mut c) = current {
                    apply_child(c, &n, &e);
                }
            }
            Ok(Event::Empty(e)) => {
                let n = name(&e);
                if n == "controller" {
                    out.push(parse_controller_attrs(&e));
                } else if let Some(ref mut c) = current {
                    apply_child(c, &n, &e);
                }
            }
            Ok(Event::End(e)) => {
                if end_name(&e) == "controller" {
                    if let Some(c) = current.take() {
                        out.push(c);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

fn apply_child(c: &mut ControllerConfig, n: &str, e: &BytesStart) {
    match n {
        "driver" => {
            if let Some(v) = attr_val(e, "queues") {
                c.queues = v.parse().ok();
            }
            if let Some(v) = attr_val(e, "iothread") {
                c.iothread = v.parse().ok();
            }
            if let Some(v) = attr_val(e, "ioeventfd") {
                c.ioeventfd = Some(v == "on");
            }
            if let Some(v) = attr_val(e, "event_idx") {
                c.event_idx = Some(v == "on");
            }
        }
        "model" => {
            // `<model name='...'/>` inside a controller refines the model.
            // The outer `model=` attribute usually wins — only fill if absent.
            if c.model.is_none() {
                if let Some(v) = attr_val(e, "name") {
                    c.model = Some(v);
                }
            }
        }
        "target" => {
            if let Some(v) = attr_val(e, "chassis") {
                c.chassis = v.parse().ok();
            }
            if let Some(v) = attr_val(e, "port") {
                c.target_port = Some(v);
            }
        }
        "address" => {
            if let Some(v) = attr_val(e, "bus") {
                c.bus = parse_hex_or_dec(&v);
            }
            if let Some(v) = attr_val(e, "slot") {
                c.slot = parse_hex_or_dec(&v);
            }
            if let Some(v) = attr_val(e, "function") {
                c.function = parse_hex_or_dec(&v);
            }
        }
        _ => {}
    }
}

fn parse_controller_attrs(e: &BytesStart) -> ControllerConfig {
    let mut c = ControllerConfig::default();
    if let Some(v) = attr_val(e, "type") {
        c.controller_type = v;
    }
    if let Some(v) = attr_val(e, "index") {
        c.index = v.parse().unwrap_or(0);
    }
    if let Some(v) = attr_val(e, "model") {
        c.model = Some(v);
    }
    if let Some(v) = attr_val(e, "ports") {
        c.ports = v.parse().ok();
    }
    if let Some(v) = attr_val(e, "vectors") {
        c.vectors = v.parse().ok();
    }
    c
}

fn name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn end_name(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn attr_val(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == key.as_bytes())
        .map(|a| a.unescape_value().unwrap_or_default().to_string())
}

fn parse_hex_or_dec(s: &str) -> Option<u32> {
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(rest, 16).ok()
    } else {
        s.parse().ok()
    }
}

// ────────── build ──────────

/// Serialize a ControllerConfig as `<controller>...</controller>` XML.
///
/// PCI controllers cannot be built (validate rejects them) — topology
/// belongs to libvirt.
pub fn build_controller_xml(cfg: &ControllerConfig) -> Result<String, VirtManagerError> {
    validate(cfg)?;
    let kind = cfg.kind().expect("validated above");

    let mut s = String::new();
    s.push_str(&format!(
        "<controller type='{}' index='{}'",
        escape_xml(&cfg.controller_type),
        cfg.index
    ));
    if let Some(ref m) = cfg.model {
        s.push_str(&format!(" model='{}'", escape_xml(m)));
    }
    if let Some(p) = cfg.ports {
        s.push_str(&format!(" ports='{p}'"));
    }
    if let Some(v) = cfg.vectors {
        s.push_str(&format!(" vectors='{v}'"));
    }

    let has_driver = matches!(kind, ControllerType::Scsi)
        && (cfg.queues.is_some()
            || cfg.iothread.is_some()
            || cfg.ioeventfd.is_some()
            || cfg.event_idx.is_some());

    if !has_driver {
        s.push_str("/>");
    } else {
        s.push('>');
        s.push_str("<driver");
        if let Some(q) = cfg.queues {
            s.push_str(&format!(" queues='{q}'"));
        }
        if let Some(i) = cfg.iothread {
            s.push_str(&format!(" iothread='{i}'"));
        }
        if let Some(v) = cfg.ioeventfd {
            s.push_str(&format!(" ioeventfd='{}'", if v { "on" } else { "off" }));
        }
        if let Some(v) = cfg.event_idx {
            s.push_str(&format!(" event_idx='{}'", if v { "on" } else { "off" }));
        }
        s.push_str("/>");
        s.push_str("</controller>");
    }
    Ok(s)
}

// ────────── apply_add / apply_remove / apply_update ──────────

/// Inject a new controller before `</devices>`.
pub fn apply_add_controller(xml: &str, cfg: &ControllerConfig) -> Result<String, VirtManagerError> {
    let frag = build_controller_xml(cfg)?;
    insert_before_devices_end(xml, &frag)
}

/// Remove the controller matching (type, index).
pub fn apply_remove_controller(
    xml: &str,
    ctype: &str,
    index: u32,
) -> Result<String, VirtManagerError> {
    let (start, end) = find_controller_span(xml, ctype, index)?;
    // Eat trailing whitespace before the fragment so we don't leave a
    // blank line behind.
    let leading_ws_start = xml[..start]
        .rfind(|c: char| !c.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(start);
    let mut out = String::with_capacity(xml.len());
    out.push_str(&xml[..leading_ws_start]);
    out.push('\n');
    out.push_str(&xml[end..]);
    Ok(out)
}

/// Replace the controller matching (type, index) with the rebuilt XML.
pub fn apply_update_controller(
    xml: &str,
    ctype: &str,
    index: u32,
    new_cfg: &ControllerConfig,
) -> Result<String, VirtManagerError> {
    let (start, end) = find_controller_span(xml, ctype, index)?;
    let frag = build_controller_xml(new_cfg)?;
    let mut out = String::with_capacity(xml.len() + frag.len());
    out.push_str(&xml[..start]);
    out.push_str(&frag);
    out.push_str(&xml[end..]);
    Ok(out)
}

fn insert_before_devices_end(xml: &str, frag: &str) -> Result<String, VirtManagerError> {
    let idx = xml.rfind("</devices>").ok_or_else(|| VirtManagerError::XmlParsingFailed {
        reason: "no </devices> found".into(),
    })?;
    let mut out = String::with_capacity(xml.len() + frag.len() + 8);
    out.push_str(&xml[..idx]);
    out.push_str("  ");
    out.push_str(frag);
    out.push('\n');
    out.push_str("  ");
    out.push_str(&xml[idx..]);
    Ok(out)
}

/// Find the byte span `[start, end)` of the matching `<controller>` element.
fn find_controller_span(
    xml: &str,
    ctype: &str,
    index: u32,
) -> Result<(usize, usize), VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();

    let mut candidate_start: Option<usize> = None;
    let mut matching = false;
    let mut depth = 0;

    loop {
        let pos_before = r.buffer_position() as usize;
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(VirtManagerError::XmlParsingFailed { reason: e.to_string() }),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if name(&e) == "controller" => {
                let t = attr_val(&e, "type").unwrap_or_default();
                let i: u32 = attr_val(&e, "index")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if t == ctype && i == index {
                    matching = true;
                    candidate_start = Some(pos_before);
                    depth = 1;
                } else {
                    matching = false;
                }
            }
            Ok(Event::Empty(e)) if name(&e) == "controller" => {
                let t = attr_val(&e, "type").unwrap_or_default();
                let i: u32 = attr_val(&e, "index")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if t == ctype && i == index {
                    let pos_after = r.buffer_position() as usize;
                    return Ok((pos_before, pos_after));
                }
            }
            Ok(Event::End(e)) if end_name(&e) == "controller" && matching => {
                depth -= 1;
                if depth == 0 {
                    let end = r.buffer_position() as usize;
                    if let Some(start) = candidate_start {
                        return Ok((start, end));
                    }
                }
            }
            Ok(Event::Start(_)) if matching => {
                depth += 1;
            }
            Ok(Event::End(_)) if matching => {
                depth -= 1;
            }
            _ => {}
        }
        buf.clear();
    }

    Err(VirtManagerError::OperationFailed {
        operation: "findController".into(),
        reason: format!("no controller with type={ctype} index={index}"),
    })
}

// ────────── tests ──────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<domain type='kvm'>
  <name>test</name>
  <devices>
    <controller type='usb' index='0' model='qemu-xhci' ports='15'>
      <alias name='usb'/>
      <address type='pci' domain='0x0000' bus='0x02' slot='0x00' function='0x0'/>
    </controller>
    <controller type='pci' index='0' model='pcie-root'/>
    <controller type='pci' index='1' model='pcie-root-port'>
      <model name='pcie-root-port'/>
      <target chassis='1' port='0x10'/>
      <address type='pci' domain='0x0000' bus='0x00' slot='0x02' function='0x0'/>
    </controller>
    <controller type='scsi' index='0' model='virtio-scsi'>
      <driver queues='4' iothread='1' ioeventfd='on' event_idx='on'/>
    </controller>
    <controller type='virtio-serial' index='0' ports='8'>
      <address type='pci' domain='0x0000' bus='0x03' slot='0x00' function='0x0'/>
    </controller>
    <controller type='sata' index='0'/>
  </devices>
</domain>
"#;

    #[test]
    fn parses_usb_controller() {
        let cs = parse_controllers(SAMPLE).unwrap();
        let usb = cs.iter().find(|c| c.controller_type == "usb" && c.index == 0).unwrap();
        assert_eq!(usb.model.as_deref(), Some("qemu-xhci"));
        assert_eq!(usb.ports, Some(15));
    }

    #[test]
    fn parses_scsi_controller_with_driver() {
        let cs = parse_controllers(SAMPLE).unwrap();
        let scsi = cs.iter().find(|c| c.controller_type == "scsi").unwrap();
        assert_eq!(scsi.model.as_deref(), Some("virtio-scsi"));
        assert_eq!(scsi.queues, Some(4));
        assert_eq!(scsi.iothread, Some(1));
        assert_eq!(scsi.ioeventfd, Some(true));
        assert_eq!(scsi.event_idx, Some(true));
    }

    #[test]
    fn parses_virtio_serial_ports() {
        let cs = parse_controllers(SAMPLE).unwrap();
        let vs = cs.iter().find(|c| c.controller_type == "virtio-serial").unwrap();
        assert_eq!(vs.ports, Some(8));
    }

    #[test]
    fn parses_pci_preserves_but_refuses_to_build() {
        let cs = parse_controllers(SAMPLE).unwrap();
        let pci_root = cs.iter().find(|c| c.controller_type == "pci" && c.index == 0).unwrap();
        assert_eq!(pci_root.model.as_deref(), Some("pcie-root"));
        let pcie_port = cs.iter().find(|c| c.controller_type == "pci" && c.index == 1).unwrap();
        assert_eq!(pcie_port.model.as_deref(), Some("pcie-root-port"));
        assert_eq!(pcie_port.chassis, Some(1));
        assert_eq!(pcie_port.target_port.as_deref(), Some("0x10"));
        assert_eq!(pcie_port.bus, Some(0));
        assert_eq!(pcie_port.slot, Some(2));
        assert_eq!(pcie_port.function, Some(0));

        let res = build_controller_xml(pci_root);
        assert!(res.is_err(), "PCI should refuse build, got {res:?}");
    }

    #[test]
    fn round_trip_usb() {
        let cs = parse_controllers(SAMPLE).unwrap();
        let usb = cs.iter().find(|c| c.controller_type == "usb").unwrap();
        let xml = build_controller_xml(usb).unwrap();
        assert!(xml.contains("type='usb'"));
        assert!(xml.contains("model='qemu-xhci'"));
        assert!(xml.contains("ports='15'"));
    }

    #[test]
    fn round_trip_scsi_emits_driver() {
        let cs = parse_controllers(SAMPLE).unwrap();
        let scsi = cs.iter().find(|c| c.controller_type == "scsi").unwrap();
        let xml = build_controller_xml(scsi).unwrap();
        assert!(xml.contains("<driver"));
        assert!(xml.contains("queues='4'"));
        assert!(xml.contains("iothread='1'"));
        assert!(xml.contains("ioeventfd='on'"));
        assert!(xml.contains("event_idx='on'"));
        assert!(xml.contains("</controller>"));
    }

    #[test]
    fn round_trip_virtio_serial() {
        let cs = parse_controllers(SAMPLE).unwrap();
        let vs = cs.iter().find(|c| c.controller_type == "virtio-serial").unwrap();
        let xml = build_controller_xml(vs).unwrap();
        let re_parsed = parse_controllers(&format!(
            "<domain><devices>{xml}</devices></domain>"
        )).unwrap();
        assert_eq!(re_parsed.len(), 1);
        assert_eq!(re_parsed[0].ports, Some(8));
        assert_eq!(re_parsed[0].controller_type, "virtio-serial");
    }

    #[test]
    fn validate_usb_xhci_ports_max_15() {
        let bad = ControllerConfig {
            controller_type: "usb".into(),
            index: 0,
            model: Some("qemu-xhci".into()),
            ports: Some(20),
            ..Default::default()
        };
        let err = validate(&bad).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("max ports"), "expected port-limit error, got {msg}");
    }

    #[test]
    fn validate_virtio_serial_ports_max_31() {
        let bad = ControllerConfig {
            controller_type: "virtio-serial".into(),
            index: 0,
            ports: Some(32),
            ..Default::default()
        };
        let err = validate(&bad).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("max ports"), "got {msg}");
    }

    #[test]
    fn validate_queues_only_on_virtio_scsi() {
        let bad = ControllerConfig {
            controller_type: "scsi".into(),
            index: 0,
            model: Some("lsilogic".into()),
            queues: Some(4),
            ..Default::default()
        };
        let err = validate(&bad).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("virtio-scsi"), "got {msg}");

        let good = ControllerConfig {
            controller_type: "scsi".into(),
            index: 0,
            model: Some("virtio-scsi".into()),
            queues: Some(4),
            iothread: Some(1),
            ..Default::default()
        };
        validate(&good).unwrap();
    }

    #[test]
    fn validate_rejects_pci_build() {
        let pci = ControllerConfig {
            controller_type: "pci".into(),
            index: 2,
            model: Some("pcie-root-port".into()),
            ..Default::default()
        };
        let err = validate(&pci).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("PCI") || msg.contains("pci"), "got {msg}");
    }

    #[test]
    fn multiple_controllers_preserve_index() {
        let cs = parse_controllers(SAMPLE).unwrap();
        let pci_indices: Vec<u32> = cs
            .iter()
            .filter(|c| c.controller_type == "pci")
            .map(|c| c.index)
            .collect();
        assert_eq!(pci_indices, vec![0, 1]);
    }

    #[test]
    fn apply_add_and_remove_round_trip() {
        let new = ControllerConfig {
            controller_type: "usb".into(),
            index: 1,
            model: Some("nec-xhci".into()),
            ports: Some(4),
            ..Default::default()
        };
        let added = apply_add_controller(SAMPLE, &new).unwrap();
        let cs = parse_controllers(&added).unwrap();
        assert!(cs.iter().any(|c| c.controller_type == "usb" && c.index == 1));

        let removed = apply_remove_controller(&added, "usb", 1).unwrap();
        let cs2 = parse_controllers(&removed).unwrap();
        assert!(!cs2.iter().any(|c| c.controller_type == "usb" && c.index == 1));
    }

    #[test]
    fn apply_update_swaps_usb_model() {
        let updated = ControllerConfig {
            controller_type: "usb".into(),
            index: 0,
            model: Some("nec-xhci".into()),
            ports: Some(8),
            ..Default::default()
        };
        let new_xml = apply_update_controller(SAMPLE, "usb", 0, &updated).unwrap();
        let cs = parse_controllers(&new_xml).unwrap();
        let usb = cs.iter().find(|c| c.controller_type == "usb" && c.index == 0).unwrap();
        assert_eq!(usb.model.as_deref(), Some("nec-xhci"));
        assert_eq!(usb.ports, Some(8));
    }

    #[test]
    fn apply_update_preserves_siblings() {
        let updated = ControllerConfig {
            controller_type: "usb".into(),
            index: 0,
            model: Some("nec-xhci".into()),
            ports: Some(8),
            ..Default::default()
        };
        let new_xml = apply_update_controller(SAMPLE, "usb", 0, &updated).unwrap();
        assert!(new_xml.contains("type='scsi'"));
        assert!(new_xml.contains("type='virtio-serial'"));
        assert!(new_xml.contains("type='sata'"));
        assert!(new_xml.contains("<name>test</name>"));
    }

    #[test]
    fn injection_safe_escapes_model() {
        let cfg = ControllerConfig {
            controller_type: "scsi".into(),
            index: 0,
            model: Some("virtio-scsi' oops='x".into()),
            ..Default::default()
        };
        let xml = build_controller_xml(&cfg).unwrap();
        assert!(!xml.contains("oops='x"),
            "unescaped single quote leaked: {xml}");
        assert!(xml.contains("&apos;"));
    }

    #[test]
    fn parse_missing_index_defaults_zero() {
        let xml = r#"<domain><devices><controller type='usb'/></devices></domain>"#;
        let cs = parse_controllers(xml).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].index, 0);
    }
}
