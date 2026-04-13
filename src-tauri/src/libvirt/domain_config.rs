//! Parse libvirt domain XML into a structured configuration.
//!
//! Single-pass state machine using quick-xml.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::models::error::VirtManagerError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DomainConfig {
    pub name: String,
    pub uuid: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub memory: Memory,
    pub current_memory: Memory,
    pub vcpus: Vcpus,
    pub cpu: Cpu,
    pub os: OsConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Memory {
    pub kib: u64,
}

impl Memory {
    pub fn mb(&self) -> u64 {
        self.kib / 1024
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Vcpus {
    pub max: u32,
    pub current: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Cpu {
    pub mode: String,
    pub model: Option<String>,
    pub sockets: Option<u32>,
    pub cores: Option<u32>,
    pub threads: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OsConfig {
    pub machine: Option<String>,
    pub arch: Option<String>,
    pub firmware: String,
    pub boot_order: Vec<String>,
}

/// Parse a libvirt domain XML string into a DomainConfig.
pub fn parse(xml: &str) -> Result<DomainConfig, VirtManagerError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut cfg = DomainConfig::default();
    let mut path: Vec<String> = Vec::new();
    // Per-element state for capturing following text
    let mut mem_unit = String::from("KiB");
    let mut cur_mem_unit = String::from("KiB");
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: format!("at pos {}: {}", reader.buffer_position(), e),
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let attrs: Vec<(String, String)> = e
                    .attributes()
                    .filter_map(|a| a.ok())
                    .map(|a| {
                        (
                            String::from_utf8_lossy(a.key.as_ref()).to_string(),
                            a.unescape_value().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();

                let is_empty = matches!(reader.decoder().decode(&[]), Ok(_))
                    && false; // placeholder; we handle via Event::Empty vs Start
                let _ = is_empty;

                // Handle attributes based on current path + this element name
                handle_attrs(&mut cfg, &mut mem_unit, &mut cur_mem_unit, &path, &name, &attrs);

                // Push for Start events; Empty events are self-closing
                // quick-xml gives us distinct Start vs Empty events here
                // We need to only push for Start
                // (We can't easily know which variant matched without re-matching; use a flag.)
            }
            _ => {}
        }
        buf.clear();
    }

    // The above Event::Start|Empty merge loses the distinction.
    // Rewrite with explicit Start vs Empty handling:
    reparse(xml, &mut cfg)?;

    if cfg.os.firmware.is_empty() {
        cfg.os.firmware = "bios".into();
    }
    if cfg.vcpus.current == 0 {
        cfg.vcpus.current = cfg.vcpus.max;
    }

    Ok(cfg)
}

fn reparse(xml: &str, cfg: &mut DomainConfig) -> Result<(), VirtManagerError> {
    // Reset config (everything except what reparse won't overwrite) — safe to reparse from scratch
    *cfg = DomainConfig::default();

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut path: Vec<String> = Vec::new();
    let mut mem_unit = String::from("KiB");
    let mut cur_mem_unit = String::from("KiB");
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
                handle_attrs(cfg, &mut mem_unit, &mut cur_mem_unit, &path, &name, &attrs);
                path.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let attrs = collect_attrs(&e);
                handle_attrs(cfg, &mut mem_unit, &mut cur_mem_unit, &path, &name, &attrs);
                // Self-closing: no push/pop
            }
            Ok(Event::End(_)) => {
                path.pop();
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                handle_text(cfg, &path, &mem_unit, &cur_mem_unit, &text);
            }
            _ => {}
        }
        buf.clear();
    }

    if cfg.os.firmware.is_empty() {
        cfg.os.firmware = "bios".into();
    }
    if cfg.vcpus.current == 0 {
        cfg.vcpus.current = cfg.vcpus.max;
    }

    Ok(())
}

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

