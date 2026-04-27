//! VM cloning: full-copy via virStorageVolCreateXMLFrom for each
//! r/w disk, then domain XML rewrite (new name, fresh UUID, MACs
//! stripped so libvirt assigns new ones).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneOptions {
    /// New domain name. Must be unique on the hypervisor.
    pub target_name: String,
    /// If true, randomize MACs on every NIC (else libvirt assigns).
    /// In v1 we always strip MACs; this flag is reserved for a future
    /// "keep MACs (cloning to a different network)" mode.
    pub randomize_macs: bool,
    /// If true, start the cloned VM immediately after define.
    pub start_after: bool,
}

/// Build the volume XML for a clone target. Same format and capacity
/// as the source; libvirt copies the bytes during create_xml_from.
pub fn build_clone_volume_xml(target_name: &str, capacity_bytes: u64, format: &str) -> String {
    format!(
        "<volume>\n  <name>{}</name>\n  <capacity unit='B'>{}</capacity>\n  <target>\n    <format type='{}'/>\n  </target>\n</volume>",
        escape_xml(target_name),
        capacity_bytes,
        escape_xml(format),
    )
}

/// Rewrite a domain XML for the clone:
/// - replace `<name>` content
/// - drop `<uuid>` (libvirt regenerates)
/// - drop every `<mac address='..'/>` inside `<interface>` (libvirt regenerates)
/// - replace each disk source path via the provided map
pub fn rewrite_domain_xml(xml: &str, target_name: &str, disk_path_map: &[(String, String)]) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut s = xml;

    // 1) Replace <name>...</name> (first occurrence at top of <domain>).
    if let (Some(start), Some(end)) = (s.find("<name>"), s.find("</name>")) {
        out.push_str(&s[..start]);
        out.push_str("<name>");
        out.push_str(&escape_xml(target_name));
        out.push_str("</name>");
        s = &s[end + "</name>".len()..];
    }
    // 2) Strip <uuid>...</uuid>.
    if let (Some(start), Some(end)) = (s.find("<uuid>"), s.find("</uuid>")) {
        out.push_str(&s[..start]);
        s = &s[end + "</uuid>".len()..];
        // Eat the trailing newline if present so we don't leave an empty line.
        if s.starts_with('\n') {
            s = &s[1..];
        }
    }
    out.push_str(s);

    // 3) Strip <mac address='..'/> entries.
    let mut without_macs = String::with_capacity(out.len());
    let mut rem = out.as_str();
    while let Some(i) = rem.find("<mac ") {
        without_macs.push_str(&rem[..i]);
        // Find the closing /> or </mac>
        let after = &rem[i..];
        let end = after.find("/>").map(|e| e + 2)
            .or_else(|| after.find("</mac>").map(|e| e + "</mac>".len()));
        if let Some(e) = end {
            // Eat preceding indentation + trailing newline so we don't leave a blank line.
            // Trim from the last \n in without_macs to the end of <mac.../>.
            if let Some(last_nl) = without_macs.rfind('\n') {
                let trail = &without_macs[last_nl + 1..];
                if trail.chars().all(|c| c.is_whitespace()) {
                    without_macs.truncate(last_nl);
                }
            }
            rem = &after[e..];
            // Eat the immediately-following newline.
            if rem.starts_with('\n') { rem = &rem[1..]; }
        } else {
            break;
        }
    }
    without_macs.push_str(rem);

    // 4) Replace disk source paths.
    let mut final_xml = without_macs;
    for (old_path, new_path) in disk_path_map {
        // Both single- and double-quoted forms.
        for q in ['\'', '"'] {
            let needle = format!("file={q}{old_path}{q}");
            let replacement = format!("file={q}{new_path}{q}");
            final_xml = final_xml.replace(&needle, &replacement);
            let needle = format!("dev={q}{old_path}{q}");
            let replacement = format!("dev={q}{new_path}{q}");
            final_xml = final_xml.replace(&needle, &replacement);
        }
    }
    final_xml
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_name_uuid_mac() {
        let xml = r#"<domain type='kvm'>
  <name>source-vm</name>
  <uuid>11111111-1111-1111-1111-111111111111</uuid>
  <memory unit='KiB'>2097152</memory>
  <devices>
    <interface type='network'>
      <mac address='52:54:00:00:00:01'/>
      <source network='default'/>
    </interface>
    <disk type='file' device='disk'>
      <source file='/var/lib/libvirt/images/source-vm.qcow2'/>
      <target dev='vda'/>
    </disk>
  </devices>
</domain>"#;
        let map = vec![(
            "/var/lib/libvirt/images/source-vm.qcow2".to_string(),
            "/var/lib/libvirt/images/clone-vm.qcow2".to_string(),
        )];
        let out = rewrite_domain_xml(xml, "clone-vm", &map);
        assert!(out.contains("<name>clone-vm</name>"));
        assert!(!out.contains("11111111-1111-1111-1111-111111111111"));
        assert!(!out.contains("<mac address"));
        assert!(out.contains("/var/lib/libvirt/images/clone-vm.qcow2"));
        assert!(!out.contains("/var/lib/libvirt/images/source-vm.qcow2"));
    }

    #[test]
    fn build_volume_xml_includes_capacity_and_format() {
        let v = build_clone_volume_xml("foo.qcow2", 21_474_836_480, "qcow2");
        assert!(v.contains("<name>foo.qcow2</name>"));
        assert!(v.contains("<capacity unit='B'>21474836480</capacity>"));
        assert!(v.contains("<format type='qcow2'/>"));
    }

    #[test]
    fn escapes_xml_in_target_name() {
        let xml = "<domain><name>x</name></domain>";
        let out = rewrite_domain_xml(xml, "a&b<c>", &[]);
        assert!(out.contains("a&amp;b&lt;c&gt;"));
    }
}
