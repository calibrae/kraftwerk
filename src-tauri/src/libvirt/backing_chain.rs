//! Backing-chain parsing for qcow2 / external-snapshot disks.
//!
//! libvirt's domain XML embeds the chain as nested `<backingStore>`
//! elements inside each `<disk>`:
//!
//! ```xml
//! <disk type='file' device='disk'>
//!   <source file='/var/lib/libvirt/images/foo.snap2'/>
//!   <target dev='vda' bus='virtio'/>
//!   <backingStore type='file'>
//!     <source file='/var/lib/libvirt/images/foo.snap1'/>
//!     <format type='qcow2'/>
//!     <backingStore type='file'>
//!       <source file='/var/lib/libvirt/images/foo.base'/>
//!       <format type='qcow2'/>
//!       <backingStore/>
//!     </backingStore>
//!   </backingStore>
//! </disk>
//! ```
//!
//! We walk the chain into a flat `Vec<ChainLink>`; the head is the
//! currently-active overlay (matches `<source>`), the tail is the
//! original base. Empty `<backingStore/>` terminates.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChainLink {
    /// Disk target (e.g. "vda"). Same across every link in a chain.
    /// Set on the disk-level summary, not per link.
    pub depth: u32,
    pub file: String,
    pub format: Option<String>,
}

/// Live block-job state for one disk. `cur` and `end` are byte
/// counters; `cur / end` is a progress fraction in [0, 1].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockJobInfo {
    pub kind: String, // "pull" | "copy" | "commit" | "active_commit" | "backup" | "unknown"
    pub bandwidth: u64,
    pub cur: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskBackingChain {
    pub target: String,
    /// `device` attr — disk / cdrom / floppy / lun. Useful so the UI
    /// can hide blockcommit/pull on read-only devices.
    pub device: String,
    pub source: Option<String>,
    pub source_format: Option<String>,
    pub readonly: bool,
    pub chain: Vec<ChainLink>,
}

/// Walk the domain XML and extract one DiskBackingChain per `<disk>`.
pub fn parse_chains(xml: &str) -> Vec<DiskBackingChain> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<disk ") {
        rest = &rest[start..];
        let Some(end_rel) = rest.find("</disk>") else { break };
        let block = &rest[..end_rel];

        let target = extract_attr(block, "<target ", "dev").unwrap_or_default();
        let device = extract_attr(block, "<disk ", "device").unwrap_or_else(|| "disk".into());
        let source = extract_attr(block, "<source ", "file")
            .or_else(|| extract_attr(block, "<source ", "dev"));
        let source_format = extract_attr(block, "<driver ", "type");
        let readonly = block.contains("<readonly/>");

        let chain = walk_backing_chain(block);

        if !target.is_empty() {
            out.push(DiskBackingChain {
                target,
                device,
                source,
                source_format,
                readonly,
                chain,
            });
        }

        rest = &rest[end_rel + "</disk>".len()..];
    }
    out
}

fn walk_backing_chain(disk_block: &str) -> Vec<ChainLink> {
    let mut links = Vec::new();
    let mut depth: u32 = 1;
    let mut cursor = disk_block;
    loop {
        let Some(idx) = cursor.find("<backingStore") else { break };
        cursor = &cursor[idx..];
        // Self-closing `<backingStore/>` terminates.
        if cursor.starts_with("<backingStore/>") {
            break;
        }
        // Only take a top-level `<backingStore>` — the recursion is
        // implicit because we re-find from the body of the previous
        // hit. To avoid double-counting, we advance past the opening tag.
        let after_tag = match cursor.find('>') {
            Some(i) => &cursor[i + 1..],
            None => break,
        };
        // Match the corresponding closing tag at the same depth.
        let body_end = match find_matching_close(after_tag, "backingStore") {
            Some(i) => i,
            None => break,
        };
        let body = &after_tag[..body_end];

        let file = extract_attr(body, "<source ", "file")
            .or_else(|| extract_attr(body, "<source ", "dev"))
            .unwrap_or_default();
        let format = extract_attr(body, "<format ", "type");

        if !file.is_empty() {
            links.push(ChainLink { depth, file, format });
        }
        depth += 1;
        cursor = body;
    }
    links
}

