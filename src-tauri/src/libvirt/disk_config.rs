//! Disk / CD-ROM device editor.
//!
//! Parses and mutates the `<devices><disk>...</disk></devices>` entries
//! of a domain XML. Mirrors the streaming approach from
//! `boot_config.rs` — we never parse-and-reserialize the whole domain,
//! we splice into the raw XML so unrelated content (seclabel, metadata,
//! iothreads, controllers we have not modeled, etc.) round-trips exactly.

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;
use crate::models::error::VirtManagerError;

/// Where the disk data lives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiskSource {
    /// `<source file='PATH'/>` — a regular file on the host FS.
    File { path: String },
    /// `<source dev='/dev/sdX'/>` — a raw host block device.
    Block { dev: String },
    /// `<source pool='NAME' volume='VOL'/>` — managed through a pool.
    Volume { pool: String, volume: String },
    /// `<source protocol='...' name='...'/>` — network storage.
    Network { protocol: String, name: String },
    /// No backing source (empty CD-ROM).
    None,
}

impl Default for DiskSource {
    fn default() -> Self { DiskSource::None }
}

/// A single `<disk>` device parsed out of a domain XML.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DiskConfig {
    pub device: String,
    pub bus: String,
    pub target: String,
    pub source: DiskSource,
    pub driver_name: Option<String>,
    pub driver_type: Option<String>,
    pub cache: Option<String>,
    pub io: Option<String>,
    pub discard: Option<String>,
    pub detect_zeroes: Option<String>,
    pub serial: Option<String>,
    pub readonly: bool,
    pub shareable: bool,
    pub removable: bool,
    pub rotation_rate: Option<u32>,
    pub iothread: Option<u32>,
    pub boot_order: Option<u32>,
}

// ──────────────────────────── Validation ────────────────────────────

/// Validate bus/target/device combinations.
pub fn validate(d: &DiskConfig) -> Result<(), VirtManagerError> {
    const BUSES: &[&str] = &["virtio", "sata", "scsi", "ide", "usb", "fdc"];
    if !BUSES.contains(&d.bus.as_str()) {
        return Err(VirtManagerError::OperationFailed {
            operation: "validateDisk".into(),
            reason: format!("unknown bus '{}'", d.bus),
        });
    }

    const DEVICES: &[&str] = &["disk", "cdrom", "floppy", "lun"];
    if !DEVICES.contains(&d.device.as_str()) {
        return Err(VirtManagerError::OperationFailed {
            operation: "validateDisk".into(),
            reason: format!("unknown device '{}'", d.device),
        });
    }

    let expected_prefix = match d.bus.as_str() {
        "virtio" => "vd",
        "sata" | "scsi" | "usb" => "sd",
        "ide" => "hd",
        "fdc" => "fd",
        _ => "",
    };
    if !expected_prefix.is_empty() && !d.target.starts_with(expected_prefix) {
        return Err(VirtManagerError::OperationFailed {
            operation: "validateDisk".into(),
            reason: format!(
                "bus='{}' requires target starting with '{}' (got '{}')",
                d.bus, expected_prefix, d.target
            ),
        });
    }

    if d.rotation_rate.is_some() && d.bus != "scsi" {
        return Err(VirtManagerError::OperationFailed {
            operation: "validateDisk".into(),
            reason: format!("rotation_rate only valid on bus=scsi (got {})", d.bus),
        });
    }

    if d.discard.as_deref() == Some("unmap") && d.bus == "ide" {
        return Err(VirtManagerError::OperationFailed {
            operation: "validateDisk".into(),
            reason: "discard=unmap is not supported on the IDE bus".into(),
        });
    }

    if d.device == "floppy" && d.bus != "fdc" {
        return Err(VirtManagerError::OperationFailed {
            operation: "validateDisk".into(),
            reason: format!("device=floppy requires bus=fdc (got {})", d.bus),
        });
    }

    Ok(())
}