fn handle_attrs(
    cfg: &mut DomainConfig,
    mem_unit: &mut String,
    cur_mem_unit: &mut String,
    path: &[String],
    name: &str,
    attrs: &[(String, String)],
) {
    let parent_is = |p: &str| path.last().map(String::as_str) == Some(p);

    match name {
        "memory" if parent_is("domain") => {
            if let Some(u) = get_attr(attrs, "unit") {
                *mem_unit = u.to_string();
            }
        }
        "currentMemory" if parent_is("domain") => {
            if let Some(u) = get_attr(attrs, "unit") {
                *cur_mem_unit = u.to_string();
            }
        }
        "vcpu" if parent_is("domain") => {
            if let Some(v) = get_attr(attrs, "current").and_then(|s| s.parse().ok()) {
                cfg.vcpus.current = v;
            }
        }
        "cpu" if parent_is("domain") => {
            cfg.cpu.mode = get_attr(attrs, "mode").unwrap_or("").to_string();
        }
        "topology" if parent_is("cpu") => {
            cfg.cpu.sockets = get_attr(attrs, "sockets").and_then(|s| s.parse().ok());
            cfg.cpu.cores = get_attr(attrs, "cores").and_then(|s| s.parse().ok());
            cfg.cpu.threads = get_attr(attrs, "threads").and_then(|s| s.parse().ok());
        }
        "type" if parent_is("os") => {
            cfg.os.arch = get_attr(attrs, "arch").map(String::from);
            cfg.os.machine = get_attr(attrs, "machine").map(String::from);
        }
        "boot" if parent_is("os") => {
            if let Some(dev) = get_attr(attrs, "dev") {
                cfg.os.boot_order.push(dev.to_string());
            }
        }
        "loader" if parent_is("os") => {
            cfg.os.firmware = "efi".into();
        }
        _ => {}
    }
}

fn handle_text(
    cfg: &mut DomainConfig,
    path: &[String],
    mem_unit: &str,
    cur_mem_unit: &str,
    text: &str,
) {
    let last = path.last().map(String::as_str);
    let parent = if path.len() >= 2 {
        Some(path[path.len() - 2].as_str())
    } else {
        None
    };

    match (last, parent) {
        (Some("name"), Some("domain")) => cfg.name = text.to_string(),
        (Some("uuid"), Some("domain")) => cfg.uuid = text.to_string(),
        (Some("title"), Some("domain")) => cfg.title = Some(text.to_string()),
        (Some("description"), Some("domain")) => cfg.description = Some(text.to_string()),
        (Some("memory"), Some("domain")) => {
            cfg.memory.kib = parse_memory(text, mem_unit);
        }
        (Some("currentMemory"), Some("domain")) => {
            cfg.current_memory.kib = parse_memory(text, cur_mem_unit);
        }
        (Some("vcpu"), Some("domain")) => {
            cfg.vcpus.max = text.parse().unwrap_or(0);
        }
        (Some("model"), Some("cpu")) => {
            cfg.cpu.model = Some(text.to_string());
        }
        _ => {}
    }
}

