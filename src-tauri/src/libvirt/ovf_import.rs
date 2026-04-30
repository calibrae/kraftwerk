//! OVA / OVF import.
//!
//! An OVA is a tar archive packaging an OVF descriptor (XML), one or
//! more VMDK disks, and optionally a manifest + cert. We parse the
//! OVF for the bits we need to build a libvirt domain XML — name,
//! CPU count, memory, disks, network — and let qemu-img convert the
//! VMDKs to qcow2 on the destination host.
//!
//! Scope (v1):
//! - One `<VirtualSystem>` per OVF (the only common case in the wild).
//! - Disks declared via `<DiskSection>` + `<References>` linkage.
//! - VMDK source format only (the `vmdk` URI in `<File>`/`<Disk>`).
//! - Network mappings get flattened to "first libvirt network on the
//!   target" — operators usually want to pick one anyway.
//!
//! What we don't handle yet:
//! - Encrypted OVAs (`<Envelope>` with security child elements).
//! - Multi-VM OVF appliances.
//! - Vendor-specific extensions (we just ignore namespaces we don't know).

use serde::{Deserialize, Serialize};

use crate::models::error::VirtManagerError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OvfMetadata {
    /// `Name` element from `<VirtualSystem>`. Empty fallback when absent.
    pub name: String,
    /// vCPU count from `<rasd:VirtualQuantity>` for ResourceType=3.
    /// `None` when unspecified — caller picks a default.
    pub vcpus: Option<u32>,
    /// Memory in MiB. Computed from ResourceType=4 plus the
    /// `AllocationUnits` (typically `byte * 2^20`).
    pub memory_mib: Option<u64>,
    /// Disks in declaration order.
    pub disks: Vec<OvfDisk>,
    /// Logical network names referenced in the OVF.
    pub networks: Vec<String>,
    /// Guest OS hint (e.g. "fedoraGuest", "ubuntu64Guest"). Lets the
    /// UI pre-pick a domain capabilities preset.
    pub guest_os: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OvfDisk {
    /// `id` attribute on `<Disk>`. Used to cross-reference `<File>`.
    pub disk_id: String,
    /// Resolved relative file path inside the OVA (from `<File href>`).
    pub file_href: Option<String>,
    /// Capacity in bytes (resolved from the `Capacity` + `CapacityAllocationUnits`
    /// attributes on `<Disk>`).
    pub capacity_bytes: Option<u64>,
    /// Source format URI, e.g.
    /// `http://www.vmware.com/interfaces/specifications/vmdk.html#streamOptimized`.
    pub format: Option<String>,
}

/// Parse an OVF descriptor (the .ovf file inside the tarball, or the
/// inline XML when shipped unbundled).
pub fn parse_ovf(xml: &str) -> Result<OvfMetadata, VirtManagerError> {
    let mut md = OvfMetadata::default();

    // <VirtualSystem><Name>...
    if let Some(name) = extract_text(xml, "Name") {
        md.name = name.trim().to_string();
    }

    // OperatingSystemSection has the guest OS id as `vmw:osType` or
    // `<Description>`.
    if let Some(os) = extract_attr(xml, "OperatingSystemSection", "vmw:osType")
        .or_else(|| extract_text_after(xml, "OperatingSystemSection", "Description"))
    {
        md.guest_os = Some(os.trim().to_string());
    }

    // Iterate `<Item>` entries inside `<VirtualHardwareSection>`. Each
    // Item describes one piece of virtual hardware: CPU, RAM, disk
    // controller, NIC, etc. We only care about CPU (ResourceType=3) and
    // RAM (ResourceType=4) here; disks are resolved from <DiskSection>.
    let mut cursor = 0usize;
    while let Some(rel) = xml[cursor..].find("<Item>").or_else(|| xml[cursor..].find("<Item ")) {
        let abs = cursor + rel;
        let end_rel = xml[abs..].find("</Item>").map(|e| abs + e + "</Item>".len());
        if let Some(end) = end_rel {
            let block = &xml[abs..end];
            if let Some(rt) = extract_text(block, "rasd:ResourceType")
                .or_else(|| extract_text(block, "ResourceType"))
            {
                let rt: u32 = rt.trim().parse().unwrap_or(0);
                let qty = extract_text(block, "rasd:VirtualQuantity")
                    .or_else(|| extract_text(block, "VirtualQuantity"))
                    .and_then(|s| s.trim().parse::<u64>().ok());
                let units = extract_text(block, "rasd:AllocationUnits")
                    .or_else(|| extract_text(block, "AllocationUnits"))
                    .unwrap_or_default();
                match rt {
                    3 => md.vcpus = qty.map(|q| q.min(u32::MAX as u64) as u32),
                    4 => {
                        if let Some(q) = qty {
                            md.memory_mib = Some(scale_memory_to_mib(q, &units));
                        }
                    }
                    10 => {
                        // Network adapter — record the connection name.
                        if let Some(net) = extract_text(block, "rasd:Connection")
                            .or_else(|| extract_text(block, "Connection"))
                        {
                            let n = net.trim().to_string();
                            if !n.is_empty() && !md.networks.contains(&n) {
                                md.networks.push(n);
                            }
                        }
                    }
                    _ => {}
                }
            }
            cursor = end;
        } else {
            break;
        }
    }

    // <DiskSection> + <References><File ...> linkage.
    let mut disks: Vec<OvfDisk> = Vec::new();
    let mut dc = 0usize;
    while let Some(rel) = xml[dc..].find("<Disk ") {
        let abs = dc + rel;
        let head_end = xml[abs..]
            .find("/>")
            .map(|e| abs + e + 2)
            .or_else(|| xml[abs..].find('>').map(|e| abs + e + 1))
            .unwrap_or(xml.len());
        let head = &xml[abs..head_end];
        let disk_id = attr_in_head(head, "ovf:diskId").unwrap_or_default();
        let file_id = attr_in_head(head, "ovf:fileRef");
        let cap_str = attr_in_head(head, "ovf:capacity");
        let cap_units = attr_in_head(head, "ovf:capacityAllocationUnits").unwrap_or_default();
        let format = attr_in_head(head, "ovf:format");
        let capacity_bytes = cap_str.and_then(|s| s.trim().parse::<u64>().ok())
            .map(|n| scale_capacity_to_bytes(n, &cap_units));
        let mut d = OvfDisk {
            disk_id,
            file_href: None,
            capacity_bytes,
            format,
        };
        if let Some(fid) = file_id {
            // Resolve <File ovf:id='fid' ovf:href='...'>.
            if let Some(href) = find_file_href_for_id(xml, &fid) {
                d.file_href = Some(href);
            }
        }
        disks.push(d);
        dc = head_end;
    }
    md.disks = disks;

    if md.name.is_empty() && md.disks.is_empty() {
        return Err(VirtManagerError::XmlParsingFailed {
            reason: "OVF missing <Name> and disks — unrecognised structure".into(),
        });
    }
    Ok(md)
}

fn find_file_href_for_id(xml: &str, file_id: &str) -> Option<String> {
    let mut cursor = 0usize;
    while let Some(rel) = xml[cursor..].find("<File ") {
        let abs = cursor + rel;
        let head_end = xml[abs..]
            .find("/>")
            .map(|e| abs + e + 2)
            .or_else(|| xml[abs..].find('>').map(|e| abs + e + 1))
            .unwrap_or(xml.len());
        let head = &xml[abs..head_end];
        let id = attr_in_head(head, "ovf:id").unwrap_or_default();
        if id == file_id {
            return attr_in_head(head, "ovf:href");
        }
        cursor = head_end;
    }
    None
}

fn extract_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = block.find(&open)? + open.len();
    let e = block[s..].find(&close)?;
    Some(block[s..s + e].to_string())
}