// ──────────────────────────── Parse ─────────────────────────────────

/// Extract every `<disk>` under `<devices>` from a domain XML, in order.
pub fn parse_disks(xml: &str) -> Result<Vec<DiskConfig>, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);

    let mut disks: Vec<DiskConfig> = Vec::new();
    let mut cur: Option<DiskConfig> = None;
    let mut in_serial = false;
    let mut buf = Vec::new();

    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => return Err(VirtManagerError::XmlParsingFailed {
                reason: format!("at {}: {}", r.buffer_position(), e),
            }),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = utf8_name(&e);
                let a = attrs(&e);
                if name == "disk" {
                    let mut d = DiskConfig::default();
                    d.device = attr(&a, "device").unwrap_or_else(|| "disk".to_string());
                    cur = Some(d);
                } else if name == "serial" && cur.is_some() {
                    in_serial = true;
                } else if let Some(ref mut d) = cur {
                    if name == "driver" { handle_driver_attrs(d, &a); }
                    else { handle_child(d, &name, &a); }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = utf8_name(&e);
                let a = attrs(&e);
                if name == "disk" {
                    let mut d = DiskConfig::default();
                    d.device = attr(&a, "device").unwrap_or_else(|| "disk".to_string());
                    disks.push(d);
                } else if let Some(ref mut d) = cur {
                    if name == "driver" { handle_driver_attrs(d, &a); }
                    else { handle_child(d, &name, &a); }
                }
            }
            Ok(Event::End(e)) => {
                let name = utf8_name_end(&e);
                if name == "disk" {
                    if let Some(d) = cur.take() { disks.push(d); }
                } else if name == "serial" {
                    in_serial = false;
                }
            }
            Ok(Event::Text(t)) => {
                if in_serial {
                    if let Some(ref mut d) = cur {
                        let s = t.unescape().unwrap_or_default().to_string();
                        d.serial = Some(s);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(disks)
}

fn handle_driver_attrs(d: &mut DiskConfig, a: &[(String, String)]) {
    if let Some(n) = attr(a, "name") { d.driver_name = Some(n); }
    if let Some(t) = attr(a, "type") { d.driver_type = Some(t); }
    if let Some(c) = attr(a, "cache") { d.cache = Some(c); }
    if let Some(io) = attr(a, "io") { d.io = Some(io); }
    if let Some(dc) = attr(a, "discard") { d.discard = Some(dc); }
    if let Some(dz) = attr(a, "detect_zeroes") { d.detect_zeroes = Some(dz); }
    if let Some(it) = attr(a, "iothread") {
        d.iothread = it.parse().ok();
    }
}

fn handle_child(d: &mut DiskConfig, name: &str, a: &[(String, String)]) {
    match name {
        "source" => {
            if let Some(f) = attr(a, "file") {
                d.source = DiskSource::File { path: f };
            } else if let Some(dev) = attr(a, "dev") {
                d.source = DiskSource::Block { dev };
            } else if let Some(pool) = attr(a, "pool") {
                let vol = attr(a, "volume").unwrap_or_default();
                d.source = DiskSource::Volume { pool, volume: vol };
            } else if let Some(proto) = attr(a, "protocol") {
                let n = attr(a, "name").unwrap_or_default();
                d.source = DiskSource::Network { protocol: proto, name: n };
            }
        }
        "target" => {
            if let Some(t) = attr(a, "dev") { d.target = t; }
            if let Some(b) = attr(a, "bus") { d.bus = b; }
            if let Some(rr) = attr(a, "rotation_rate") {
                d.rotation_rate = rr.parse().ok();
            }
        }
        "readonly" => d.readonly = true,
        "shareable" => d.shareable = true,
        "removable" => d.removable = true,
        "boot" => {
            if let Some(o) = attr(a, "order") {
                d.boot_order = o.parse().ok();
            }
        }
        _ => {}
    }
}

/// Public entry point (kept for symmetry with the enrich-serials path).
pub fn parse_disks_full(xml: &str) -> Result<Vec<DiskConfig>, VirtManagerError> {
    parse_disks(xml)
}

// ──────────────────────────── Build ─────────────────────────────────

/// Emit a `<disk>...</disk>` fragment for the given config.
pub fn build_disk_xml(d: &DiskConfig) -> String {
    let mut s = String::new();
    let disk_type = match d.source {
        DiskSource::File { .. } | DiskSource::None => "file",
        DiskSource::Block { .. } => "block",
        DiskSource::Volume { .. } => "volume",
        DiskSource::Network { .. } => "network",
    };
    s.push_str(&format!(
        "<disk type='{}' device='{}'>\n",
        escape_xml(disk_type),
        escape_xml(&d.device),
    ));

    // <driver> attrs.
    let mut driver_attrs: Vec<String> = Vec::new();
    let name = d.driver_name.clone().unwrap_or_else(|| "qemu".to_string());
    driver_attrs.push(format!("name='{}'", escape_xml(&name)));
    if let Some(ref t) = d.driver_type {
        driver_attrs.push(format!("type='{}'", escape_xml(t)));
    }
    if let Some(ref c) = d.cache {
        driver_attrs.push(format!("cache='{}'", escape_xml(c)));
    }
    if let Some(ref io) = d.io {
        driver_attrs.push(format!("io='{}'", escape_xml(io)));
    }
    if let Some(ref dc) = d.discard {
        driver_attrs.push(format!("discard='{}'", escape_xml(dc)));
    }
    if let Some(ref dz) = d.detect_zeroes {
        driver_attrs.push(format!("detect_zeroes='{}'", escape_xml(dz)));
    }
    if let Some(it) = d.iothread {
        driver_attrs.push(format!("iothread='{}'", it));
    }
    s.push_str(&format!("  <driver {}/>\n", driver_attrs.join(" ")));

    match &d.source {
        DiskSource::File { path } => {
            s.push_str(&format!("  <source file='{}'/>\n", escape_xml(path)));
        }
        DiskSource::Block { dev } => {
            s.push_str(&format!("  <source dev='{}'/>\n", escape_xml(dev)));
        }
        DiskSource::Volume { pool, volume } => {
            s.push_str(&format!(
                "  <source pool='{}' volume='{}'/>\n",
                escape_xml(pool),
                escape_xml(volume),
            ));
        }
        DiskSource::Network { protocol, name } => {
            s.push_str(&format!(
                "  <source protocol='{}' name='{}'/>\n",
                escape_xml(protocol),
                escape_xml(name),
            ));
        }
        DiskSource::None => {}
    }

    if let Some(rr) = d.rotation_rate {
        s.push_str(&format!(
            "  <target dev='{}' bus='{}' rotation_rate='{}'/>\n",
            escape_xml(&d.target), escape_xml(&d.bus), rr,
        ));
    } else {
        s.push_str(&format!(
            "  <target dev='{}' bus='{}'/>\n",
            escape_xml(&d.target), escape_xml(&d.bus),
        ));
    }

    if d.readonly { s.push_str("  <readonly/>\n"); }
    if d.shareable { s.push_str("  <shareable/>\n"); }
    if d.removable { s.push_str("  <removable/>\n"); }

    if let Some(ref ser) = d.serial {
        s.push_str(&format!("  <serial>{}</serial>\n", escape_xml(ser)));
    }

    if let Some(order) = d.boot_order {
        s.push_str(&format!("  <boot order='{}'/>\n", order));
    }

    s.push_str("</disk>");
    s
}

// ──────────────────────────── Apply ─────────────────────────────────

/// Add a disk to a domain XML. Splices the built fragment just before
/// `</devices>`.
pub fn apply_disk_add(xml: &str, disk: &DiskConfig) -> Result<String, VirtManagerError> {
    validate(disk)?;

    let existing = parse_disks(xml)?;
    if existing.iter().any(|d| d.target == disk.target) {
        return Err(VirtManagerError::OperationFailed {
            operation: "addDisk".into(),
            reason: format!("disk with target '{}' already exists", disk.target),
        });
    }

    let Some(idx) = xml.rfind("</devices>") else {
        return Err(VirtManagerError::XmlParsingFailed {
            reason: "no </devices> section found".into(),
        });
    };

    let fragment = build_disk_xml(disk);
    let indented = fragment
        .lines()
        .map(|l| format!("    {}", l))
        .collect::<Vec<_>>()
        .join("\n");

    let mut out = String::with_capacity(xml.len() + indented.len() + 1);
    let head = xml[..idx].trim_end_matches(|c: char| c == ' ' || c == '\t');
    let head = head.trim_end_matches('\n');
    out.push_str(head);
    out.push('\n');
    out.push_str(&indented);
    out.push('\n');
    out.push_str("  ");
    out.push_str(&xml[idx..]);
    Ok(out)
}

/// Remove a disk by target dev name.
pub fn apply_disk_remove(xml: &str, target_dev: &str) -> Result<String, VirtManagerError> {
    let (start, end) = find_disk_span(xml, target_dev).ok_or_else(|| {
        VirtManagerError::OperationFailed {
            operation: "removeDisk".into(),
            reason: format!("disk with target '{}' not found", target_dev),
        }
    })?;

    let mut trim_start = start;
    while trim_start > 0 {
        let ch = xml.as_bytes()[trim_start - 1];
        if ch == b' ' || ch == b'\t' { trim_start -= 1; } else { break; }
    }
    let mut trim_end = end;
    if trim_end < xml.len() && xml.as_bytes()[trim_end] == b'\n' {
        trim_end += 1;
    }

    let mut out = String::with_capacity(xml.len());
    out.push_str(&xml[..trim_start]);
    out.push_str(&xml[trim_end..]);
    Ok(out)
}

/// Replace a disk (identified by target dev) with a new config.
pub fn apply_disk_update(xml: &str, disk: &DiskConfig) -> Result<String, VirtManagerError> {
    validate(disk)?;

    let (start, end) = find_disk_span(xml, &disk.target).ok_or_else(|| {
        VirtManagerError::OperationFailed {
            operation: "updateDisk".into(),
            reason: format!("disk with target '{}' not found", disk.target),
        }
    })?;

    let line_start = xml[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent: String = xml[line_start..start].chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    let fragment = build_disk_xml(disk);
    let reindented = fragment
        .lines()
        .enumerate()
        .map(|(i, l)| if i == 0 { l.to_string() } else { format!("{}{}", indent, l) })
        .collect::<Vec<_>>()
        .join("\n");

    let mut out = String::with_capacity(xml.len());
    out.push_str(&xml[..start]);
    out.push_str(&reindented);
    out.push_str(&xml[end..]);
    Ok(out)
}

/// Find `(start, end_exclusive)` of the `<disk>` element whose
/// `<target dev='X'/>` matches `target_dev`. Returns None if absent.
fn find_disk_span(xml: &str, target_dev: &str) -> Option<(usize, usize)> {
    let bytes = xml.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = xml[search_from..].find("<disk") {
        let start = search_from + rel;
        let after = start + "<disk".len();
        let is_tag = after < xml.len() && matches!(
            bytes[after],
            b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/'
        );
        if !is_tag {
            search_from = after;
            continue;
        }

        let gt = match xml[start..].find('>') { Some(g) => start + g, None => return None };
        let self_closing = gt > 0 && bytes[gt - 1] == b'/';
        let block_end = if self_closing {
            gt + 1
        } else {
            match xml[gt..].find("</disk>") {
                Some(off) => gt + off + "</disk>".len(),
                None => return None,
            }
        };

        let block = &xml[start..block_end];
        if disk_has_target(block, target_dev) {
            return Some((start, block_end));
        }
        search_from = block_end;
    }
    None
}

fn disk_has_target(block: &str, target_dev: &str) -> bool {
    use regex::Regex;
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"<target\b[^>]*\bdev=['"]([^'"]+)['"]"#).unwrap()
    });
    RE.captures(block)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str() == target_dev)
        .unwrap_or(false)
}

// ─────────────────────── Low-level helpers ──────────────────────────

fn utf8_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn utf8_name_end(e: &BytesEnd) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .filter_map(|a| a.ok())
        .map(|a| (
            String::from_utf8_lossy(a.key.as_ref()).to_string(),
            a.unescape_value().unwrap_or_default().to_string(),
        ))
        .collect()
}

