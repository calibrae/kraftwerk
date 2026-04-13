//! Parse libvirt storage pool + volume XML, and build XML for creation.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

// ────────────── Pool config ──────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PoolConfig {
    pub name: String,
    pub uuid: String,
    pub pool_type: String,
    pub capacity: u64,
    pub allocation: u64,
    pub available: u64,
    pub target_path: Option<String>,
    /// For netfs pools: the NFS host.
    pub source_host: Option<String>,
    /// For netfs pools: the exported directory on the host.
    pub source_dir: Option<String>,
    /// For logical pools: the volume group name.
    pub source_name: Option<String>,
}

/// Parse pool XML.
pub fn parse_pool(xml: &str) -> Result<PoolConfig, VirtManagerError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut cfg = PoolConfig::default();
    let mut path: Vec<String> = Vec::new();
    let mut buf = Vec::new();

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
                handle_pool_start(&mut cfg, &path, &name, &attrs);
                path.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let attrs = collect_attrs(&e);
                handle_pool_start(&mut cfg, &path, &name, &attrs);
            }
            Ok(Event::End(_)) => {
                path.pop();
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                handle_pool_text(&mut cfg, &path, &text);
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(cfg)
}

fn handle_pool_start(cfg: &mut PoolConfig, path: &[String], name: &str, attrs: &[(String, String)]) {
    let parent_is = |p: &str| path.last().map(String::as_str) == Some(p);

    match name {
        "pool" if path.is_empty() => {
            cfg.pool_type = get_attr(attrs, "type").unwrap_or("").to_string();
        }
        "capacity" | "allocation" | "available" => {
            // values come in as text
        }
        "host" if parent_is("source") => {
            cfg.source_host = get_attr(attrs, "name").map(String::from);
        }
        "dir" if parent_is("source") => {
            cfg.source_dir = get_attr(attrs, "path").map(String::from);
        }
        "name" if parent_is("source") => {
            // logical pool VG name is a text node; captured in handle_pool_text
        }
        _ => {}
    }
}

fn handle_pool_text(cfg: &mut PoolConfig, path: &[String], text: &str) {
    let last = path.last().map(String::as_str);
    let parent = if path.len() >= 2 {
        Some(path[path.len() - 2].as_str())
    } else {
        None
    };

    match (last, parent) {
        (Some("name"), Some("pool")) => cfg.name = text.to_string(),
        (Some("uuid"), Some("pool")) => cfg.uuid = text.to_string(),
        (Some("capacity"), Some("pool")) => {
            cfg.capacity = text.trim().parse().unwrap_or(0);
        }
        (Some("allocation"), Some("pool")) => {
            cfg.allocation = text.trim().parse().unwrap_or(0);
        }
        (Some("available"), Some("pool")) => {
            cfg.available = text.trim().parse().unwrap_or(0);
        }
        (Some("path"), Some("target")) => {
            cfg.target_path = Some(text.to_string());
        }
        (Some("name"), Some("source")) => {
            cfg.source_name = Some(text.to_string());
        }
        _ => {}
    }
}

// ────────────── Volume config ──────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VolumeConfig {
    pub name: String,
    pub key: String,
    pub path: String,
    pub capacity: u64,
    pub allocation: u64,
    pub format: String,
}

pub fn parse_volume(xml: &str) -> Result<VolumeConfig, VirtManagerError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut cfg = VolumeConfig::default();
    let mut path: Vec<String> = Vec::new();
    let mut buf = Vec::new();

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
                handle_volume_start(&mut cfg, &path, &name, &attrs);
                path.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let attrs = collect_attrs(&e);
                handle_volume_start(&mut cfg, &path, &name, &attrs);
            }
            Ok(Event::End(_)) => {
                path.pop();
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                handle_volume_text(&mut cfg, &path, &text);
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(cfg)
}

fn handle_volume_start(cfg: &mut VolumeConfig, path: &[String], name: &str, attrs: &[(String, String)]) {
    let parent_is = |p: &str| path.last().map(String::as_str) == Some(p);

    match name {
        "format" if parent_is("target") => {
            cfg.format = get_attr(attrs, "type").unwrap_or("").to_string();
        }
        _ => {}
    }
}

