//! Host device enumeration and hostdev XML parsing / building.
//!
//! The node-device XML from libvirt has a different shape per capability
//! (pci / usb_device / drm / etc). We only handle what the passthrough
//! UI needs: vendor/product IDs + names, PCI bus address, USB bus/device
//! number, and the currently-bound driver (so the UI can warn you when a
//! device is still held by the host kernel driver).

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

/// A PCI device on the hypervisor host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPciDevice {
    /// libvirt name like `pci_0000_01_00_0`.
    pub name: String,
    pub domain: u16,
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    /// 4-hex-digit vendor ID (e.g. 0x8086 for Intel).
    pub vendor_id: u16,
    pub vendor_name: String,
    pub product_id: u16,
    pub product_name: String,
    /// Name of the kernel driver currently bound, if any. `vfio-pci` =
    /// ready for passthrough; anything else (e.g. `nouveau`) means the
    /// host is using it and passthrough will need a driver unbind first.
    pub driver: Option<String>,
    /// IOMMU group number. Passthrough only works cleanly when the
    /// whole group is detached together.
    pub iommu_group: Option<u32>,
    /// PCI class code (`0x030000` = VGA, `0x020000` = ethernet, etc).
    pub class_code: Option<u32>,
}

/// A USB device on the hypervisor host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostUsbDevice {
    /// libvirt name like `usb_1_17`.
    pub name: String,
    pub bus: u8,
    pub device: u8,
    pub vendor_id: u16,
    pub vendor_name: String,
    pub product_id: u16,
    pub product_name: String,
    pub driver: Option<String>,
}

/// Domain-side hostdev entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostDevice {
    Pci {
        domain: u16,
        bus: u8,
        slot: u8,
        function: u8,
        /// `managed='yes'` in libvirt = libvirt handles the driver
        /// detach/reattach dance. Almost always what you want.
        managed: bool,
    },
    /// USB by bus/device address. Simple, but the address changes when
    /// the user unplugs and replugs the device.
    UsbAddress {
        bus: u8,
        device: u8,
        managed: bool,
    },
    /// USB by vendor+product IDs. Portable across replugs; matches
    /// whichever connected device has those IDs.
    UsbVendor {
        vendor_id: u16,
        product_id: u16,
        managed: bool,
    },
}

// ──────────────────────────────────────────────────────────────────────
// Node device XML -> host device struct
// ──────────────────────────────────────────────────────────────────────

