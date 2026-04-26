//! VM snapshot model + parsing helpers.
//!
//! For v1 we surface internal qcow2 snapshots only (the common case).
//! External snapshots — which require pre-creating overlay files and
//! managing backing chains — are deferred.

use serde::{Deserialize, Serialize};

/// A flat snapshot record. Tree relationships are reconstructed on the
/// frontend from `parent_name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub name: String,
    pub parent_name: Option<String>,
    pub description: Option<String>,
    /// libvirt domain state captured in the snapshot:
    /// "running", "paused", "shutoff", "crashed", "pmsuspended", "unknown".
    pub state: String,
    /// Unix epoch seconds when the snapshot was created.
    pub creation_time: i64,
    pub is_current: bool,
    pub has_memory: bool,
    pub has_metadata: bool,
    /// Best-effort number of disks captured.
    pub disk_count: u32,
}

/// Parse a single `<domainsnapshot>` XML blob into a SnapshotInfo.
/// Caller fills in `is_current` and `has_metadata` from libvirt query
/// methods rather than relying on the XML.
pub fn parse_snapshot_xml(xml: &str) -> SnapshotInfo {
    let name = extract_tag_text(xml, "name").unwrap_or_default();
    let description = extract_tag_text(xml, "description");
    let state = extract_tag_text(xml, "state").unwrap_or_else(|| "unknown".into());
    let creation_time = extract_tag_text(xml, "creationTime")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let parent_name = extract_parent_name(xml);
    let has_memory = extract_memory_snapshot_kind(xml)
        .map(|k| k != "no")
        .unwrap_or(false);
    let disk_count = count_disks_with_snapshot(xml);

    SnapshotInfo {
        name,
        parent_name,
        description,
        state,
        creation_time,
        is_current: false,
        has_memory,
        has_metadata: true,
        disk_count,
    }
}

/// Build the minimal snapshot XML for create. libvirt fills in disks /
/// memory / state from the running domain.
pub fn build_create_xml(name: &str, description: Option<&str>) -> String {
    let name_esc = escape_xml(name);
    match description {
        Some(d) if !d.is_empty() => format!(
            "<domainsnapshot>\n  <name>{}</name>\n  <description>{}</description>\n</domainsnapshot>",
            name_esc,
            escape_xml(d)
        ),
        _ => format!(
            "<domainsnapshot>\n  <name>{}</name>\n</domainsnapshot>",
            name_esc
        ),
    }
}

// --- helpers ---

fn extract_tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

fn extract_parent_name(xml: &str) -> Option<String> {
    // <parent><name>foo</name></parent>
    let parent_start = xml.find("<parent>")? + "<parent>".len();
    let parent_end = xml[parent_start..].find("</parent>")? + parent_start;
    extract_tag_text(&xml[parent_start..parent_end], "name")
}

fn extract_memory_snapshot_kind(xml: &str) -> Option<String> {
    // <memory snapshot="internal"/> or <memory snapshot='external' file='...'/>
    let i = xml.find("<memory")?;
    let rest = &xml[i..];
    let attr_start = rest.find("snapshot=")? + "snapshot=".len() + 1; // skip quote
    let quote = rest.as_bytes()[rest.find("snapshot=")? + "snapshot=".len()] as char;
    let attr_end = rest[attr_start..].find(quote)? + attr_start;
    Some(rest[attr_start..attr_end].to_string())
}

fn count_disks_with_snapshot(xml: &str) -> u32 {
    // Count <disk ...> entries inside <disks>...</disks>.
    let Some(start) = xml.find("<disks>") else { return 0 };
    let after = &xml[start..];
    let Some(end) = after.find("</disks>") else { return 0 };
    let block = &after[..end];
    block
        .match_indices("<disk")
        .filter(|(i, _)| {
            let after = block.as_bytes().get(i + 5).copied();
            // Exclude <disks (the parent element itself); accept <disk + space/slash/etc.
            !matches!(after, Some(b's') | Some(b'S'))
        })
        .count() as u32
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
    fn parses_minimal_snapshot_xml() {
        let xml = r#"<domainsnapshot>
            <name>snap1</name>
            <description>before update</description>
            <state>running</state>
            <creationTime>1700000000</creationTime>
            <memory snapshot='internal'/>
            <disks>
              <disk name='vda' snapshot='internal'/>
              <disk name='vdb' snapshot='internal'/>
            </disks>
        </domainsnapshot>"#;
        let s = parse_snapshot_xml(xml);
        assert_eq!(s.name, "snap1");
        assert_eq!(s.description.as_deref(), Some("before update"));
        assert_eq!(s.state, "running");
        assert_eq!(s.creation_time, 1_700_000_000);
        assert_eq!(s.parent_name, None);
        assert!(s.has_memory);
        assert_eq!(s.disk_count, 2);
    }

    #[test]
    fn extracts_parent_name() {
        let xml = r#"<domainsnapshot>
            <name>child</name>
            <parent><name>parent</name></parent>
            <state>shutoff</state>
            <creationTime>0</creationTime>
        </domainsnapshot>"#;
        let s = parse_snapshot_xml(xml);
        assert_eq!(s.parent_name.as_deref(), Some("parent"));
    }

    #[test]
    fn no_memory_snapshot_means_disk_only() {
        let xml = r#"<domainsnapshot>
            <name>diskonly</name>
            <state>shutoff</state>
            <creationTime>0</creationTime>
            <memory snapshot='no'/>
        </domainsnapshot>"#;
        let s = parse_snapshot_xml(xml);
        assert!(!s.has_memory);
    }

    #[test]
    fn build_create_xml_escapes_name_and_description() {
        let xml = build_create_xml("a&b", Some("foo<bar>"));
        assert!(xml.contains("<name>a&amp;b</name>"));
        assert!(xml.contains("<description>foo&lt;bar&gt;</description>"));
    }

    #[test]
    fn build_create_xml_omits_empty_description() {
        let xml = build_create_xml("snap", None);
        assert!(!xml.contains("description"));
        let xml2 = build_create_xml("snap", Some(""));
        assert!(!xml2.contains("description"));
    }
}