fn handle_volume_text(cfg: &mut VolumeConfig, path: &[String], text: &str) {
    let last = path.last().map(String::as_str);
    let parent = if path.len() >= 2 {
        Some(path[path.len() - 2].as_str())
    } else {
        None
    };

    match (last, parent) {
        (Some("name"), Some("volume")) => cfg.name = text.to_string(),
        (Some("key"), Some("volume")) => cfg.key = text.to_string(),
        (Some("capacity"), Some("volume")) => {
            cfg.capacity = text.trim().parse().unwrap_or(0);
        }
        (Some("allocation"), Some("volume")) => {
            cfg.allocation = text.trim().parse().unwrap_or(0);
        }
        (Some("path"), Some("target")) => {
            cfg.path = text.to_string();
        }
        _ => {}
    }
}

// ────────────── Builders ──────────────

/// Parameters for a new storage pool.
#[derive(Debug, Clone, Default)]
pub struct PoolBuildParams<'a> {
    pub name: &'a str,
    /// "dir" | "netfs" | "logical" | "iscsi"
    pub pool_type: &'a str,
    /// Target path on host (used by `dir` and `netfs` as the mount point).
    pub target_path: Option<&'a str>,
    /// For netfs: NFS server hostname.
    pub source_host: Option<&'a str>,
    /// For netfs: exported dir on the server, or iSCSI target path.
    pub source_dir: Option<&'a str>,
    /// For logical: VG name. For iscsi: target IQN.
    pub source_name: Option<&'a str>,
}

pub fn build_pool_xml(p: &PoolBuildParams) -> String {
    let t = p.pool_type.trim();
    let mut xml = format!("<pool type='{}'>\n", escape_xml(t));
    xml.push_str(&format!("  <name>{}</name>\n", escape_xml(p.name)));

    // Source section — only for types that need it
    let needs_source = matches!(t, "netfs" | "logical" | "iscsi");
    if needs_source {
        xml.push_str("  <source>\n");
        if let Some(h) = p.source_host {
            if !h.is_empty() {
                xml.push_str(&format!("    <host name='{}'/>\n", escape_xml(h)));
            }
        }
        if let Some(d) = p.source_dir {
            if !d.is_empty() {
                xml.push_str(&format!("    <dir path='{}'/>\n", escape_xml(d)));
            }
        }
        if let Some(n) = p.source_name {
            if !n.is_empty() {
                xml.push_str(&format!("    <name>{}</name>\n", escape_xml(n)));
            }
        }
        xml.push_str("  </source>\n");
    }

    // Target — always present
    if let Some(path) = p.target_path {
        if !path.is_empty() {
            xml.push_str("  <target>\n");
            xml.push_str(&format!("    <path>{}</path>\n", escape_xml(path)));
            xml.push_str("  </target>\n");
        }
    }

    xml.push_str("</pool>\n");
    xml
}

/// Parameters for a new volume.
#[derive(Debug, Clone)]
pub struct VolumeBuildParams<'a> {
    pub name: &'a str,
    /// Virtual capacity in bytes.
    pub capacity_bytes: u64,
    /// "qcow2" | "raw" | "iso"
    pub format: &'a str,
    /// Optional initial allocation. None = thin-provisioned for qcow2.
    pub allocation_bytes: Option<u64>,
}

pub fn build_volume_xml(p: &VolumeBuildParams) -> String {
    let mut xml = String::from("<volume>\n");
    xml.push_str(&format!("  <name>{}</name>\n", escape_xml(p.name)));
    xml.push_str(&format!("  <capacity unit='bytes'>{}</capacity>\n", p.capacity_bytes));
    if let Some(a) = p.allocation_bytes {
        xml.push_str(&format!("  <allocation unit='bytes'>{}</allocation>\n", a));
    }
    xml.push_str("  <target>\n");
    xml.push_str(&format!("    <format type='{}'/>\n", escape_xml(p.format)));
    xml.push_str("  </target>\n");
    xml.push_str("</volume>\n");
    xml
}