fn parse_memory(value: &str, unit: &str) -> u64 {
    let v: u64 = value.trim().parse().unwrap_or(0);
    match unit {
        "b" | "bytes" => v / 1024,
        "KB" => v,
        "KiB" | "K" | "" => v,
        "MB" | "M" | "MiB" => v * 1024,
        "GB" | "G" | "GiB" => v * 1024 * 1024,
        "TB" | "T" | "TiB" => v * 1024 * 1024 * 1024,
        _ => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"<?xml version="1.0"?>
<domain type="kvm">
  <name>test-vm</name>
  <uuid>12345678-1234-1234-1234-123456789abc</uuid>
  <title>Test VM</title>
  <description>A test VM for unit tests.</description>
  <memory unit="KiB">2097152</memory>
  <currentMemory unit="KiB">2097152</currentMemory>
  <vcpu current="2">4</vcpu>
  <os>
    <type arch="x86_64" machine="q35">hvm</type>
    <boot dev="hd"/>
    <boot dev="cdrom"/>
  </os>
  <cpu mode="host-passthrough" check="none"/>
</domain>
"#;

    #[test]
    fn parses_basic_fields() {
        let cfg = parse(SAMPLE_XML).unwrap();
        assert_eq!(cfg.name, "test-vm");
        assert_eq!(cfg.uuid, "12345678-1234-1234-1234-123456789abc");
        assert_eq!(cfg.title, Some("Test VM".into()));
        assert_eq!(cfg.description, Some("A test VM for unit tests.".into()));
    }

    #[test]
    fn parses_memory_as_kib() {
        let cfg = parse(SAMPLE_XML).unwrap();
        assert_eq!(cfg.memory.kib, 2_097_152);
        assert_eq!(cfg.memory.mb(), 2048);
    }

    #[test]
    fn parses_vcpus_max_and_current() {
        let cfg = parse(SAMPLE_XML).unwrap();
        assert_eq!(cfg.vcpus.max, 4);
        assert_eq!(cfg.vcpus.current, 2);
    }

    #[test]
    fn parses_cpu_mode() {
        let cfg = parse(SAMPLE_XML).unwrap();
        assert_eq!(cfg.cpu.mode, "host-passthrough");
    }

    #[test]
    fn parses_os_machine_and_arch() {
        let cfg = parse(SAMPLE_XML).unwrap();
        assert_eq!(cfg.os.machine, Some("q35".into()));
        assert_eq!(cfg.os.arch, Some("x86_64".into()));
    }

    #[test]
    fn parses_boot_order() {
        let cfg = parse(SAMPLE_XML).unwrap();
        assert_eq!(cfg.os.boot_order, vec!["hd", "cdrom"]);
    }

    #[test]
    fn defaults_firmware_to_bios() {
        let cfg = parse(SAMPLE_XML).unwrap();
        assert_eq!(cfg.os.firmware, "bios");
    }

    #[test]
    fn detects_efi_firmware() {
        let xml = r#"<?xml version="1.0"?>
<domain type="kvm">
  <name>efi-vm</name>
  <uuid>aaaa-bbbb</uuid>
  <memory unit="KiB">1048576</memory>
  <vcpu>2</vcpu>
  <os>
    <type arch="x86_64" machine="q35">hvm</type>
    <loader readonly="yes" type="pflash">/usr/share/OVMF/OVMF_CODE.fd</loader>
  </os>
</domain>
"#;
        let cfg = parse(xml).unwrap();
        assert_eq!(cfg.os.firmware, "efi");
    }

    #[test]
    fn memory_unit_conversion_mib() {
        let xml = r#"<?xml version="1.0"?>
<domain type="kvm">
  <name>mib-vm</name>
  <uuid>x</uuid>
  <memory unit="MiB">2048</memory>
  <currentMemory unit="MiB">2048</currentMemory>
  <vcpu>1</vcpu>
  <os><type arch="x86_64">hvm</type></os>
</domain>
"#;
        let cfg = parse(xml).unwrap();
        assert_eq!(cfg.memory.kib, 2048 * 1024);
        assert_eq!(cfg.memory.mb(), 2048);
    }

    #[test]
    fn parses_cpu_topology() {
        let xml = r#"<?xml version="1.0"?>
<domain type="kvm">
  <name>topo-vm</name>
  <uuid>x</uuid>
  <memory unit="KiB">1024</memory>
  <vcpu>8</vcpu>
  <cpu mode="custom">
    <model>Broadwell</model>
    <topology sockets="2" cores="2" threads="2"/>
  </cpu>
  <os><type arch="x86_64">hvm</type></os>
</domain>
"#;
        let cfg = parse(xml).unwrap();
        assert_eq!(cfg.cpu.mode, "custom");
        assert_eq!(cfg.cpu.model, Some("Broadwell".into()));
        assert_eq!(cfg.cpu.sockets, Some(2));
        assert_eq!(cfg.cpu.cores, Some(2));
        assert_eq!(cfg.cpu.threads, Some(2));
    }

    #[test]
    fn invalid_xml_returns_error() {
        let result = parse("not xml at all <unclosed");
        assert!(result.is_err());
    }

    #[test]
    fn empty_vcpu_current_defaults_to_max() {
        let xml = r#"<?xml version="1.0"?>
<domain type="kvm">
  <name>novcur</name>
  <uuid>x</uuid>
  <memory unit="KiB">1024</memory>
  <vcpu>4</vcpu>
  <os><type arch="x86_64">hvm</type></os>
</domain>
"#;
        let cfg = parse(xml).unwrap();
        assert_eq!(cfg.vcpus.max, 4);
        assert_eq!(cfg.vcpus.current, 4);
    }

    #[test]
    fn serializes_to_json() {
        let cfg = parse(SAMPLE_XML).unwrap();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"name\":\"test-vm\""));
        assert!(json.contains("\"machine\":\"q35\""));
    }

    #[test]
    fn memory_mb_helper() {
        let m = Memory { kib: 4096 };
        assert_eq!(m.mb(), 4);
    }

    #[test]
    fn handles_missing_optional_fields() {
        let xml = r#"<?xml version="1.0"?>
<domain type="kvm">
  <name>minimal</name>
  <uuid>x</uuid>
  <memory unit="KiB">1024</memory>
  <vcpu>1</vcpu>
  <os><type arch="x86_64">hvm</type></os>
</domain>
"#;
        let cfg = parse(xml).unwrap();
        assert_eq!(cfg.title, None);
        assert_eq!(cfg.description, None);
        assert_eq!(cfg.cpu.mode, "");
    }
}