fn attr(a: &[(String, String)], k: &str) -> Option<String> {
    a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
}

// ──────────────────────────── Tests ─────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<domain type='kvm'>
  <name>test</name>
  <devices>
    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2' cache='none' io='native'/>
      <source file='/var/lib/libvirt/images/root.qcow2'/>
      <target dev='vda' bus='virtio'/>
    </disk>
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <target dev='sda' bus='sata'/>
      <readonly/>
    </disk>
  </devices>
</domain>
"#;

    #[test]
    fn parses_two_disks_preserves_order() {
        let v = parse_disks_full(SAMPLE).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].target, "vda");
        assert_eq!(v[0].bus, "virtio");
        assert_eq!(v[0].device, "disk");
        assert_eq!(v[0].driver_type.as_deref(), Some("qcow2"));
        assert_eq!(v[0].cache.as_deref(), Some("none"));
        assert_eq!(v[0].io.as_deref(), Some("native"));
        if let DiskSource::File { ref path } = v[0].source {
            assert_eq!(path, "/var/lib/libvirt/images/root.qcow2");
        } else { panic!("expected File source"); }
        assert_eq!(v[1].target, "sda");
        assert_eq!(v[1].device, "cdrom");
        assert!(v[1].readonly);
    }

    #[test]
    fn build_round_trips_basic_disk() {
        let d = DiskConfig {
            device: "disk".into(), bus: "virtio".into(), target: "vdb".into(),
            source: DiskSource::File { path: "/tmp/data.qcow2".into() },
            driver_name: Some("qemu".into()), driver_type: Some("qcow2".into()),
            cache: Some("writeback".into()), io: Some("threads".into()),
            discard: Some("unmap".into()), ..Default::default()
        };
        let xml = format!("<domain><devices>{}</devices></domain>", build_disk_xml(&d));
        let parsed = parse_disks(&xml).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], d);
    }

    #[test]
    fn build_with_volume_source() {
        let d = DiskConfig {
            device: "disk".into(), bus: "scsi".into(), target: "sdb".into(),
            source: DiskSource::Volume { pool: "default".into(), volume: "data.qcow2".into() },
            driver_type: Some("qcow2".into()), ..Default::default()
        };
        let frag = build_disk_xml(&d);
        assert!(frag.contains("<source pool='default' volume='data.qcow2'/>"));
        assert!(frag.contains("type='volume'"));
    }

    #[test]
    fn add_then_parse() {
        let d = DiskConfig {
            device: "disk".into(), bus: "virtio".into(), target: "vdc".into(),
            source: DiskSource::File { path: "/var/lib/libvirt/images/extra.qcow2".into() },
            driver_type: Some("qcow2".into()), ..Default::default()
        };
        let new_xml = apply_disk_add(SAMPLE, &d).unwrap();
        let disks = parse_disks_full(&new_xml).unwrap();
        assert_eq!(disks.len(), 3);
        assert!(disks.iter().any(|x| x.target == "vdc"));
    }

    #[test]
    fn remove_by_target() {
        let new_xml = apply_disk_remove(SAMPLE, "sda").unwrap();
        let disks = parse_disks_full(&new_xml).unwrap();
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].target, "vda");
        assert!(!new_xml.contains("device='cdrom'"));
    }

    #[test]
    fn update_by_target_replaces_whole_disk() {
        let mut d = parse_disks_full(SAMPLE).unwrap().into_iter()
            .find(|x| x.target == "vda").unwrap();
        d.cache = Some("writeback".into());
        d.boot_order = Some(1);
        let new_xml = apply_disk_update(SAMPLE, &d).unwrap();
        let back = parse_disks_full(&new_xml).unwrap();
        let vda = back.iter().find(|x| x.target == "vda").unwrap();
        assert_eq!(vda.cache.as_deref(), Some("writeback"));
        assert_eq!(vda.boot_order, Some(1));
    }

    #[test]
    fn cache_io_discard_values_survive() {
        let d = DiskConfig {
            device: "disk".into(), bus: "scsi".into(), target: "sdc".into(),
            source: DiskSource::File { path: "/data.raw".into() },
            driver_type: Some("raw".into()),
            cache: Some("directsync".into()), io: Some("io_uring".into()),
            discard: Some("unmap".into()), detect_zeroes: Some("unmap".into()),
            ..Default::default()
        };
        let xml = format!("<domain><devices>{}</devices></domain>", build_disk_xml(&d));
        let parsed = parse_disks_full(&xml).unwrap();
        assert_eq!(parsed[0].cache.as_deref(), Some("directsync"));
        assert_eq!(parsed[0].io.as_deref(), Some("io_uring"));
        assert_eq!(parsed[0].discard.as_deref(), Some("unmap"));
        assert_eq!(parsed[0].detect_zeroes.as_deref(), Some("unmap"));
    }

    #[test]
    fn serial_round_trips_and_is_escaped() {
        let d = DiskConfig {
            device: "disk".into(), bus: "virtio".into(), target: "vdd".into(),
            source: DiskSource::File { path: "/d.qcow2".into() },
            driver_type: Some("qcow2".into()),
            serial: Some("abc<>&'".into()), ..Default::default()
        };
        let frag = build_disk_xml(&d);
        assert!(frag.contains("<serial>abc&lt;&gt;&amp;&apos;</serial>"));
        let xml = format!("<domain><devices>{}</devices></domain>", frag);
        let parsed = parse_disks_full(&xml).unwrap();
        assert_eq!(parsed[0].serial.as_deref(), Some("abc<>&'"));
    }

    #[test]
    fn rotation_rate_rejected_on_non_scsi() {
        let d = DiskConfig {
            device: "disk".into(), bus: "virtio".into(), target: "vde".into(),
            rotation_rate: Some(1), ..Default::default()
        };
        assert!(validate(&d).is_err());
    }

    #[test]
    fn rotation_rate_accepted_on_scsi() {
        let d = DiskConfig {
            device: "disk".into(), bus: "scsi".into(), target: "sde".into(),
            rotation_rate: Some(1), driver_type: Some("qcow2".into()),
            ..Default::default()
        };
        validate(&d).unwrap();
        let xml = build_disk_xml(&d);
        assert!(xml.contains("rotation_rate='1'"));
        let wrapped = format!("<domain><devices>{}</devices></domain>", xml);
        let parsed = parse_disks_full(&wrapped).unwrap();
        assert_eq!(parsed[0].rotation_rate, Some(1));
    }

    #[test]
    fn discard_unmap_rejected_on_ide() {
        let d = DiskConfig {
            device: "disk".into(), bus: "ide".into(), target: "hda".into(),
            discard: Some("unmap".into()), ..Default::default()
        };
        assert!(validate(&d).is_err());
    }

    #[test]
    fn bus_target_mismatch_rejected() {
        let d = DiskConfig {
            device: "disk".into(), bus: "virtio".into(), target: "sda".into(),
            ..Default::default()
        };
        assert!(validate(&d).is_err());
    }

    #[test]
    fn empty_devices_list() {
        let xml = r#"<domain><devices><emulator>/x</emulator></devices></domain>"#;
        assert!(parse_disks_full(xml).unwrap().is_empty());
    }

    #[test]
    fn multiple_disks_preserve_ordering_across_roundtrip() {
        let disks_in = vec![
            DiskConfig { device: "disk".into(), bus: "virtio".into(), target: "vda".into(),
                source: DiskSource::File { path: "/a".into() }, driver_type: Some("qcow2".into()),
                ..Default::default() },
            DiskConfig { device: "disk".into(), bus: "virtio".into(), target: "vdb".into(),
                source: DiskSource::File { path: "/b".into() }, driver_type: Some("qcow2".into()),
                ..Default::default() },
            DiskConfig { device: "disk".into(), bus: "virtio".into(), target: "vdc".into(),
                source: DiskSource::File { path: "/c".into() }, driver_type: Some("qcow2".into()),
                ..Default::default() },
        ];
        let frags: String = disks_in.iter().map(build_disk_xml).collect::<Vec<_>>().join("\n");
        let xml = format!("<domain><devices>{}</devices></domain>", frags);
        let parsed = parse_disks_full(&xml).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].target, "vda");
        assert_eq!(parsed[1].target, "vdb");
        assert_eq!(parsed[2].target, "vdc");
    }

    #[test]
    fn apply_add_rejects_duplicate_target() {
        let d = DiskConfig {
            device: "disk".into(), bus: "virtio".into(), target: "vda".into(),
            source: DiskSource::File { path: "/x".into() },
            driver_type: Some("qcow2".into()), ..Default::default()
        };
        assert!(apply_disk_add(SAMPLE, &d).is_err());
    }

    #[test]
    fn add_then_remove_is_noop_on_parse() {
        let d = DiskConfig {
            device: "disk".into(), bus: "virtio".into(), target: "vdz".into(),
            source: DiskSource::File { path: "/z.qcow2".into() },
            driver_type: Some("qcow2".into()), ..Default::default()
        };
        let added = apply_disk_add(SAMPLE, &d).unwrap();
        let removed = apply_disk_remove(&added, "vdz").unwrap();
        let a = parse_disks_full(SAMPLE).unwrap();
        let b = parse_disks_full(&removed).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn cdrom_media_swap_via_update() {
        let base = r#"<domain><devices>
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <target dev='sda' bus='sata'/>
      <readonly/>
    </disk>
</devices></domain>"#;
        let updated = DiskConfig {
            device: "cdrom".into(), bus: "sata".into(), target: "sda".into(),
            source: DiskSource::File { path: "/iso/fedora.iso".into() },
            driver_name: Some("qemu".into()), driver_type: Some("raw".into()),
            readonly: true, ..Default::default()
        };
        let new_xml = apply_disk_update(base, &updated).unwrap();
        let disks = parse_disks_full(&new_xml).unwrap();
        assert_eq!(disks.len(), 1);
        if let DiskSource::File { ref path } = disks[0].source {
            assert_eq!(path, "/iso/fedora.iso");
        } else { panic!("expected File source"); }
    }

    #[test]
    fn injection_safe_for_paths() {
        let d = DiskConfig {
            device: "disk".into(), bus: "virtio".into(), target: "vdy".into(),
            source: DiskSource::File { path: "/tmp/x'><inject/>.qcow2".into() },
            driver_type: Some("qcow2".into()), ..Default::default()
        };
        let frag = build_disk_xml(&d);
        assert!(!frag.contains("<inject/>"));
        assert!(frag.contains("&apos;"));
        let wrapped = format!("<domain><devices>{}</devices></domain>", frag);
        let parsed = parse_disks_full(&wrapped).unwrap();
        if let DiskSource::File { ref path } = parsed[0].source {
            assert_eq!(path, "/tmp/x'><inject/>.qcow2");
        } else { panic!("expected File source"); }
    }
}