// ────────────── Helpers ──────────────

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

#[cfg(test)]
mod tests {
    use super::*;

    const DIR_POOL_XML: &str = r#"<pool type='dir'>
  <name>default</name>
  <uuid>b92ad468-745b-4806-8e69-b9c7051aad1a</uuid>
  <capacity unit='bytes'>254288068608</capacity>
  <allocation unit='bytes'>195908390912</allocation>
  <available unit='bytes'>58379677696</available>
  <source>
  </source>
  <target>
    <path>/var/lib/libvirt/images</path>
  </target>
</pool>
"#;

    const NETFS_POOL_XML: &str = r#"<pool type='netfs'>
  <name>isos</name>
  <uuid>x</uuid>
  <source>
    <host name='nfs.example'/>
    <dir path='/exports/isos'/>
    <format type='nfs'/>
  </source>
  <target>
    <path>/mnt/isos</path>
  </target>
</pool>
"#;

    const VOLUME_XML: &str = r#"<volume type='file'>
  <name>disk.qcow2</name>
  <key>/var/lib/libvirt/images/disk.qcow2</key>
  <capacity unit='bytes'>10737418240</capacity>
  <allocation unit='bytes'>6257197056</allocation>
  <target>
    <path>/var/lib/libvirt/images/disk.qcow2</path>
    <format type='qcow2'/>
  </target>
</volume>
"#;

    // Pool parser tests
    #[test]
    fn parses_dir_pool_basics() {
        let p = parse_pool(DIR_POOL_XML).unwrap();
        assert_eq!(p.name, "default");
        assert_eq!(p.uuid, "b92ad468-745b-4806-8e69-b9c7051aad1a");
        assert_eq!(p.pool_type, "dir");
    }

    #[test]
    fn parses_pool_capacity_numbers() {
        let p = parse_pool(DIR_POOL_XML).unwrap();
        assert_eq!(p.capacity, 254288068608);
        assert_eq!(p.allocation, 195908390912);
        assert_eq!(p.available, 58379677696);
    }

    #[test]
    fn parses_pool_target_path() {
        let p = parse_pool(DIR_POOL_XML).unwrap();
        assert_eq!(p.target_path.as_deref(), Some("/var/lib/libvirt/images"));
    }

    #[test]
    fn parses_netfs_pool_source() {
        let p = parse_pool(NETFS_POOL_XML).unwrap();
        assert_eq!(p.pool_type, "netfs");
        assert_eq!(p.source_host.as_deref(), Some("nfs.example"));
        assert_eq!(p.source_dir.as_deref(), Some("/exports/isos"));
    }

    #[test]
    fn parses_logical_pool_vg_name() {
        let xml = r#"<pool type='logical'>
  <name>vg-pool</name>
  <uuid>x</uuid>
  <source>
    <name>my-vg</name>
  </source>
  <target>
    <path>/dev/my-vg</path>
  </target>
</pool>
"#;
        let p = parse_pool(xml).unwrap();
        assert_eq!(p.pool_type, "logical");
        assert_eq!(p.source_name.as_deref(), Some("my-vg"));
    }

    // Volume parser tests
    #[test]
    fn parses_volume_name_and_key() {
        let v = parse_volume(VOLUME_XML).unwrap();
        assert_eq!(v.name, "disk.qcow2");
        assert_eq!(v.key, "/var/lib/libvirt/images/disk.qcow2");
    }

    #[test]
    fn parses_volume_capacity() {
        let v = parse_volume(VOLUME_XML).unwrap();
        assert_eq!(v.capacity, 10737418240);
        assert_eq!(v.allocation, 6257197056);
    }

    #[test]
    fn parses_volume_format_and_path() {
        let v = parse_volume(VOLUME_XML).unwrap();
        assert_eq!(v.format, "qcow2");
        assert_eq!(v.path, "/var/lib/libvirt/images/disk.qcow2");
    }

