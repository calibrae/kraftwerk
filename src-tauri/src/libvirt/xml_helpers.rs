use regex::Regex;
use std::sync::LazyLock;

/// Escape a string for safe interpolation into XML attributes and text.
pub fn escape_xml(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(ch),
        }
    }
    output
}

static GRAPHICS_TYPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<graphics\s+type=['"]([\w]+)['""]"#).unwrap());

static SERIAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<(serial|console)\s+type=["']"#).unwrap());

/// Extract the graphics type (vnc/spice) from domain XML.
pub fn extract_graphics_type(xml: &str) -> Option<String> {
    GRAPHICS_TYPE_RE
        .captures(xml)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Check if domain XML contains a serial console.
pub fn has_serial_console(xml: &str) -> bool {
    SERIAL_RE.is_match(xml)
}


/// A parsed <interface> device entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceTarget {
    /// The host-side path libvirt uses for interface_stats lookups
    /// (`<target dev='vnetN'/>` when present, otherwise the MAC as a fallback).
    pub path: String,
    pub mac: String,
    pub model: String,
}

/// Extract disk target device names (`<target dev='vda'/>`) in order.
/// Used for per-disk block stats lookups.
pub fn extract_disk_targets(xml: &str) -> Vec<String> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        // Capture the `<disk>...<target dev='...'/>...</disk>` blocks, but only
        // for `device='disk'` (skip cdrom/floppy).
        regex::Regex::new(
            r#"(?s)<disk[^>]*device=['"]disk['"][^>]*>.*?<target\s+dev=['"]([^'"]+)['"]"#,
        )
        .unwrap()
    });
    RE.captures_iter(xml)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Extract interface target devices, MAC, and model in document order.
pub fn extract_interface_targets(xml: &str) -> Vec<InterfaceTarget> {
    static IFACE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?s)<interface\s[^>]*>(.*?)</interface>"#).unwrap()
    });
    static MAC_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"<mac\s+address=['"]([^'"]+)['"]"#).unwrap()
    });
    static TARGET_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"<target\s+dev=['"]([^'"]+)['"]"#).unwrap()
    });
    static MODEL_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"<model\s+type=['"]([^'"]+)['"]"#).unwrap()
    });

    IFACE_RE
        .captures_iter(xml)
        .map(|c| {
            let body = c.get(1).map(|m| m.as_str()).unwrap_or("");
            let mac = MAC_RE
                .captures(body)
                .and_then(|m| m.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let path = TARGET_RE
                .captures(body)
                .and_then(|m| m.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| mac.clone()); // fallback if target dev isn't assigned yet
            let model = MODEL_RE
                .captures(body)
                .and_then(|m| m.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            InterfaceTarget { path, mac, model }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_xml_special_chars() {
        assert_eq!(escape_xml(r#"a<b>c&d"e'f"#), "a&lt;b&gt;c&amp;d&quot;e&apos;f");
    }

    #[test]
    fn escape_xml_passthrough() {
        assert_eq!(escape_xml("hello world"), "hello world");
    }

    #[test]
    fn extract_graphics_vnc_double_quotes() {
        let xml = r#"<domain><devices><graphics type="vnc" port="-1"/></devices></domain>"#;
        assert_eq!(extract_graphics_type(xml), Some("vnc".into()));
    }

    #[test]
    fn extract_graphics_spice_single_quotes() {
        let xml = "<domain><devices><graphics type='spice' autoport='yes'/></devices></domain>";
        assert_eq!(extract_graphics_type(xml), Some("spice".into()));
    }

    #[test]
    fn extract_graphics_none() {
        let xml = r#"<domain><devices></devices></domain>"#;
        assert_eq!(extract_graphics_type(xml), None);
    }

    #[test]
    fn has_serial_console_double_quotes() {
        let xml = r#"<domain><devices><serial type="pty"/></devices></domain>"#;
        assert!(has_serial_console(xml));
    }

    #[test]
    fn has_serial_console_single_quotes() {
        let xml = "<domain><devices><console type='pty'/></devices></domain>";
        assert!(has_serial_console(xml));
    }

    #[test]
    fn has_serial_console_false() {
        let xml = r#"<domain><devices><graphics type="vnc"/></devices></domain>"#;
        assert!(!has_serial_console(xml));
    }

    #[test]
    fn extracts_disk_targets_skipping_cdrom() {
        let xml = r#"<domain><devices>
            <disk type='file' device='disk'><target dev='vda' bus='virtio'/></disk>
            <disk type='file' device='cdrom'><target dev='sda' bus='sata'/></disk>
            <disk type='file' device='disk'><target dev='vdb' bus='virtio'/></disk>
        </devices></domain>"#;
        let targets = extract_disk_targets(xml);
        assert_eq!(targets, vec!["vda", "vdb"]);
    }

    #[test]
    fn extracts_interface_targets_with_target_dev() {
        let xml = r#"<domain><devices>
            <interface type='network'>
                <mac address='52:54:00:aa:bb:cc'/>
                <source network='default'/>
                <target dev='vnet3'/>
                <model type='virtio'/>
            </interface>
        </devices></domain>"#;
        let iface = extract_interface_targets(xml);
        assert_eq!(iface.len(), 1);
        assert_eq!(iface[0].path, "vnet3");
        assert_eq!(iface[0].mac, "52:54:00:aa:bb:cc");
        assert_eq!(iface[0].model, "virtio");
    }

    #[test]
    fn interface_falls_back_to_mac_when_no_target() {
        let xml = r#"<domain><devices>
            <interface type='bridge'>
                <mac address='52:54:00:11:22:33'/>
                <source bridge='br0'/>
                <model type='e1000e'/>
            </interface>
        </devices></domain>"#;
        let iface = extract_interface_targets(xml);
        assert_eq!(iface[0].path, "52:54:00:11:22:33");
    }

    #[test]
    fn extracts_multiple_interfaces_in_order() {
        let xml = r#"<domain><devices>
            <interface type='bridge'><mac address='aa:aa:aa:aa:aa:aa'/><target dev='vnet1'/><model type='virtio'/></interface>
            <interface type='bridge'><mac address='bb:bb:bb:bb:bb:bb'/><target dev='vnet2'/><model type='virtio'/></interface>
        </devices></domain>"#;
        let iface = extract_interface_targets(xml);
        assert_eq!(iface.len(), 2);
        assert_eq!(iface[0].path, "vnet1");
        assert_eq!(iface[1].path, "vnet2");
    }

}