/// Like `extract_text`, but find the tag *inside* a parent tag's
/// block. Used for DESCRIPTION-style fields.
fn extract_text_after(xml: &str, parent: &str, tag: &str) -> Option<String> {
    let parent_open = format!("<{parent}");
    let parent_idx = xml.find(&parent_open)?;
    let parent_close = format!("</{parent}>");
    let end = xml[parent_idx..].find(&parent_close).map(|e| parent_idx + e)?;
    let block = &xml[parent_idx..end];
    extract_text(block, tag)
}

fn extract_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let open = format!("<{tag} ");
    let s = xml.find(&open)?;
    let after = &xml[s..];
    let head_end = after.find('>').unwrap_or(after.len());
    let head = &after[..head_end];
    attr_in_head(head, attr)
}

fn attr_in_head(head: &str, attr: &str) -> Option<String> {
    for q in ['\'', '"'] {
        let needle = format!("{attr}={q}");
        if let Some(s) = head.find(&needle) {
            let after = &head[s + needle.len()..];
            if let Some(e) = after.find(q) {
                return Some(after[..e].to_string());
            }
        }
    }
    None
}

/// OVF AllocationUnits look like "byte * 2^20" or "MByte" (SI). We
/// only need to recognise the common variants and scale to MiB.
pub fn scale_memory_to_mib(qty: u64, units: &str) -> u64 {
    let u = units.replace(' ', "").to_lowercase();
    if u.contains("2^20") || u == "mbyte" || u == "mibyte" || u == "megabyte" || u == "mb" {
        qty
    } else if u.contains("2^30") || u == "gbyte" || u == "gibyte" || u == "gb" {
        qty * 1024
    } else if u.contains("2^10") || u == "kbyte" || u == "kibyte" || u == "kb" {
        qty / 1024
    } else if u == "byte" || u.is_empty() {
        qty / (1024 * 1024)
    } else {
        // Unknown unit — assume the value is already in MiB.
        qty
    }
}