/// Find the index in `s` where the matching `</tag>` for the next
/// equally-named element starts, accounting for nested same-name tags.
fn find_matching_close(s: &str, tag: &str) -> Option<usize> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth: i32 = 1;
    let mut pos = 0;
    while pos < s.len() {
        let next_open = s[pos..].find(&open).map(|i| pos + i);
        let next_close = s[pos..].find(&close).map(|i| pos + i);
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                // Self-closing `<backingStore/>` doesn't increment depth.
                let after = &s[o..];
                if after.starts_with(&format!("<{tag}/>")) {
                    pos = o + format!("<{tag}/>").len();
                } else {
                    depth += 1;
                    pos = o + open.len();
                }
            }
            (_, Some(c)) => {
                depth -= 1;
                if depth == 0 {
                    return Some(c);
                }
                pos = c + close.len();
            }
            _ => return None,
        }
    }
    None
}

fn extract_attr(block: &str, tag_prefix: &str, attr: &str) -> Option<String> {
    let i = block.find(tag_prefix)?;
    let rest = &block[i..];
    // Stop at the closing `>` of this opening tag so we don't read
    // attributes from a sibling element.
    let close = rest.find('>')?;
    let header = &rest[..close];
    for q in ['\'', '"'] {
        let needle = format!("{attr}={q}");
        if let Some(s) = header.find(&needle) {
            let after = &header[s + needle.len()..];
            if let Some(e) = after.find(q) {
                return Some(after[..e].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<domain>
<devices>
<disk type='file' device='disk'>
  <driver name='qemu' type='qcow2'/>
  <source file='/var/lib/libvirt/images/foo.snap2'/>
  <target dev='vda' bus='virtio'/>
  <backingStore type='file' index='1'>
    <format type='qcow2'/>
    <source file='/var/lib/libvirt/images/foo.snap1'/>
    <backingStore type='file' index='2'>
      <format type='qcow2'/>
      <source file='/var/lib/libvirt/images/foo.base'/>
      <backingStore/>
    </backingStore>
  </backingStore>
</disk>
<disk type='file' device='cdrom'>
  <driver name='qemu' type='raw'/>
  <source file='/srv/iso/install.iso'/>
  <target dev='sda' bus='sata'/>
  <readonly/>
  <backingStore/>
</disk>
</devices>
</domain>"#;

    #[test]
    fn parses_two_disks() {
        let chains = parse_chains(SAMPLE);
        assert_eq!(chains.len(), 2);
        assert_eq!(chains[0].target, "vda");
        assert_eq!(chains[1].target, "sda");
        assert_eq!(chains[1].device, "cdrom");
        assert!(chains[1].readonly);
    }

    #[test]
    fn walks_nested_backing_chain() {
        let chains = parse_chains(SAMPLE);
        let vda = &chains[0];
        assert_eq!(vda.source.as_deref(), Some("/var/lib/libvirt/images/foo.snap2"));
        assert_eq!(vda.chain.len(), 2);
        assert_eq!(vda.chain[0].depth, 1);
        assert_eq!(vda.chain[0].file, "/var/lib/libvirt/images/foo.snap1");
        assert_eq!(vda.chain[0].format.as_deref(), Some("qcow2"));
        assert_eq!(vda.chain[1].depth, 2);
        assert_eq!(vda.chain[1].file, "/var/lib/libvirt/images/foo.base");
    }

    #[test]
    fn empty_backing_store_terminates() {
        let chains = parse_chains(SAMPLE);
        let cdrom = &chains[1];
        assert!(cdrom.chain.is_empty());
    }

    #[test]
    fn handles_disk_without_backing_chain_at_all() {
        let xml = r#"<domain><devices>
            <disk type='block' device='disk'>
              <source dev='/dev/vg/foo'/>
              <target dev='vdb'/>
            </disk>
        </devices></domain>"#;
        let chains = parse_chains(xml);
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].source.as_deref(), Some("/dev/vg/foo"));
        assert!(chains[0].chain.is_empty());
    }
}