/// Parse a libvirt `nodedev-dumpxml` output for a PCI device.
pub fn parse_pci_node_device(xml: &str) -> Result<HostPciDevice, VirtManagerError> {
    let mut r = mk_reader(xml);
    let mut name = String::new();
    let mut driver: Option<String> = None;
    let mut iommu_group: Option<u32> = None;
    let mut class_code: Option<u32> = None;
    let mut domain = 0u16;
    let mut bus = 0u8;
    let mut slot = 0u8;
    let mut function = 0u8;
    let mut vendor_id = 0u16;
    let mut vendor_name = String::new();
    let mut product_id = 0u16;
    let mut product_name = String::new();

    let mut path: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    let mut current_text_target: Option<TextTarget> = None;
    // For element text like <domain>0</domain> — pending_attrs holds attrs
    // that came in on the start tag (e.g. vendor id) while we wait for text.
    let mut pending_vendor_id: Option<u16> = None;
    let mut pending_product_id: Option<u16> = None;

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(xml_err(e, r.buffer_position())),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let attrs = attrs(&e);
                match (path.last().map(String::as_str), n.as_str()) {
                    (Some("device"), "name") => current_text_target = Some(TextTarget::Name),
                    (Some("driver"), "name") => current_text_target = Some(TextTarget::Driver),
                    (Some("capability"), "domain") => current_text_target = Some(TextTarget::Domain),
                    (Some("capability"), "bus") => current_text_target = Some(TextTarget::Bus),
                    (Some("capability"), "slot") => current_text_target = Some(TextTarget::Slot),
                    (Some("capability"), "function") => current_text_target = Some(TextTarget::Function),
                    (Some("capability"), "class") => current_text_target = Some(TextTarget::Class),
                    (Some("capability"), "vendor") => {
                        pending_vendor_id = get_attr(&attrs, "id").and_then(parse_hex_u16);
                        current_text_target = Some(TextTarget::VendorName);
                    }
                    (Some("capability"), "product") => {
                        pending_product_id = get_attr(&attrs, "id").and_then(parse_hex_u16);
                        current_text_target = Some(TextTarget::ProductName);
                    }
                    (Some("iommuGroup"), _) => {}
                    (Some("capability"), "iommuGroup") => {
                        iommu_group = get_attr(&attrs, "number").and_then(|s| s.parse().ok());
                    }
                    _ => {}
                }
                path.push(n);
            }
            Ok(Event::End(_)) => {
                path.pop();
                current_text_target = None;
            }
            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if n == "iommuGroup" && path.last().map(String::as_str) == Some("capability") {
                    iommu_group = get_attr(&a, "number").and_then(|s| s.parse().ok());
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                if let Some(target) = current_text_target {
                    match target {
                        TextTarget::Name => name = text,
                        TextTarget::Driver => driver = Some(text),
                        TextTarget::Domain => domain = parse_maybe_hex_u16(&text).unwrap_or(0),
                        TextTarget::Bus => bus = parse_maybe_hex_u8(&text).unwrap_or(0),
                        TextTarget::Slot => slot = parse_maybe_hex_u8(&text).unwrap_or(0),
                        TextTarget::Function => function = parse_maybe_hex_u8(&text).unwrap_or(0),
                        TextTarget::Class => class_code = parse_hex_u32(&text),
                        TextTarget::VendorName => {
                            vendor_name = text;
                            if let Some(id) = pending_vendor_id.take() { vendor_id = id; }
                        }
                        TextTarget::ProductName => {
                            product_name = text;
                            if let Some(id) = pending_product_id.take() { product_id = id; }
                        }
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(HostPciDevice {
        name,
        domain, bus, slot, function,
        vendor_id, vendor_name, product_id, product_name,
        driver, iommu_group, class_code,
    })
}

pub fn parse_usb_node_device(xml: &str) -> Result<HostUsbDevice, VirtManagerError> {
    let mut r = mk_reader(xml);
    let mut name = String::new();
    let mut driver: Option<String> = None;
    let mut bus = 0u8;
    let mut device = 0u8;
    let mut vendor_id = 0u16;
    let mut vendor_name = String::new();
    let mut product_id = 0u16;
    let mut product_name = String::new();

    let mut path: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    let mut target: Option<TextTarget> = None;
    let mut pending_vendor: Option<u16> = None;
    let mut pending_product: Option<u16> = None;

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(xml_err(e, r.buffer_position())),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                match (path.last().map(String::as_str), n.as_str()) {
                    (Some("device"), "name") => target = Some(TextTarget::Name),
                    (Some("driver"), "name") => target = Some(TextTarget::Driver),
                    (Some("capability"), "bus") => target = Some(TextTarget::Bus),
                    (Some("capability"), "device") => target = Some(TextTarget::Slot), // reuse slot slot
                    (Some("capability"), "vendor") => {
                        pending_vendor = get_attr(&a, "id").and_then(parse_hex_u16);
                        target = Some(TextTarget::VendorName);
                    }
                    (Some("capability"), "product") => {
                        pending_product = get_attr(&a, "id").and_then(parse_hex_u16);
                        target = Some(TextTarget::ProductName);
                    }
                    _ => {}
                }
                path.push(n);
            }
            Ok(Event::End(_)) => { path.pop(); target = None; }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                if let Some(tgt) = target {
                    match tgt {
                        TextTarget::Name => name = text,
                        TextTarget::Driver => driver = Some(text),
                        TextTarget::Bus => bus = parse_maybe_hex_u8(&text).unwrap_or(0),
                        TextTarget::Slot => device = parse_maybe_hex_u8(&text).unwrap_or(0),
                        TextTarget::VendorName => {
                            vendor_name = text;
                            if let Some(v) = pending_vendor.take() { vendor_id = v; }
                        }
                        TextTarget::ProductName => {
                            product_name = text;
                            if let Some(p) = pending_product.take() { product_id = p; }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(HostUsbDevice {
        name, bus, device,
        vendor_id, vendor_name, product_id, product_name,
        driver,
    })
}

/// Parse all `<hostdev>` entries from a domain XML, returning PCI + USB
/// passthrough assignments.
pub fn parse_hostdevs(xml: &str) -> Result<Vec<HostDevice>, VirtManagerError> {
    let mut r = mk_reader(xml);
    let mut buf = Vec::new();

    // We track the depth stack from Start/End only — Empty events do not
    // mutate the stack (they model `<foo/>` which opens+closes in one token).
    let mut path: Vec<String> = Vec::new();

    let mut out: Vec<HostDevice> = Vec::new();

    // Per-hostdev accumulators
    let mut in_hostdev = false;
    let mut hd_type = String::new();
    let mut hd_managed = true;
    let mut pci_domain: Option<u16> = None;
    let mut pci_bus: Option<u8> = None;
    let mut pci_slot: Option<u8> = None;
    let mut pci_func: Option<u8> = None;
    let mut usb_bus: Option<u8> = None;
    let mut usb_device: Option<u8> = None;
    let mut usb_vendor: Option<u16> = None;
    let mut usb_product: Option<u16> = None;

    // Handle a <source>-child element (the only place hostdev data lives).
    // Called from both Start and Empty branches.
    let mut handle_source_child = |name: &str,
                                   a: &[(String, String)],
                                   hd_type: &str,
                                   pci_domain: &mut Option<u16>,
                                   pci_bus: &mut Option<u8>,
                                   pci_slot: &mut Option<u8>,
                                   pci_func: &mut Option<u8>,
                                   usb_bus: &mut Option<u8>,
                                   usb_device: &mut Option<u8>,
                                   usb_vendor: &mut Option<u16>,
                                   usb_product: &mut Option<u16>| {
        match (hd_type, name) {
            ("pci", "address") => {
                *pci_domain = get_attr(a, "domain").and_then(parse_hex_u16);
                *pci_bus    = get_attr(a, "bus").and_then(parse_hex_u8);
                *pci_slot   = get_attr(a, "slot").and_then(parse_hex_u8);
                *pci_func   = get_attr(a, "function").and_then(parse_hex_u8);
            }
            ("usb", "address") => {
                *usb_bus    = get_attr(a, "bus").and_then(|s| s.parse().ok());
                *usb_device = get_attr(a, "device").and_then(|s| s.parse().ok());
            }
            ("usb", "vendor") => {
                *usb_vendor = get_attr(a, "id").and_then(parse_hex_u16);
            }
            ("usb", "product") => {
                *usb_product = get_attr(a, "id").and_then(parse_hex_u16);
            }
            _ => {}
        }
    };

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(xml_err(e, r.buffer_position())),
            Ok(Event::Eof) => break,

            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if n == "hostdev" {
                    in_hostdev = true;
                    hd_type = get_attr(&a, "type").unwrap_or_default();
                    hd_managed = get_attr(&a, "managed").as_deref() != Some("no");
                }
                if in_hostdev && path.last().map(String::as_str) == Some("source") {
                    handle_source_child(&n, &a, &hd_type,
                        &mut pci_domain, &mut pci_bus, &mut pci_slot, &mut pci_func,
                        &mut usb_bus, &mut usb_device, &mut usb_vendor, &mut usb_product);
                }
                path.push(n);
            }

            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                if in_hostdev && path.last().map(String::as_str) == Some("source") {
                    handle_source_child(&n, &a, &hd_type,
                        &mut pci_domain, &mut pci_bus, &mut pci_slot, &mut pci_func,
                        &mut usb_bus, &mut usb_device, &mut usb_vendor, &mut usb_product);
                }
                // Self-closing — do NOT push.
            }

            Ok(Event::End(e)) => {
                let n = utf8_name_end(&e);
                if n == "hostdev" && in_hostdev {
                    match hd_type.as_str() {
                        "pci" => {
                            if let (Some(d), Some(b), Some(s), Some(f)) =
                                (pci_domain, pci_bus, pci_slot, pci_func)
                            {
                                out.push(HostDevice::Pci {
                                    domain: d, bus: b, slot: s, function: f,
                                    managed: hd_managed,
                                });
                            }
                        }
                        "usb" => {
                            if let (Some(v), Some(p)) = (usb_vendor, usb_product) {
                                out.push(HostDevice::UsbVendor {
                                    vendor_id: v, product_id: p, managed: hd_managed,
                                });
                            } else if let (Some(b), Some(d)) = (usb_bus, usb_device) {
                                out.push(HostDevice::UsbAddress {
                                    bus: b, device: d, managed: hd_managed,
                                });
                            }
                        }
                        _ => {}
                    }
                    in_hostdev = false;
                    hd_type.clear();
                    hd_managed = true;
                    pci_domain = None; pci_bus = None; pci_slot = None; pci_func = None;
                    usb_bus = None; usb_device = None; usb_vendor = None; usb_product = None;
                }
                path.pop();
            }

            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

// quick_xml's Empty doesn't auto-pop our `path`. In practice the elements
// we care about (address / vendor / product) are self-closing and emitted
// as Event::Empty. Since we push on Start|Empty, we need to pop after an
// Empty too. Fix: use a separate helper path stack only for Start events,
// and query the *Start* depth inside Empty matching. Simpler: re-do the
// parse_hostdevs match to only push on Start, and inline-handle Empty
// without touching path.

// ── helpers ───────────────────────────────────────────────────────────

fn mk_reader(xml: &str) -> Reader<&[u8]> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    r
}

fn xml_err(e: quick_xml::Error, pos: u64) -> VirtManagerError {
    VirtManagerError::XmlParsingFailed { reason: format!("at {pos}: {e}") }
}

fn utf8_name(e: &quick_xml::events::BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn utf8_name_end(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn attrs(e: &quick_xml::events::BytesStart) -> Vec<(String, String)> {
    e.attributes().filter_map(|a| a.ok()).map(|a| (
        String::from_utf8_lossy(a.key.as_ref()).to_string(),
        a.unescape_value().unwrap_or_default().to_string(),
    )).collect()
}

fn get_attr(attrs: &[(String, String)], key: &str) -> Option<String> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn parse_hex_u16(s: String) -> Option<u16> {
    let s = s.trim().trim_start_matches("0x");
    u16::from_str_radix(s, 16).ok()
}

fn parse_hex_u8(s: String) -> Option<u8> {
    let s = s.trim().trim_start_matches("0x");
    u8::from_str_radix(s, 16).ok()
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches("0x");
    u32::from_str_radix(s, 16).ok()
}

fn parse_maybe_hex_u16(s: &str) -> Option<u16> {
    let t = s.trim();
    if let Some(h) = t.strip_prefix("0x") {
        u16::from_str_radix(h, 16).ok()
    } else {
        t.parse().ok()
    }
}

fn parse_maybe_hex_u8(s: &str) -> Option<u8> {
    let t = s.trim();
    if let Some(h) = t.strip_prefix("0x") {
        u8::from_str_radix(h, 16).ok()
    } else {
        t.parse().ok()
    }
}

#[derive(Copy, Clone)]
enum TextTarget {
    Name, Driver, Domain, Bus, Slot, Function, Class,
    VendorName, ProductName,
}

// ── Builders ──────────────────────────────────────────────────────────

/// Build a `<hostdev>` XML fragment for a single host device assignment.
pub fn build_hostdev_xml(dev: &HostDevice) -> String {
    match dev {
        HostDevice::Pci { domain, bus, slot, function, managed } => {
            format!(
                "<hostdev mode='subsystem' type='pci' managed='{}'>\n  <source>\n    <address domain='0x{:04x}' bus='0x{:02x}' slot='0x{:02x}' function='0x{:x}'/>\n  </source>\n</hostdev>\n",
                if *managed { "yes" } else { "no" },
                domain, bus, slot, function,
            )
        }
        HostDevice::UsbAddress { bus, device, managed } => {
            format!(
                "<hostdev mode='subsystem' type='usb' managed='{}'>\n  <source>\n    <address bus='{}' device='{}'/>\n  </source>\n</hostdev>\n",
                if *managed { "yes" } else { "no" },
                bus, device,
            )
        }
        HostDevice::UsbVendor { vendor_id, product_id, managed } => {
            format!(
                "<hostdev mode='subsystem' type='usb' managed='{}'>\n  <source>\n    <vendor id='0x{:04x}'/>\n    <product id='0x{:04x}'/>\n  </source>\n</hostdev>\n",
                if *managed { "yes" } else { "no" },
                vendor_id, product_id,
            )
        }
    }
}

/// Suppress unused — silence until we inline host-device names into XML comments.
#[allow(dead_code)]
pub fn escape_label(s: &str) -> String { escape_xml(s) }

#[cfg(test)]
mod tests {
    use super::*;

    const PCI_XML: &str = r#"<device>
  <name>pci_0000_00_1f_3</name>
  <path>/sys/devices/pci0000:00/0000:00:1f.3</path>
  <parent>computer</parent>
  <driver>
    <name>snd_hda_intel</name>
  </driver>
  <capability type='pci'>
    <class>0x040300</class>
    <domain>0</domain>
    <bus>0</bus>
    <slot>31</slot>
    <function>3</function>
    <product id='0x4dc8'>Jasper Lake HD Audio</product>
    <vendor id='0x8086'>Intel Corporation</vendor>
    <iommuGroup number='14'/>
  </capability>
</device>
"#;

    const USB_XML: &str = r#"<device>
  <name>usb_1_7</name>
  <path>/sys/devices/pci0000:00/0000:00:14.0/usb1/1-7</path>
  <devnode type='dev'>/dev/bus/usb/001/017</devnode>
  <parent>usb_usb1</parent>
  <driver>
    <name>usb</name>
  </driver>
  <capability type='usb_device'>
    <bus>1</bus>
    <device>17</device>
    <product id='0x7523'>CH340 serial converter</product>
    <vendor id='0x1a86'>QinHeng Electronics</vendor>
  </capability>
</device>
"#;

    #[test]
    fn parses_pci_node_device() {
        let d = parse_pci_node_device(PCI_XML).unwrap();
        assert_eq!(d.name, "pci_0000_00_1f_3");
        assert_eq!(d.domain, 0);
        assert_eq!(d.bus, 0);
        assert_eq!(d.slot, 31);
        assert_eq!(d.function, 3);
        assert_eq!(d.vendor_id, 0x8086);
        assert_eq!(d.vendor_name, "Intel Corporation");
        assert_eq!(d.product_id, 0x4dc8);
        assert_eq!(d.product_name, "Jasper Lake HD Audio");
        assert_eq!(d.driver.as_deref(), Some("snd_hda_intel"));
        assert_eq!(d.iommu_group, Some(14));
        assert_eq!(d.class_code, Some(0x040300));
    }

    #[test]
    fn parses_usb_node_device() {
        let d = parse_usb_node_device(USB_XML).unwrap();
        assert_eq!(d.name, "usb_1_7");
        assert_eq!(d.bus, 1);
        assert_eq!(d.device, 17);
        assert_eq!(d.vendor_id, 0x1a86);
        assert_eq!(d.product_id, 0x7523);
        assert_eq!(d.vendor_name, "QinHeng Electronics");
    }

    #[test]
    fn builds_pci_hostdev_xml() {
        let xml = build_hostdev_xml(&HostDevice::Pci {
            domain: 0, bus: 1, slot: 0, function: 0, managed: true,
        });
        assert!(xml.contains("<hostdev mode='subsystem' type='pci' managed='yes'>"));
        assert!(xml.contains("domain='0x0000' bus='0x01' slot='0x00' function='0x0'"));
    }

    #[test]
    fn builds_usb_address_hostdev_xml() {
        let xml = build_hostdev_xml(&HostDevice::UsbAddress {
            bus: 1, device: 17, managed: true,
        });
        assert!(xml.contains("type='usb'"));
        assert!(xml.contains("<address bus='1' device='17'/>"));
    }

    #[test]
    fn builds_usb_vendor_hostdev_xml() {
        let xml = build_hostdev_xml(&HostDevice::UsbVendor {
            vendor_id: 0x1a86, product_id: 0x7523, managed: true,
        });
        assert!(xml.contains("<vendor id='0x1a86'/>"));
        assert!(xml.contains("<product id='0x7523'/>"));
    }

    #[test]
    fn parses_pci_hostdev_in_domain() {
        let xml = r#"<domain><devices>
            <hostdev mode='subsystem' type='pci' managed='yes'>
              <source>
                <address domain='0x0000' bus='0x01' slot='0x00' function='0x0'/>
              </source>
            </hostdev>
        </devices></domain>"#;
        let devs = parse_hostdevs(xml).unwrap();
        assert_eq!(devs.len(), 1);
        match &devs[0] {
            HostDevice::Pci { domain, bus, slot, function, managed } => {
                assert_eq!(*domain, 0);
                assert_eq!(*bus, 1);
                assert_eq!(*slot, 0);
                assert_eq!(*function, 0);
                assert!(*managed);
            }
            _ => panic!("expected PCI"),
        }
    }

    #[test]
    fn parses_usb_address_hostdev_in_domain() {
        let xml = r#"<domain><devices>
            <hostdev mode='subsystem' type='usb' managed='yes'>
              <source>
                <address bus='1' device='17'/>
              </source>
            </hostdev>
        </devices></domain>"#;
        let devs = parse_hostdevs(xml).unwrap();
        assert_eq!(devs.len(), 1);
        match &devs[0] {
            HostDevice::UsbAddress { bus, device, .. } => {
                assert_eq!(*bus, 1);
                assert_eq!(*device, 17);
            }
            _ => panic!("expected USB address"),
        }
    }

    #[test]
    fn parses_usb_vendor_hostdev_in_domain() {
        let xml = r#"<domain><devices>
            <hostdev mode='subsystem' type='usb' managed='yes'>
              <source>
                <vendor id='0x1a86'/>
                <product id='0x7523'/>
              </source>
            </hostdev>
        </devices></domain>"#;
        let devs = parse_hostdevs(xml).unwrap();
        match &devs[0] {
            HostDevice::UsbVendor { vendor_id, product_id, .. } => {
                assert_eq!(*vendor_id, 0x1a86);
                assert_eq!(*product_id, 0x7523);
            }
            _ => panic!("expected USB vendor"),
        }
    }

    #[test]
    fn parses_multiple_hostdevs_preserves_order() {
        let xml = r#"<domain><devices>
            <hostdev mode='subsystem' type='pci' managed='yes'>
              <source><address domain='0x0000' bus='0x01' slot='0x00' function='0x0'/></source>
            </hostdev>
            <hostdev mode='subsystem' type='usb' managed='no'>
              <source><address bus='2' device='3'/></source>
            </hostdev>
        </devices></domain>"#;
        let devs = parse_hostdevs(xml).unwrap();
        assert_eq!(devs.len(), 2);
        matches!(devs[0], HostDevice::Pci { .. });
        matches!(devs[1], HostDevice::UsbAddress { managed: false, .. });
    }

    #[test]
    fn empty_xml_returns_empty_hostdevs() {
        let xml = r#"<domain><devices></devices></domain>"#;
        assert!(parse_hostdevs(xml).unwrap().is_empty());
    }

    #[test]
    fn roundtrip_pci() {
        let orig = HostDevice::Pci { domain: 0, bus: 1, slot: 2, function: 3, managed: true };
        let xml = format!("<domain><devices>{}</devices></domain>", build_hostdev_xml(&orig));
        let parsed = parse_hostdevs(&xml).unwrap();
        assert_eq!(parsed.len(), 1);
        if let HostDevice::Pci { domain, bus, slot, function, managed } = parsed[0] {
            assert_eq!((domain, bus, slot, function, managed), (0, 1, 2, 3, true));
        } else { panic!() }
    }

    #[test]
    fn roundtrip_usb_vendor() {
        let orig = HostDevice::UsbVendor { vendor_id: 0x1a86, product_id: 0x7523, managed: true };
        let xml = format!("<domain><devices>{}</devices></domain>", build_hostdev_xml(&orig));
        let parsed = parse_hostdevs(&xml).unwrap();
        if let HostDevice::UsbVendor { vendor_id, product_id, .. } = parsed[0] {
            assert_eq!((vendor_id, product_id), (0x1a86, 0x7523));
        } else { panic!() }
    }
}