/// Same idea for capacity — usually `byte * 2^30` or just `byte`.
/// Returns bytes.
pub fn scale_capacity_to_bytes(qty: u64, units: &str) -> u64 {
    let u = units.replace(' ', "").to_lowercase();
    if u.contains("2^30") || u == "gbyte" || u == "gb" || u == "gibyte" {
        qty * 1024u64.pow(3)
    } else if u.contains("2^20") || u == "mbyte" || u == "mb" || u == "mibyte" {
        qty * 1024u64.pow(2)
    } else if u.contains("2^10") || u == "kbyte" || u == "kb" || u == "kibyte" {
        qty * 1024
    } else {
        qty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OVF: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<Envelope xmlns="http://schemas.dmtf.org/ovf/envelope/1"
          xmlns:ovf="http://schemas.dmtf.org/ovf/envelope/1"
          xmlns:rasd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData">
  <References>
    <File ovf:id='file1' ovf:href='disk1.vmdk' ovf:size='123456789'/>
  </References>
  <DiskSection>
    <Disk ovf:diskId='vmdisk1' ovf:fileRef='file1' ovf:capacity='40' ovf:capacityAllocationUnits='byte * 2^30' ovf:format='http://www.vmware.com/interfaces/specifications/vmdk.html#streamOptimized'/>
  </DiskSection>
  <NetworkSection>
    <Network ovf:name='VM Network'><Description>The VM Network</Description></Network>
  </NetworkSection>
  <VirtualSystem ovf:id='vm1'>
    <Name>imported-vm</Name>
    <OperatingSystemSection ovf:id='80' vmw:osType='ubuntu64Guest'>
      <Description>Ubuntu Linux (64-bit)</Description>
    </OperatingSystemSection>
    <VirtualHardwareSection>
      <Item>
        <rasd:ResourceType>3</rasd:ResourceType>
        <rasd:VirtualQuantity>4</rasd:VirtualQuantity>
      </Item>
      <Item>
        <rasd:AllocationUnits>byte * 2^20</rasd:AllocationUnits>
        <rasd:ResourceType>4</rasd:ResourceType>
        <rasd:VirtualQuantity>4096</rasd:VirtualQuantity>
      </Item>
      <Item>
        <rasd:Connection>VM Network</rasd:Connection>
        <rasd:ResourceType>10</rasd:ResourceType>
      </Item>
    </VirtualHardwareSection>
  </VirtualSystem>
</Envelope>"#;

    #[test]
    fn parses_basic_ovf() {
        let md = parse_ovf(SAMPLE_OVF).unwrap();
        assert_eq!(md.name, "imported-vm");
        assert_eq!(md.vcpus, Some(4));
        assert_eq!(md.memory_mib, Some(4096));
        assert_eq!(md.disks.len(), 1);
        assert_eq!(md.disks[0].disk_id, "vmdisk1");
        assert_eq!(md.disks[0].file_href.as_deref(), Some("disk1.vmdk"));
        assert_eq!(md.disks[0].capacity_bytes, Some(40 * 1024u64.pow(3)));
        assert!(md.disks[0].format.as_deref().unwrap().contains("vmdk"));
        assert_eq!(md.networks, vec!["VM Network".to_string()]);
        assert!(md.guest_os.as_deref().unwrap().contains("ubuntu"));
    }

    #[test]
    fn memory_unit_scales() {
        assert_eq!(scale_memory_to_mib(4096, "byte * 2^20"), 4096);
        assert_eq!(scale_memory_to_mib(4, "byte * 2^30"), 4096);
        assert_eq!(scale_memory_to_mib(2, "GByte"), 2048);
        assert_eq!(scale_memory_to_mib(2048, "MByte"), 2048);
    }

    #[test]
    fn capacity_unit_scales() {
        assert_eq!(scale_capacity_to_bytes(40, "byte * 2^30"), 40 * 1024u64.pow(3));
        assert_eq!(scale_capacity_to_bytes(512, "byte * 2^20"), 512 * 1024u64.pow(2));
        assert_eq!(scale_capacity_to_bytes(1000, ""), 1000);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_ovf("<not-ovf/>").is_err());
    }

    #[test]
    fn parses_minimal_no_network() {
        let xml = r#"<Envelope>
  <References><File ovf:id='f' ovf:href='d.vmdk'/></References>
  <DiskSection><Disk ovf:diskId='d1' ovf:fileRef='f' ovf:capacity='10' ovf:capacityAllocationUnits='byte * 2^30'/></DiskSection>
  <VirtualSystem><Name>tiny</Name></VirtualSystem>
</Envelope>"#;
        let md = parse_ovf(xml).unwrap();
        assert_eq!(md.name, "tiny");
        assert!(md.networks.is_empty());
        assert_eq!(md.disks.len(), 1);
        assert_eq!(md.disks[0].capacity_bytes, Some(10 * 1024u64.pow(3)));
    }
}