    // Builder tests
    #[test]
    fn builds_dir_pool_xml() {
        let xml = build_pool_xml(&PoolBuildParams {
            name: "test",
            pool_type: "dir",
            target_path: Some("/srv/libvirt"),
            source_host: None,
            source_dir: None,
            source_name: None,
        });
        assert!(xml.contains("<pool type='dir'>"));
        assert!(xml.contains("<name>test</name>"));
        assert!(xml.contains("<path>/srv/libvirt</path>"));
        assert!(!xml.contains("<source>"), "dir pools don't need <source>");
    }

    #[test]
    fn builds_netfs_pool_xml() {
        let xml = build_pool_xml(&PoolBuildParams {
            name: "nfs-pool",
            pool_type: "netfs",
            target_path: Some("/mnt/nfs"),
            source_host: Some("nas.lan"),
            source_dir: Some("/export/vm"),
            source_name: None,
        });
        assert!(xml.contains("<pool type='netfs'>"));
        assert!(xml.contains("<host name='nas.lan'/>"));
        assert!(xml.contains("<dir path='/export/vm'/>"));
        assert!(xml.contains("<path>/mnt/nfs</path>"));
    }

    #[test]
    fn builds_logical_pool_xml() {
        let xml = build_pool_xml(&PoolBuildParams {
            name: "vg-pool",
            pool_type: "logical",
            target_path: Some("/dev/my-vg"),
            source_host: None,
            source_dir: None,
            source_name: Some("my-vg"),
        });
        assert!(xml.contains("<pool type='logical'>"));
        assert!(xml.contains("<name>my-vg</name>"));
    }

    #[test]
    fn builds_pool_escapes_input() {
        let xml = build_pool_xml(&PoolBuildParams {
            name: "a'><x",
            pool_type: "dir",
            target_path: Some("/<inject>"),
            source_host: None,
            source_dir: None,
            source_name: None,
        });
        assert!(!xml.contains("<x"));
        assert!(!xml.contains("<inject>"));
    }

    #[test]
    fn pool_parse_build_roundtrip() {
        let xml = build_pool_xml(&PoolBuildParams {
            name: "rt",
            pool_type: "dir",
            target_path: Some("/tmp/pool"),
            source_host: None,
            source_dir: None,
            source_name: None,
        });
        let p = parse_pool(&xml).unwrap();
        assert_eq!(p.name, "rt");
        assert_eq!(p.pool_type, "dir");
        assert_eq!(p.target_path.as_deref(), Some("/tmp/pool"));
    }

    #[test]
    fn builds_qcow2_volume_xml() {
        let xml = build_volume_xml(&VolumeBuildParams {
            name: "disk.qcow2",
            capacity_bytes: 10 * 1024 * 1024 * 1024,
            format: "qcow2",
            allocation_bytes: None,
        });
        assert!(xml.contains("<name>disk.qcow2</name>"));
        assert!(xml.contains("<capacity unit='bytes'>10737418240</capacity>"));
        assert!(xml.contains("<format type='qcow2'/>"));
        assert!(!xml.contains("<allocation"), "thin-provisioned by default");
    }

    #[test]
    fn builds_raw_volume_with_allocation() {
        let xml = build_volume_xml(&VolumeBuildParams {
            name: "raw.img",
            capacity_bytes: 1024 * 1024 * 1024,
            format: "raw",
            allocation_bytes: Some(1024 * 1024 * 1024),
        });
        assert!(xml.contains("<format type='raw'/>"));
        assert!(xml.contains("<allocation unit='bytes'>1073741824</allocation>"));
    }

    #[test]
    fn volume_parse_build_roundtrip() {
        let xml = build_volume_xml(&VolumeBuildParams {
            name: "rt.qcow2",
            capacity_bytes: 5_000_000_000,
            format: "qcow2",
            allocation_bytes: None,
        });
        let v = parse_volume(&xml).unwrap();
        assert_eq!(v.name, "rt.qcow2");
        assert_eq!(v.capacity, 5_000_000_000);
        assert_eq!(v.format, "qcow2");
    }

    #[test]
    fn invalid_pool_xml_errors() {
        assert!(parse_pool("<not-xml").is_err());
    }

    #[test]
    fn invalid_volume_xml_errors() {
        assert!(parse_volume("<not-xml").is_err());
    }
}
